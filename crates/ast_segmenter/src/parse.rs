use std::{convert::TryInto, path::Path};

use eyre::{ensure, WrapErr};
use logger_srcfile::WrapFileLogger;
use oxc_allocator::Allocator;
use oxc_span::{SourceType, Span};
use oxc_utils_parse::ModuleFormat;

use crate::{try_segment_file, Segment};

thread_local! {
    static MODULE_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Source-module parser invocations on this thread, including re-extraction.
pub fn module_parse_count() -> usize {
    MODULE_PARSES.with(std::cell::Cell::get)
}

#[cfg(test)]
#[path = "commonjs_tests.rs"]
mod commonjs_tests;

mod records;
pub use records::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModuleBody {
    Esm,
    /// One indivisible executable segment, excluding the hashbang.
    CommonJs {
        #[serde(with = "crate::owned::span_serde")]
        span: Span,
    },
}

/// Parse and extract all statements, rejecting parser, semantic or extraction errors.
///
/// Unlike the unused-tracker adapter, generated-file marker comments do not skip
/// source. CommonJS internals remain one opaque executable body.
pub fn parse_module(
    logger: &impl logger::Logger,
    source: &str,
    path: &Path,
    package_format: ModuleFormat,
) -> eyre::Result<ParsedModule> {
    parse_module_with_declarations(
        logger,
        source,
        path,
        package_format,
        ecco::DeclarationConfig::default().effective()?,
    )
}

/// Extract with explicit effective contracts; these become part of provenance.
pub fn parse_module_with_declarations(
    logger: &impl logger::Logger,
    source: &str,
    path: &Path,
    package_format: ModuleFormat,
    declarations: ecco::LocalFacts,
) -> eyre::Result<ParsedModule> {
    let declarations = crate::PreparedDeclarations::new(declarations)?;
    parse_module_with_prepared_declarations(logger, source, path, package_format, &declarations)
}

/// Extract using one immutable configuration shared across source/cache operations.
pub fn parse_module_with_prepared_declarations(
    logger: &impl logger::Logger,
    source: &str,
    path: &Path,
    package_format: ModuleFormat,
    declarations: &crate::PreparedDeclarations,
) -> eyre::Result<ParsedModule> {
    let allocator = Allocator::default();
    MODULE_PARSES.with(|count| count.set(count.get() + 1));
    let parsed = oxc_utils_parse::parse_bundler_file(&allocator, source, path, package_format)?;
    let semantic = oxc_semantic::SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    ensure!(
        semantic.errors.is_empty(),
        "{}: semantic diagnostics: {:?}",
        path.display(),
        semantic.errors
    );
    let mut facts = crate::owned::extract(&parsed.program, &semantic.semantic)
        .wrap_err_with(|| format!("{}: occurrence extraction failed", path.display()))?;
    crate::policy::check(&parsed.program, &facts)
        .wrap_err_with(|| format!("{}: unsupported bundler syntax", path.display()))?;
    *facts.ecco_declarations_hash_mut() = declarations.fingerprint();
    let (body, segments) = if parsed.program.source_type.is_commonjs() {
        let span = Span::new(
            parsed.program.hashbang.as_ref().map_or(0, |h| h.span.end),
            source
                .len()
                .try_into()
                .wrap_err("source exceeds span range")?,
        );
        let mut module_deps = crate::raw_module_deps::RawModuleDeps::default();
        for dependency in facts.dependencies() {
            match dependency.kind() {
                crate::owned::DependencyKind::Require => {
                    module_deps
                        .requires
                        .insert(dependency.specifier().to_owned());
                }
                crate::owned::DependencyKind::DynamicImport => {
                    module_deps
                        .dynamic_imports
                        .entry(dependency.specifier().to_owned())
                        .or_default()
                        .insert(crate::raw_module_deps::Symbol::Namespace);
                }
                _ => {}
            }
        }
        (
            ModuleBody::CommonJs { span },
            vec![Segment {
                span,
                module_deps,
                variables: Default::default(),
            }],
        )
    } else {
        let file_logger =
            WrapFileLogger::new(path.display().to_string(), source.to_owned(), logger);
        let segments = try_segment_file(&file_logger, &parsed.program, &semantic.semantic)
            .wrap_err_with(|| format!("{}: segment extraction failed", path.display()))?;
        (ModuleBody::Esm, segments)
    };
    if matches!(body, ModuleBody::Esm) {
        let (mut definitions, symbols) = declarations.facts().clone().into_parts();
        definitions.retain(|definition| match definition.binding() {
            ecco::Binding::Global(path) => facts.references().iter().any(|reference| {
                reference.binding().is_none()
                    && path.first().map(String::as_str) == Some(reference.name())
            }),
            ecco::Binding::Module { module, .. } => facts
                .dependencies()
                .iter()
                .any(|dependency| !dependency.type_only() && *dependency.specifier() == *module),
            ecco::Binding::Local(_) => true,
        });
        *facts.ecco_mut() = Some(ecco::parse_from_ast_with_segments(
            &parsed.program,
            &semantic.semantic,
            ecco::LocalFacts::new(definitions, symbols),
            &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
        )?);
    }
    let provenance = crate::SourceProvenance::extracted(
        source,
        crate::ParserConfig::from_source_type(parsed.program.source_type),
        body,
        &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
        &facts,
    )?;
    Ok(ParsedModule::new(
        parsed.program.source_type,
        body,
        segments,
        facts,
        provenance,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_declarations_share_identity_without_rehashing_or_mutable_pairing() {
        use crate::{fingerprint::declaration_fingerprint_count, PreparedDeclarations};

        let mut facts =
            ecco::parse_declarations("global fn noop() { call_effects = false }").unwrap();
        let before = declaration_fingerprint_count();
        let prepared = PreparedDeclarations::new(facts.clone()).unwrap();
        let shared = prepared.clone();
        facts = ecco::LocalFacts::default();
        assert_eq!(prepared.facts().definitions().len(), 1);
        for (path, source) in [
            ("file.mjs", "noop(); export const value = 1;"),
            ("file.cjs", "noop(); module.exports = 1;"),
        ] {
            let parsed = parse_module_with_prepared_declarations(
                &logger::StdioLogger::new(),
                source,
                Path::new(path),
                ModuleFormat::Esm,
                &shared,
            )
            .unwrap();
            assert_eq!(
                parsed.facts().ecco_declarations_hash(),
                prepared.fingerprint()
            );
            let mut changed = parsed.facts().clone();
            *changed.ecco_declarations_hash_mut() = Default::default();
            assert!(parsed
                .provenance()
                .verify(
                    source,
                    crate::ParserConfig::from_source_type(parsed.source_type()),
                    parsed.body(),
                    &parsed.segments().iter().map(|s| s.span).collect::<Vec<_>>(),
                    &changed,
                )
                .is_err());
        }
        assert_eq!(declaration_fingerprint_count(), before + 1);
        let changed = PreparedDeclarations::new(facts).unwrap();
        assert_ne!(prepared.fingerprint(), changed.fingerprint());
        let (mut definitions, symbols) = prepared.facts().clone().into_parts();
        definitions.push(definitions[0].clone());
        let invalid = ecco::LocalFacts::new(definitions, symbols);
        assert!(PreparedDeclarations::new(invalid).is_err());
        assert_eq!(declaration_fingerprint_count(), before + 2);
    }

    fn parse(source: &str, path: &str) -> eyre::Result<ParsedModule> {
        parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new(path),
            ModuleFormat::Esm,
        )
    }

    #[test]
    fn compound_declarations_have_disjoint_ownership_and_shared_source_prefixes() {
        let source = "export const café = 1, { value: next = café } = {}, last = next;\nlet a, b = last;\nvar c = b, d = c;";
        let parsed = parse(source, "file.mjs").unwrap();
        assert_eq!(parsed.segments().len(), 7);
        let pieces: Vec<_> = parsed
            .facts()
            .emission()
            .segments()
            .iter()
            .map(|recipe| {
                recipe
                    .source()
                    .as_slice()
                    .iter()
                    .map(|span| &source[span.start as usize..span.end as usize])
                    .collect::<String>()
            })
            .collect();
        assert_eq!(
            pieces,
            [
                "export const café = 1",
                "export const { value: next = café } = {}",
                "export const last = next",
                "let a",
                "let b = last",
                "var c = b",
                "var d = c",
            ]
        );
        assert!(parsed
            .segments()
            .windows(2)
            .all(|s| s[0].span.end <= s[1].span.start));
        assert_eq!(
            parsed.segments()[1]
                .variables
                .get_locals()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            ["next"]
        );
        assert_eq!(
            parsed.segments()[1]
                .variables
                .get_escaped_symbols()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            ["café"]
        );
        for (index, export) in parsed.facts().exports().iter().enumerate() {
            assert_eq!(export.span(), parsed.segments()[index].span);
            assert_eq!(parsed.segments()[index].module_deps.exports_locals.len(), 1);
        }
    }

    #[test]
    fn ecco_uses_one_parse_and_owned_split_roots() {
        let source = "export const a = 1, b = a; function lazy() { unknown(); }";
        let before = module_parse_count();
        let parsed = parse(source, "file.mjs").unwrap();
        assert_eq!(module_parse_count(), before + 1);
        let ecco = parsed.facts().ecco().unwrap();
        assert_eq!(ecco.roots().len(), parsed.segments().len());
        assert_eq!(ecco.functions().len(), 1);
        for (root, segment) in ecco.roots().iter().zip(parsed.segments()) {
            assert!(segment.span.start <= root.span().start && root.span().end <= segment.span.end);
        }
        assert!(ecco.symbols().iter().any(|s| s.name() == "lazy"));
        let encoded = serde_cbor::to_vec(&parsed.facts()).unwrap();
        let decoded: crate::owned::ModuleFacts = serde_cbor::from_slice(&encoded).unwrap();
        assert_eq!(&decoded, parsed.facts());
        assert_eq!(serde_cbor::to_vec(&decoded).unwrap(), encoded);
        parsed
            .provenance()
            .verify(
                source,
                crate::ParserConfig::from_source_type(parsed.source_type()),
                parsed.body(),
                &parsed.segments().iter().map(|s| s.span).collect::<Vec<_>>(),
                &decoded,
            )
            .unwrap();
    }

    #[test]
    fn split_declarations_keep_each_dependency_and_redeclaration() {
        use crate::raw_module_deps::Symbol;
        let parsed = parse(
            "var value = import('./first').then(({named}) => named), value = import('./second');",
            "file.mjs",
        )
        .unwrap();
        assert_eq!(parsed.segments().len(), 2);
        assert_eq!(
            parsed
                .facts()
                .bindings()
                .iter()
                .find(|b| b.name() == "value")
                .unwrap()
                .declarations()
                .len(),
            2
        );
        for segment in parsed.segments() {
            assert_eq!(
                segment
                    .variables
                    .get_locals()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
                ["value"]
            );
            assert_eq!(segment.module_deps.dynamic_imports.len(), 1);
        }
        assert!(parsed.segments()[0].module_deps.dynamic_imports["./first"]
            .contains(&Symbol::named("named")));
        assert!(parsed.segments()[1]
            .module_deps
            .dynamic_imports
            .contains_key("./second"));
        let cjs = parse_module(
            &logger::StdioLogger::new(),
            "var a = 1, b = a;",
            Path::new("file.cjs"),
            ModuleFormat::CommonJs,
        )
        .unwrap();
        assert_eq!(cjs.segments().len(), 1);
        assert_eq!(
            cjs.facts().emission().segments()[0].source().as_slice(),
            [cjs.segments()[0].span]
        );
    }

    #[test]
    fn extracts_owned_records_in_each_supported_language() {
        for (path, source) in [
            ("file.js", "export const value = 1;"),
            ("file.ts", "export const value: number = 1;"),
            ("file.jsx", "export const value = 1;"),
            ("file.tsx", "export const value: unknown = 1;"),
        ] {
            let module = parse(source, path).unwrap();
            assert_eq!(module.segments().len(), 1);
            assert_eq!(module.segments()[0].span.end as usize, source.len());
            assert_eq!(module.segments()[0].module_deps.exports_locals.len(), 1);
        }
    }

    #[test]
    fn reports_parser_and_semantic_errors_without_partial_output() {
        let parser_error = parse(
            "export const valid = 1; export const broken = ;",
            "broken.js",
        )
        .unwrap_err();
        assert!(parser_error
            .to_string()
            .contains("broken.js: parser diagnostics"));

        let source = "export const value = 1; const value = 2;";
        let allocator = Allocator::default();
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            source,
            Path::new("duplicate.js"),
            ModuleFormat::Esm,
        )
        .unwrap();
        assert!(parsed.errors.is_empty());
        let semantic_error = parse(source, "duplicate.js").unwrap_err();
        assert!(semantic_error
            .to_string()
            .contains("duplicate.js: semantic diagnostics"));
        assert!(semantic_error.to_string().contains("value"));
    }

    #[test]
    fn generated_markers_do_not_skip_bundler_input() {
        let source = "// This file is auto-generated\nexport const value = 1;";
        assert_eq!(parse(source, "generated.js").unwrap().segments().len(), 1);
        assert!(parse("// This file is auto-generated\nconst = ;", "generated.js").is_err());
    }

    #[test]
    fn unsupported_module_forms_return_diagnostics() {
        for (path, source, diagnostic) in [
            (
                "file.ts",
                "const value = 1; export = value;",
                "TypeScript export assignment",
            ),
            (
                "file.ts",
                "import value = require('./value');",
                "TypeScript import assignment",
            ),
            (
                "file.d.ts",
                "export as namespace Foo;",
                "TypeScript namespace export",
            ),
        ] {
            let error = format!("{:#}", parse(source, path).unwrap_err());
            assert!(error.contains(path), "{}", error);
            assert!(error.contains(diagnostic), "{}", error);
        }
    }

    #[test]
    fn checked_extraction_rejects_statements_tracker_would_skip() {
        let allocator = Allocator::default();
        let source = "const valid = 1; with (object) {}";
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            source,
            Path::new("file.cjs"),
            ModuleFormat::CommonJs,
        )
        .unwrap();
        let semantic = oxc_semantic::SemanticBuilder::new().build(&parsed.program);
        let logger = logger::StdioLogger::new();
        let file_logger = WrapFileLogger::new("file.cjs", source.to_owned(), &logger);
        let error =
            try_segment_file(&file_logger, &parsed.program, &semantic.semantic).unwrap_err();
        assert!(error.to_string().contains(&format!(
            "unsupported with statement at bytes {}..",
            source.find("with").unwrap()
        )));
    }
}
