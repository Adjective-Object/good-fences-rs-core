use std::{convert::TryInto, path::Path};

use eyre::{ensure, WrapErr};
use logger_srcfile::WrapFileLogger;
use oxc_allocator::Allocator;
use oxc_span::{SourceType, Span};
use oxc_utils_parse::ModuleFormat;

use crate::{try_segment_file, Segment};

#[cfg(test)]
#[path = "commonjs_tests.rs"]
mod commonjs_tests;

/// Owned parse facts, not transformed JavaScript or an executable bundle.
#[derive(Debug, Clone)]
pub struct ParsedModule {
    pub source_type: SourceType,
    pub body: ModuleBody,
    pub segments: Vec<Segment>,
    pub facts: crate::owned::ModuleFacts,
    provenance: crate::SourceProvenance,
}

impl ParsedModule {
    pub fn provenance(&self) -> &crate::SourceProvenance {
        &self.provenance
    }

    /// Reconstitute owned extraction records only with matching extraction/cache provenance.
    pub fn from_provenance(
        source: &str,
        provenance: crate::SourceProvenance,
        body: ModuleBody,
        segments: Vec<Segment>,
        facts: crate::owned::ModuleFacts,
    ) -> eyre::Result<Self> {
        provenance.verify(
            source,
            provenance.config(),
            body,
            &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
            &facts,
        )?;
        Ok(Self {
            source_type: provenance.config().source_type(),
            body,
            segments,
            facts,
            provenance,
        })
    }
}

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
    let allocator = Allocator::default();
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
    let facts = crate::owned::extract(&parsed.program, &semantic.semantic)
        .wrap_err_with(|| format!("{}: occurrence extraction failed", path.display()))?;
    crate::policy::check(&parsed.program, &facts)
        .wrap_err_with(|| format!("{}: unsupported bundler syntax", path.display()))?;
    let (body, segments) = if parsed.program.source_type.is_commonjs() {
        let span = Span::new(
            parsed.program.hashbang.as_ref().map_or(0, |h| h.span.end),
            source
                .len()
                .try_into()
                .wrap_err("source exceeds span range")?,
        );
        let mut module_deps = crate::raw_module_deps::RawModuleDeps::default();
        for dependency in &facts.dependencies {
            match dependency.kind {
                crate::owned::DependencyKind::Require => {
                    module_deps.requires.insert(dependency.specifier.clone());
                }
                crate::owned::DependencyKind::DynamicImport => {
                    module_deps
                        .dynamic_imports
                        .entry(dependency.specifier.clone())
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
    let provenance = crate::SourceProvenance::extracted(
        source,
        crate::ParserConfig::from_source_type(parsed.program.source_type),
        body,
        &segments.iter().map(|s| s.span).collect::<Vec<_>>(),
        &facts,
    )?;
    Ok(ParsedModule {
        source_type: parsed.program.source_type,
        body,
        segments,
        facts,
        provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str, path: &str) -> eyre::Result<ParsedModule> {
        parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new(path),
            ModuleFormat::Esm,
        )
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
            assert_eq!(module.segments.len(), 1);
            assert_eq!(module.segments[0].span.end as usize, source.len());
            assert_eq!(module.segments[0].module_deps.exports_locals.len(), 1);
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
        assert_eq!(parse(source, "generated.js").unwrap().segments.len(), 1);
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
