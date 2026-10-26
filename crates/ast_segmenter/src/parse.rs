use std::{collections::HashSet, convert::TryInto, path::Path};

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
    parse_module_with_dependencies(logger, source, path, package_format, declarations, |_| {})
}

/// Observe checked syntax before effects, recipes and Ecco extraction.
///
/// Copied observations are provisional until the returned module succeeds.
/// The AST and semantic arena remain on this thread throughout extraction.
pub fn parse_module_with_dependencies(
    logger: &impl logger::Logger,
    source: &str,
    path: &Path,
    package_format: ModuleFormat,
    declarations: &crate::PreparedDeclarations,
    dependencies: impl FnOnce(&[crate::owned::Dependency]),
) -> eyre::Result<ParsedModule> {
    let allocator = Allocator::default();
    MODULE_PARSES.with(|count| count.set(count.get() + 1));
    let parsed = oxc_utils_parse::parse_bundler_file(&allocator, source, path, package_format)
        .wrap_err_with(|| format!("parsing module for extraction {}", path.display()))?;
    let semantic = oxc_semantic::SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    ensure!(
        semantic.diagnostics.is_empty(),
        "{}: semantic diagnostics: {:?}",
        path.display(),
        semantic.diagnostics
    );
    let mut facts = crate::owned::extract_syntax(&parsed.program, &semantic.semantic)
        .wrap_err_with(|| format!("{}: occurrence extraction failed", path.display()))?;
    crate::policy::check(&parsed.program, &facts)
        .wrap_err_with(|| format!("{}: unsupported bundler syntax", path.display()))?;
    dependencies(facts.dependencies());
    crate::owned::extract_recipes(&parsed.program, &mut facts);
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
        let global_names: HashSet<_> = facts
            .references()
            .iter()
            .filter(|reference| reference.binding().is_none())
            .map(|reference| reference.name())
            .collect();
        let module_specifiers: HashSet<_> = facts
            .dependencies()
            .iter()
            .filter(|dependency| !dependency.type_only())
            .map(|dependency| dependency.specifier())
            .collect();
        let definitions = declarations
            .facts()
            .definitions()
            .iter()
            .filter(|definition| match definition.binding() {
                ecco::Binding::Global(path) => path
                    .first()
                    .is_some_and(|name| global_names.contains(name.as_str())),
                ecco::Binding::Module { module, .. } => module_specifiers.contains(module.as_str()),
                ecco::Binding::Local(_) => true,
            })
            .cloned()
            .collect();
        *facts.ecco_mut() = Some(ecco::parse_from_ast_with_segments(
            &parsed.program,
            &semantic.semantic,
            ecco::LocalFacts::new(definitions, declarations.facts().global_symbols().to_vec()),
            &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
        )?);
    }
    let facts = std::sync::Arc::new(facts);
    let provenance = crate::SourceProvenance::extracted(
        source,
        crate::ParserConfig::for_path(path, package_format)?.resolved(parsed.program.source_type),
        body,
        &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
        facts.clone(),
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
    fn early_dependencies_match_final_occurrences_and_reject_invalid_inputs() {
        let declarations = crate::PreparedDeclarations::new(ecco::LocalFacts::default()).unwrap();
        for (path, source) in [
            (
                "file.mts",
                r#"
                import type { Type } from "types";
                import json from "./data.json" with { type: "json" };
                export { value } from "./reexport.mjs";
                export const load = name => import(`./pages/${name}.mjs`);
                export const broad = request => import(request);
                export const ignored = () => import(/* webpackIgnore: true */ "ignored");
                export const worker = () => new Worker(new URL("./worker.mjs", import.meta.url));
            "#,
            ),
            (
                "file.cjs",
                r#"
                const dep = require("./dep.cjs");
                exports.load = name => require("./pages/" + name + ".cjs");
                exports.async = () => import("./async.mjs");
            "#,
            ),
        ] {
            let mut early = None;
            let before = module_parse_count();
            let parsed = parse_module_with_dependencies(
                &logger::StdioLogger::new(),
                source,
                Path::new(path),
                ModuleFormat::Esm,
                &declarations,
                |dependencies| {
                    assert_eq!(module_parse_count(), before + 1);
                    early = Some(dependencies.to_vec());
                },
            )
            .unwrap();
            assert_eq!(early.unwrap(), parsed.owned_facts().unwrap().dependencies());
            assert_eq!(module_parse_count(), before + 1);
        }
        for source in [
            "import './dep.mjs'; const = ;",
            "import './dep.mjs'; let duplicate; let duplicate;",
            "import './dep.mjs'; eval('unsupported');",
            "import './dep.mjs'; import('options', {});",
        ] {
            let mut observed = false;
            assert!(parse_module_with_dependencies(
                &logger::StdioLogger::new(),
                source,
                Path::new("bad.mjs"),
                ModuleFormat::Esm,
                &declarations,
                |_| observed = true,
            )
            .is_err());
            assert!(!observed, "{}", source);
        }
    }

    #[test]
    fn prepared_declarations_filter_by_source_without_changing_contracts_or_order() {
        let declarations = ecco::parse_declarations(
            r#"
            global fn unused() { call_effects = true }
            module "runtime" { export fn run(value) { returns = value } }
            global fn library.other() { call_effects = true }
            global fn shadowed() { call_effects = false }
            fn localContract(value) { returns = value }
            module "types" { export fn Type() { call_effects = false } }
            global fn library.used() { call_effects = false returns void }
            module "mixed" { export fn run() { call_effects = false } }
            global fn TypeGlobal() { call_effects = false }
            module "side-effect" { export fn run() { call_effects = true } }
            module "dynamic" { export fn run() { call_effects = false } }
            module "absent" { export fn run() { call_effects = true } }
            global symbol unread { read_effects = true }
            global symbol library { read_effects = false }
            "#,
        )
        .unwrap();
        let prepared = crate::PreparedDeclarations::new(declarations.clone()).unwrap();
        for (source, retained) in [
            (
                r#"
                import type { Type } from "types";
                import type { Type as MixedType } from "mixed";
                import { run } from "runtime";
                import { run as mixedRun } from "mixed";
                import "side-effect";
                type OnlyType = TypeGlobal;
                const shadowed = 1;
                shadowed;
                library.used();
                library.used();
                run(mixedRun());
                import("dynamic");
                "#,
                &[1, 2, 4, 6, 7, 8, 9, 10][..],
            ),
            ("export {};", &[4][..]),
        ] {
            let parsed = parse_module_with_prepared_declarations(
                &logger::StdioLogger::new(),
                source,
                Path::new("file.mts"),
                ModuleFormat::Esm,
                &prepared,
            )
            .unwrap();
            let expected = ecco::LocalFacts::new(
                retained
                    .iter()
                    .map(|&index| declarations.definitions()[index].clone())
                    .collect(),
                declarations.global_symbols().to_vec(),
            );
            assert_eq!(
                parsed
                    .owned_facts()
                    .unwrap()
                    .ecco()
                    .as_ref()
                    .unwrap()
                    .declarations(),
                &expected
            );
            assert_eq!(
                parsed.facts().ecco_declarations_hash(),
                prepared.fingerprint()
            );
            assert_ne!(
                parsed.facts().ecco_declarations_hash(),
                crate::PreparedDeclarations::new(expected)
                    .unwrap()
                    .fingerprint()
            );
        }
        assert_eq!(prepared.facts(), &declarations);
    }

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
            let mut changed = parsed.owned_facts().unwrap().clone();
            *changed.ecco_declarations_hash_mut() = Default::default();
            assert!(parsed
                .provenance()
                .verify(
                    source,
                    crate::ParserConfig::from_source_type(parsed.source_type()),
                    parsed.body(),
                    &parsed.segments().iter().map(|s| s.span).collect::<Vec<_>>(),
                    (&changed).into(),
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
    fn sealed_extraction_encodes_once_through_clones_and_cache_framing() {
        use crate::wire_encode::MODULE_ENCODINGS;

        for (path, source) in [
            ("file.mjs", "export const first = 1, second = first;"),
            ("file.cjs", "module.exports = require('./dep');"),
        ] {
            let before = MODULE_ENCODINGS.with(std::cell::Cell::get);
            let parsed = parse(source, path).unwrap();
            let clone = parsed.clone();
            assert!(std::ptr::eq(
                parsed.owned_facts().unwrap(),
                clone.owned_facts().unwrap()
            ));
            let bytes = parsed.to_flatbuffer(source, "test").unwrap();
            assert_eq!(bytes, clone.to_flatbuffer(source, "test").unwrap());
            let (_, body, segments, facts, provenance) = clone.into_parts();
            let restored =
                ParsedModule::from_provenance(source, provenance, body, segments, facts).unwrap();
            assert_eq!(bytes, restored.to_flatbuffer(source, "test").unwrap());
            let wire = cache_flatbuffers::WireRecord::new(
                bytes.clone().into(),
                source,
                restored.provenance().config().wire(),
                "test",
            )
            .unwrap();
            let borrowed = ParsedModule::from_wire(source, wire).unwrap();
            assert_eq!(bytes, borrowed.to_flatbuffer(source, "test").unwrap());
            assert!(borrowed.to_flatbuffer(source, "other parser").is_err());
            assert_eq!(MODULE_ENCODINGS.with(std::cell::Cell::get), before + 1);
        }
    }

    #[test]
    fn sealed_extraction_checks_arbitrary_owned_and_serde_projections() {
        let source = "export const first = 1, second = first;";
        let parsed = parse(source, "file.mjs").unwrap();
        let ranges = parsed.segments().iter().map(|s| s.span).collect::<Vec<_>>();
        let config = parsed.provenance().config();
        let verify = |provenance: &crate::SourceProvenance,
                      source,
                      body,
                      ranges: &[Span],
                      facts: &crate::owned::ModuleFacts| {
            provenance.verify(source, config, body, ranges, facts.into())
        };
        let facts: crate::owned::ModuleFacts =
            serde_cbor::from_slice(&serde_cbor::to_vec(parsed.owned_facts().unwrap()).unwrap())
                .unwrap();
        let provenance: crate::SourceProvenance =
            serde_cbor::from_slice(&serde_cbor::to_vec(parsed.provenance()).unwrap()).unwrap();
        let mut changed_ranges = ranges.clone();
        changed_ranges[0].end -= 1;
        assert!(verify(
            parsed.provenance(),
            source,
            parsed.body(),
            &changed_ranges,
            parsed.owned_facts().unwrap(),
        )
        .is_err());
        assert!(verify(
            parsed.provenance(),
            source,
            ModuleBody::CommonJs { span: ranges[0] },
            &ranges,
            parsed.owned_facts().unwrap(),
        )
        .is_err());
        let (_, body, segments, storage, seal) = parsed.clone().into_parts();
        let crate::views::FactsStorage::Owned(mut shared_facts) = storage else {
            panic!("cold extraction is owned");
        };
        *std::sync::Arc::make_mut(&mut shared_facts).ecco_declarations_hash_mut() =
            Default::default();
        assert!(ParsedModule::from_provenance(
            source,
            seal,
            body,
            segments,
            crate::views::FactsStorage::Owned(shared_facts),
        )
        .is_err());
        verify(
            parsed.provenance(),
            source,
            parsed.body(),
            &ranges,
            parsed.owned_facts().unwrap(),
        )
        .unwrap();
        for provenance in [parsed.provenance(), &provenance] {
            verify(provenance, source, parsed.body(), &ranges, &facts).unwrap();
            assert!(verify(provenance, source, parsed.body(), &changed_ranges, &facts).is_err());
            assert!(verify(
                provenance,
                source,
                ModuleBody::CommonJs { span: ranges[0] },
                &ranges,
                &facts,
            )
            .is_err());
            assert!(verify(
                provenance,
                "export const other = 1;",
                parsed.body(),
                &ranges,
                &facts
            )
            .is_err());
            let mut projection = serde_cbor::value::to_value(&facts).unwrap();
            let serde_cbor::Value::Map(fields) = &mut projection else {
                panic!("facts projection is a map");
            };
            fields.insert(
                serde_cbor::Value::Text("commonjs".into()),
                serde_cbor::Value::Bool(true),
            );
            let changed = serde_cbor::value::from_value(projection).unwrap();
            assert!(verify(provenance, source, parsed.body(), &ranges, &changed).is_err());
            let unrelated = parse("export const other = 1;", "file.mjs").unwrap();
            assert!(verify(
                provenance,
                source,
                parsed.body(),
                &ranges,
                unrelated.owned_facts().unwrap(),
            )
            .is_err());
        }
        let restored = ParsedModule::from_provenance(
            source,
            provenance,
            parsed.body(),
            parsed.owned_segments().unwrap().to_vec(),
            facts,
        )
        .unwrap();
        assert_eq!(
            parsed.to_flatbuffer(source, "test").unwrap(),
            restored.to_flatbuffer(source, "test").unwrap()
        );
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
                    .spans()
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
            .owned_segments()
            .unwrap()
            .windows(2)
            .all(|s| s[0].span.end <= s[1].span.start));
        assert_eq!(
            parsed.owned_segments().unwrap()[1]
                .variables
                .get_locals()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            ["next"]
        );
        assert_eq!(
            parsed.owned_segments().unwrap()[1]
                .variables
                .get_escaped_symbols()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            ["café"]
        );
        for (index, export) in parsed.facts().exports().iter().enumerate() {
            assert_eq!(export.span(), parsed.segments().get(index).unwrap().span);
            assert_eq!(
                parsed.owned_segments().unwrap()[index]
                    .module_deps
                    .exports_locals
                    .len(),
                1
            );
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
        let encoded = serde_cbor::to_vec(parsed.owned_facts().unwrap()).unwrap();
        let decoded: crate::owned::ModuleFacts = serde_cbor::from_slice(&encoded).unwrap();
        assert_eq!(&decoded, parsed.owned_facts().unwrap());
        assert_eq!(serde_cbor::to_vec(&decoded).unwrap(), encoded);
        parsed
            .provenance()
            .verify(
                source,
                crate::ParserConfig::from_source_type(parsed.source_type()),
                parsed.body(),
                &parsed.segments().iter().map(|s| s.span).collect::<Vec<_>>(),
                (&decoded).into(),
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
        for segment in parsed.owned_segments().unwrap() {
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
        assert!(parsed.owned_segments().unwrap()[0]
            .module_deps
            .dynamic_imports["./first"]
            .contains(&Symbol::named("named")));
        assert!(parsed.owned_segments().unwrap()[1]
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
            cjs.facts()
                .emission()
                .segments()
                .get(0)
                .unwrap()
                .source()
                .spans(),
            [cjs.segments().get(0).unwrap().span]
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
            assert_eq!(
                module.segments().get(0).unwrap().span.end as usize,
                source.len()
            );
            assert_eq!(
                module.owned_segments().unwrap()[0]
                    .module_deps
                    .exports_locals
                    .len(),
                1
            );
        }
    }

    #[test]
    fn reports_parser_and_semantic_errors_without_partial_output() {
        let parser_error = parse(
            "export const valid = 1; export const broken = ;",
            "broken.js",
        )
        .unwrap_err();
        assert!(format!("{parser_error:#}").contains("broken.js: parser diagnostics"));

        let source = "export const value = 1; const value = 2;";
        let allocator = Allocator::default();
        let parsed = oxc_utils_parse::parse_bundler_file(
            &allocator,
            source,
            Path::new("duplicate.js"),
            ModuleFormat::Esm,
        )
        .unwrap();
        assert!(parsed.diagnostics.is_empty());
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
        let semantic = oxc_semantic::SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&parsed.program);
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
