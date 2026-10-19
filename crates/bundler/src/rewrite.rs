//! Checked byte edits against original UTF-8 source, followed by JS validation.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ast_segmenter::owned::{BindingId, ExportValue};
use eyre::{ensure, OptionExt};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

thread_local! {
    static OUTPUT_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn output_parse_count() -> usize {
    OUTPUT_PARSES.with(std::cell::Cell::get)
}

pub(crate) fn count_output_parse() {
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

/// Validate JavaScript parser/early errors; explicit `.cjs` uses wrapper grammar.
pub fn validate(path: &Path, source: &str) -> eyre::Result<()> {
    count_output_parse();
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

pub(crate) fn source_hint_edits(source: &str) -> Vec<Edit> {
    count_output_parse();
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::mjs()).parse();
    parsed
        .program
        .comments
        .iter()
        .filter_map(|comment| {
            let text = &source[comment.span.start as usize..comment.span.end as usize];
            let body = text
                .strip_prefix("//")
                .or_else(|| text.strip_prefix("/*").and_then(|t| t.strip_suffix("*/")))?;
            let directive = body
                .strip_prefix('#')
                .or_else(|| body.strip_prefix('@'))?
                .trim_start();
            (directive.starts_with("sourceMappingURL=") || directive.starts_with("sourceURL="))
                .then(|| Edit {
                    span: comment.span,
                    replacement: String::new(),
                })
        })
        .collect()
}

/// Rename private lexical bindings only. Public import/export rewrites belong
/// to the linker; reject capture rather than guessing scope ancestry.
pub fn rename_private(
    path: &Path,
    source: &str,
    names: &[(BindingId, String)],
) -> eyre::Result<String> {
    validate(path, source)?;
    let parsed = ast_segmenter::parse_module(
        &logger::StdioLogger::new(),
        source,
        path,
        ast_segmenter::ModuleFormat::Esm,
    )?;
    let allocator = Allocator::default();
    let program = Parser::new(&allocator, source, SourceType::mjs()).parse();
    let mut shorthand = Shorthand::default();
    shorthand.visit_program(&program.program);
    let mut targets = BTreeSet::new();
    let mut edits = Vec::new();
    for (id, name) in names {
        let binding = parsed
            .facts
            .bindings
            .iter()
            .find(|b| b.id == *id)
            .ok_or_eyre("rename references missing lexical binding")?;
        ensure!(
            !binding.import
                && !parsed
                    .facts
                    .exports
                    .iter()
                    .any(|e| matches!(e.value, ExportValue::Binding(value) if value == *id)),
            "{}: public binding rename needs linker at bytes {}",
            path.display(),
            id.0
        );
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_alphabetic()
                        || b == b'_'
                        || b == b'$'
                        || (i > 0 && b.is_ascii_digit())),
            "invalid generated identifier"
        );
        ensure!(targets.insert(name), "duplicate rename target");
        ensure!(
            !parsed
                .facts
                .bindings
                .iter()
                .any(|b| b.id != *id && b.name == *name)
                && !parsed
                    .facts
                    .references
                    .iter()
                    .any(|r| r.binding != Some(*id) && r.name == *name),
            "rename would capture {}",
            name
        );
        let spans: BTreeSet<_> = binding
            .declarations
            .iter()
            .chain(
                parsed
                    .facts
                    .references
                    .iter()
                    .filter(|r| r.binding == Some(*id))
                    .map(|r| &r.span),
            )
            .map(|s| (s.start, s.end))
            .collect();
        for (start, end) in spans {
            let span = Span::new(start, end);
            let replacement = shorthand.replacement(source, span, name);
            edits.push(Edit { span, replacement });
        }
    }
    let output = apply(source, &edits)?;
    validate(path, &output)?;
    Ok(output)
}

#[derive(Default)]
pub(crate) struct Shorthand(BTreeMap<u32, bool>);

impl Shorthand {
    pub(crate) fn replacement(&self, source: &str, span: Span, name: &str) -> String {
        match self.0.get(&span.start) {
            // Only an object literal's noncomputed __proto__ key is a setter.
            Some(true) => format!("[\"__proto__\"]: {name}"),
            Some(false) => format!(
                "{}: {name}",
                &source[span.start as usize..span.end as usize]
            ),
            None => name.to_owned(),
        }
    }
}

impl<'a> Visit<'a> for Shorthand {
    fn visit_assignment_target_property_identifier(
        &mut self,
        property: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        self.0.insert(property.binding.span.start, false);
        walk::walk_assignment_target_property_identifier(self, property);
    }

    fn visit_object_property(&mut self, property: &ObjectProperty<'a>) {
        if property.shorthand {
            self.0.insert(
                property.value.span().start,
                matches!(&property.key, PropertyKey::StaticIdentifier(key) if key.name == "__proto__"),
            );
        }
        walk::walk_object_property(self, property);
    }

    fn visit_binding_property(&mut self, property: &BindingProperty<'a>) {
        if property.shorthand {
            self.0.insert(property.value.span().start, false);
        }
        walk::walk_binding_property(self, property);
    }
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
        .facts;
        let id = facts
            .bindings
            .iter()
            .find(|b| b.name == "x" && b.top_level)
            .unwrap()
            .id;
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
        let source = "const keep = '//# sourceMappingURL=inside'; // ordinary sourceMappingURL=keep\n//# sourceMappingURL=old.map\n/*@ sourceURL=old.js */\n";
        let output = apply(source, &source_hint_edits(source)).unwrap();
        assert_eq!(
            output,
            "const keep = '//# sourceMappingURL=inside'; // ordinary sourceMappingURL=keep\n\n\n"
        );
        validate(Path::new("output.mjs"), &output).unwrap();
    }
}
