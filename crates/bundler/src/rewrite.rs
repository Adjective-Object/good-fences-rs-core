//! Checked byte edits against original UTF-8 source, followed by JS validation.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ast_segmenter::owned::{BindingId, ExportValue};
use eyre::{ensure, OptionExt};
#[cfg(debug_assertions)]
use oxc_allocator::Allocator;
#[cfg(debug_assertions)]
use oxc_parser::Parser;
#[cfg(debug_assertions)]
use oxc_span::SourceType;
use oxc_span::Span;

thread_local! {
    static OUTPUT_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FINAL_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static WORKER_SOURCE_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static WORKER_DECLARATION_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Clone, Copy)]
pub(crate) struct ParseCounts {
    pub(crate) source: usize,
    pub(crate) output: usize,
    pub(crate) validation: usize,
    pub(crate) declarations: usize,
}

impl ParseCounts {
    pub(crate) fn difference(self, before: Self) -> Self {
        Self {
            source: self.source - before.source,
            output: self.output - before.output,
            validation: self.validation - before.validation,
            declarations: self.declarations - before.declarations,
        }
    }
}

pub(crate) fn source_parse_count() -> usize {
    ast_segmenter::module_parse_count() + WORKER_SOURCE_PARSES.with(std::cell::Cell::get)
}

pub(crate) fn parse_counts() -> ParseCounts {
    ParseCounts {
        source: source_parse_count(),
        output: output_parse_count(),
        validation: final_validation_count(),
        declarations: ast_segmenter::fingerprint::declaration_fingerprint_count()
            + WORKER_DECLARATION_HASHES.with(std::cell::Cell::get),
    }
}

pub(crate) fn record_worker_parses(counts: ParseCounts) {
    WORKER_SOURCE_PARSES.with(|count| count.set(count.get() + counts.source));
    OUTPUT_PARSES.with(|count| count.set(count.get() + counts.output));
    FINAL_VALIDATIONS.with(|count| count.set(count.get() + counts.validation));
    WORKER_DECLARATION_HASHES.with(|count| count.set(count.get() + counts.declarations));
}

pub(crate) fn output_parse_count() -> usize {
    OUTPUT_PARSES.with(std::cell::Cell::get)
}

pub(crate) fn final_validation_count() -> usize {
    FINAL_VALIDATIONS.with(std::cell::Cell::get)
}

#[cfg(debug_assertions)]
fn count_output_parse() {
    OUTPUT_PARSES.with(|count| count.set(count.get() + 1));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub span: Span,
    pub replacement: String,
}

/// Edits refer to the original source, never offsets in an intermediate result.
/// Adjacent edits are legal; overlapping or duplicate insertions are rejected.
pub fn apply(source: &str, edits: &[Edit]) -> eyre::Result<String> {
    let mut edits: Vec<_> = edits.iter().collect();
    edits.sort_by_key(|e| (e.span.start, e.span.end));
    let mut end = 0;
    let mut previous = None;
    let mut result = String::new();
    for edit in edits {
        let start = edit.span.start as usize;
        let next = edit.span.end as usize;
        ensure!(
            start <= next
                && next <= source.len()
                && source.is_char_boundary(start)
                && source.is_char_boundary(next),
            "invalid UTF-8 edit range {}..{}",
            start,
            next
        );
        ensure!(
            start >= end && previous != Some(edit.span.start),
            "overlapping edits at bytes {}..{}",
            start,
            next
        );
        result.push_str(&source[end..start]);
        result.push_str(&edit.replacement);
        end = next;
        previous = Some(edit.span.start);
    }

    result.push_str(&source[end..]);
    Ok(result)
}

/// Debug-only final JavaScript validation; `.cjs` uses wrapper grammar.
#[cfg(debug_assertions)]
pub fn validate(path: &Path, source: &str) -> eyre::Result<()> {
    let _span = tracing::info_span!("build.validate_output", bytes = source.len()).entered();
    count_output_parse();
    FINAL_VALIDATIONS.with(|count| count.set(count.get() + 1));
    let allocator = Allocator::default();
    let source_type = if path.extension().is_some_and(|e| e == "cjs") {
        SourceType::mjs().with_commonjs(true)
    } else {
        SourceType::mjs()
    };
    let parsed = Parser::new(&allocator, source, source_type).parse();
    ensure!(
        !parsed.panicked && parsed.errors.is_empty(),
        "{}: emitted JavaScript parser diagnostics: {:?}",
        path.display(),
        parsed.errors
    );
    let semantic = oxc_semantic::SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&parsed.program);
    ensure!(
        semantic.errors.is_empty(),
        "{}: emitted JavaScript semantic diagnostics: {:?}",
        path.display(),
        semantic.errors
    );
    Ok(())
}

/// Release output relies on checked recipes, identifiers, targets and edits.
#[cfg(not(debug_assertions))]
pub fn validate(_path: &Path, _source: &str) -> eyre::Result<()> {
    Ok(())
}

/// Rename private lexical bindings only. Public import/export rewrites belong
/// to the linker; reject capture rather than guessing scope ancestry.
pub fn rename_private(
    path: &Path,
    source: &str,
    names: &[(BindingId, String)],
) -> eyre::Result<String> {
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        path,
        ast_segmenter::ModuleFormat::Esm,
    )?;
    let mut targets = BTreeSet::new();
    let mut renamed = BTreeMap::new();
    for (id, name) in names {
        let binding = parsed
            .facts()
            .bindings()
            .iter()
            .find(|b| b.id() == *id)
            .ok_or_eyre("rename references missing lexical binding")?;
        ensure!(
            !binding.import()
                && !parsed
                    .facts()
                    .exports()
                    .iter()
                    .any(|e| matches!(e.value(), ExportValue::Binding(value) if *value == *id)),
            "{}: public binding rename needs linker at bytes {}",
            path.display(),
            id.0
        );
        ensure!(
            crate::symbols::valid_rename_target(name),
            "invalid generated identifier"
        );
        ensure!(targets.insert(name), "duplicate rename target");
        ensure!(
            !parsed
                .facts()
                .bindings()
                .iter()
                .any(|b| b.id() != *id && b.name() == *name)
                && !parsed
                    .facts()
                    .references()
                    .iter()
                    .any(|r| r.binding() != Some(*id) && r.name() == *name),
            "rename would capture {}",
            name
        );
        renamed.insert(id.0, name);
    }
    let mut edits = BTreeMap::new();
    for recipe in parsed.facts().emission().segments() {
        for operation in recipe.operations() {
            use ast_segmenter::emission::OperationKind;
            let (binding, shorthand) = match operation.kind() {
                OperationKind::Binding {
                    binding, shorthand, ..
                } => (Some(*binding), *shorthand),
                OperationKind::Reference {
                    reference,
                    shorthand,
                } => (
                    parsed.facts().references()[*reference as usize].binding(),
                    *shorthand,
                ),
                _ => continue,
            };
            let Some(name) = binding.and_then(|id| renamed.get(&id.0)) else {
                continue;
            };
            let span = operation.kind().span(&parsed.facts(), recipe);
            edits.insert(
                (span.start, span.end),
                Edit {
                    span,
                    replacement: shorthand_replacement(source, span, name, shorthand)?,
                },
            );
        }
    }
    let edits: Vec<_> = edits.into_values().collect();
    let output = apply(source, &edits)?;
    validate(path, &output)?;
    Ok(output)
}

pub(crate) fn shorthand_replacement(
    source: &str,
    span: Span,
    name: &str,
    shorthand: ast_segmenter::emission::Shorthand,
) -> eyre::Result<String> {
    Ok(match shorthand {
        // Only an object literal's noncomputed __proto__ key is a setter.
        ast_segmenter::emission::Shorthand::Proto => format!("[\"__proto__\"]: {name}"),
        ast_segmenter::emission::Shorthand::Property => format!(
            "{}: {name}",
            source
                .get(span.start as usize..span.end as usize)
                .ok_or_eyre("invalid UTF-8 shorthand span")?
        ),
        ast_segmenter::emission::Shorthand::None => name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_boundaries_utf8_and_overlap() {
        let edit = |a, b, text: &str| Edit {
            span: Span::new(a, b),
            replacement: text.into(),
        };
        assert_eq!(
            apply("é12", &[edit(3, 4, "b"), edit(2, 3, "a")]).unwrap(),
            "éab"
        );
        for edits in [
            vec![edit(1, 2, "")],
            vec![edit(0, 9, "")],
            vec![edit(3, 2, "")],
            vec![edit(0, 3, ""), edit(2, 4, "")],
            vec![edit(0, 0, "a"), edit(0, 0, "b")],
        ] {
            assert!(apply("é12", &edits).is_err());
        }
    }

    #[test]
    fn lexical_rename_preserves_keys_shadowing_and_preamble() {
        let source = "#!/usr/bin/env node\n// legal é\n'use strict';\nconst {x = 1} = {}; const obj = {x}; function f(x) { return {x}; } console.log(obj, x);";
        let path = Path::new("rename.mjs");
        let facts = ast_segmenter::parse_module(
            &logger::StdioLogger::new(),
            source,
            path,
            ast_segmenter::ModuleFormat::Esm,
        )
        .unwrap()
        .into_facts();
        let id = facts
            .bindings()
            .iter()
            .find(|b| b.name() == "x" && b.top_level())
            .unwrap()
            .id();
        let output = rename_private(path, source, &[(id, "renamed".into())]).unwrap();
        insta::assert_snapshot!(output, @r###"
        #!/usr/bin/env node
        // legal é
        'use strict';
        const {x: renamed = 1} = {}; const obj = {x: renamed}; function f(x) { return {x}; } console.log(obj, renamed);
        "###);
        assert!(rename_private(path, source, &[(id, "obj".into())]).is_err());
        assert!(rename_private(path, source, &[(id, "for".into())]).is_err());
    }

    #[test]
    fn assignment_shorthand_preserves_original_key() {
        let source = "let x; ({x = 1} = {}); globalThis.result = x;";
        let output = rename_private(
            Path::new("input.mjs"),
            source,
            &[(BindingId(4), "renamed".into())],
        )
        .unwrap();
        assert_eq!(
            output,
            "let renamed; ({x: renamed = 1} = {}); globalThis.result = renamed;"
        );
    }

    #[test]
    fn proto_shorthand_uses_computed_keys_only_in_object_literals() {
        let source = "let {__proto__ = null} = {}; ({__proto__ = 4} = {}); console.log({__proto__, nested: {__proto__}, __proto__: __proto__});";
        let output = rename_private(
            Path::new("input.mjs"),
            source,
            &[(BindingId(5), "renamed".into())],
        )
        .unwrap();
        insta::assert_snapshot!(output, @r#"let {__proto__: renamed = null} = {}; ({__proto__: renamed = 4} = {}); console.log({["__proto__"]: renamed, nested: {["__proto__"]: renamed}, __proto__: renamed});"#);
    }

    #[test]
    fn source_hints_are_comments_not_matching_strings_or_ordinary_prose() {
        let source = "const keep = (/*@ sourceURL=old.js */'//# sourceMappingURL=inside' // ordinary sourceMappingURL=keep\n//# sourceMappingURL=old.map\n);";
        let parsed = ast_segmenter::parse_module(
            &logger::StdioLogger::new(),
            source,
            Path::new("input.mjs"),
            ast_segmenter::ModuleFormat::Esm,
        )
        .unwrap();
        let output = crate::emit::CachedRenderer::new(
            source,
            &parsed.facts(),
            &parsed.facts().emission().segments()[0],
            &[],
        )
        .render(parsed.segments()[0].span)
        .unwrap()
        .render(
            Path::new("output.mjs"),
            Path::new("output.mjs"),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            output,
            "const keep = ('//# sourceMappingURL=inside' // ordinary sourceMappingURL=keep\n\n);"
        );
        validate(Path::new("output.mjs"), &output).unwrap();
    }

    #[test]
    fn rename_legality_remains_checked_without_output_parsing() {
        for target in [
            "",
            "1bad",
            "x-y",
            "for",
            "await",
            "yield",
            "eval",
            "arguments",
            "implements",
        ] {
            assert!(
                rename_private(
                    Path::new("input.mjs"),
                    "let x;",
                    &[(BindingId(4), target.into())]
                )
                .is_err(),
                "{target}"
            );
        }
        assert!(rename_private(Path::new("input.mjs"), "const =;", &[]).is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn final_validation_reports_semantic_diagnostics() {
        let error = validate(Path::new("output.mjs"), "let x; let x;").unwrap_err();
        assert!(
            error.to_string().contains("semantic diagnostics"),
            "{error}"
        );
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn final_validation_does_not_parse_release_output() {
        let before = output_parse_count();
        validate(Path::new("output.mjs"), "not valid JavaScript !!").unwrap();
        assert_eq!(output_parse_count(), before);
    }
}
