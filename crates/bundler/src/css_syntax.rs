//! Owned module syntax only; no allocator-backed nodes cross this boundary.
use std::{ops::Range, path::Path};

use eyre::{bail, ensure, WrapErr};
use oxc_css_parser::{ast::*, Allocator, Parser, Span, Syntax};

pub(crate) const PARSER_REVISION: &str = "6e1604ddd0ad0d08279aac4a2910ff8ca585f7c6";

fn own_decoded(value: &str) -> String {
    // The pinned Oxc decoder preserves zero escapes as NUL, unlike CSS Syntax.
    value.replace('\0', "\u{fffd}")
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Name {
    pub(crate) decoded: String,
    pub(crate) span: Range<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SelectorKind {
    Class,
    Id,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SelectorName {
    pub(crate) kind: SelectorKind,
    pub(crate) name: Name,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Local,
    Global,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScopeMarker {
    pub(crate) kind: ScopeKind,
    pub(crate) span: Range<usize>,
    pub(crate) argument: Option<Range<usize>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CompositionSource {
    Local,
    Global,
    File(Name),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Composition {
    pub(crate) span: Range<usize>,
    pub(crate) selector: Option<Range<usize>>,
    pub(crate) names: Vec<Name>,
    pub(crate) source: CompositionSource,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ModuleSyntax {
    pub(crate) names: Vec<SelectorName>,
    pub(crate) scopes: Vec<ScopeMarker>,
    pub(crate) compositions: Vec<Composition>,
}

pub(crate) fn parse(path: &Path, source: &str) -> eyre::Result<ModuleSyntax> {
    parse_inner(source).wrap_err_with(|| format!("{}: CSS Modules syntax", path.display()))
}

fn parse_inner(source: &str) -> eyre::Result<ModuleSyntax> {
    // This is the CSS parser's allocator, not the separately pinned JS allocator.
    let allocator = Allocator::default();
    let mut parser = Parser::new(&allocator, source, Syntax::Css);
    let stylesheet = parser.parse::<Stylesheet>().map_err(|error| {
        eyre::eyre!(
            "Oxc CSS at {}..{}: {:?}",
            error.span.start,
            error.span.end,
            error.kind
        )
    })?;
    if let Some(error) = parser.recoverable_errors().first() {
        bail!(
            "Oxc CSS recovery at {}..{}: {:?}; cannot safely transform recovered CSS",
            error.span.start,
            error.span.end,
            error.kind
        );
    }
    let mut collector = Collector {
        source,
        syntax: ModuleSyntax::default(),
    };
    collector.statements(&stylesheet.statements, None, 0)?;
    Ok(collector.syntax)
}

struct Collector<'a> {
    source: &'a str,
    syntax: ModuleSyntax,
}

impl Collector<'_> {
    fn span(&self, span: Span) -> eyre::Result<Range<usize>> {
        checked_span(self.source, span.start..span.end)
    }

    fn name(&self, name: &InterpolableIdent<'_>) -> eyre::Result<Name> {
        let InterpolableIdent::Literal(name) = name else {
            bail!("interpolated CSS Modules names are unsupported");
        };
        Ok(Name {
            decoded: own_decoded(name.name),
            span: self.span(name.span)?,
        })
    }

    fn statements(
        &mut self,
        statements: &[Statement<'_>],
        selector: Option<Range<usize>>,
        depth: usize,
    ) -> eyre::Result<()> {
        ensure!(depth < 128, "CSS Modules nesting limit exceeded");
        for statement in statements {
            match statement {
                Statement::QualifiedRule(rule) => {
                    self.selectors(&rule.selector, depth + 1)?;
                    self.statements(
                        &rule.block.statements,
                        Some(self.span(rule.selector.span)?),
                        depth + 1,
                    )?;
                }
                Statement::Declaration(declaration) => {
                    if self
                        .name(&declaration.name)?
                        .decoded
                        .eq_ignore_ascii_case("composes")
                    {
                        self.composition(declaration, selector.clone())?;
                    }
                }
                Statement::AtRule(rule) => {
                    match &rule.prelude {
                        Some(AtRulePrelude::Nest(selectors)) => self.selectors(selectors, depth + 1)?,
                        Some(AtRulePrelude::Scope(scope)) => match scope.as_ref() {
                            ScopePrelude::StartOnly(start) => {
                                if let Some(selectors) = &start.selector {
                                    self.selectors(selectors, depth + 1)?;
                                }
                            }
                            ScopePrelude::EndOnly(end) => {
                                if let Some(selectors) = &end.selector {
                                    self.selectors(selectors, depth + 1)?;
                                }
                            }
                            ScopePrelude::Both(scope) => {
                                if let Some(selectors) = &scope.start.selector {
                                    self.selectors(selectors, depth + 1)?;
                                }
                                if let Some(selectors) = &scope.end.selector {
                                    self.selectors(selectors, depth + 1)?;
                                }
                            }
                        },
                        None
                        | Some(AtRulePrelude::Charset(_)
                            | AtRulePrelude::Import(_)
                            | AtRulePrelude::Namespace(_)
                            | AtRulePrelude::Media(_)
                            | AtRulePrelude::Layer(_)
                            | AtRulePrelude::Container(_)
                            | AtRulePrelude::Keyframes(_)) => {}
                        _ => bail!(
                            "unsupported CSS Modules at-rule prelude at {:?}; selector-bearing syntax must be handled explicitly",
                            self.span(rule.span)?
                        ),
                    }
                    if let Some(block) = &rule.block {
                        self.statements(&block.statements, selector.clone(), depth + 1)?;
                    }
                }
                Statement::KeyframeBlock(block) => {
                    self.statements(&block.block.statements, None, depth + 1)?;
                }
                Statement::UnknownQualifiedRule(rule) => bail!(
                    "raw CSS selector at {:?}; cannot safely transform CSS Modules",
                    self.span(rule.prelude.span)?
                ),
                Statement::UnknownSassAtRule(rule) => bail!(
                    "raw CSS at-rule at {:?}; cannot safely transform CSS Modules",
                    self.span(rule.span)?
                ),
                _ => bail!("unsupported CSS Modules statement; cannot safely transform raw syntax"),
            }
        }
        Ok(())
    }

    fn selectors(&mut self, selectors: &SelectorList<'_>, depth: usize) -> eyre::Result<()> {
        for selector in &selectors.selectors {
            self.complex(selector, depth + 1)?;
        }
        Ok(())
    }

    fn complex(&mut self, selector: &ComplexSelector<'_>, depth: usize) -> eyre::Result<()> {
        ensure!(depth < 128, "CSS Modules selector nesting limit exceeded");
        for child in &selector.children {
            if let ComplexSelectorChild::CompoundSelector(selector) = child {
                self.compound(selector, depth + 1)?;
            }
        }
        Ok(())
    }

    fn compound(&mut self, selector: &CompoundSelector<'_>, depth: usize) -> eyre::Result<()> {
        ensure!(depth < 128, "CSS Modules selector nesting limit exceeded");
        for child in &selector.children {
            match child {
                SimpleSelector::Class(class) => self.syntax.names.push(SelectorName {
                    kind: SelectorKind::Class,
                    name: self.name(&class.name)?,
                }),
                SimpleSelector::Id(id) => self.syntax.names.push(SelectorName {
                    kind: SelectorKind::Id,
                    name: self.name(&id.name)?,
                }),
                SimpleSelector::PseudoClass(pseudo) => {
                    let name = self.name(&pseudo.name)?;
                    let scope = if name.decoded.eq_ignore_ascii_case("local") {
                        Some(ScopeKind::Local)
                    } else if name.decoded.eq_ignore_ascii_case("global") {
                        Some(ScopeKind::Global)
                    } else {
                        None
                    };
                    if let Some(kind) = scope {
                        self.syntax.scopes.push(ScopeMarker {
                            kind,
                            span: self.span(pseudo.span)?,
                            argument: pseudo
                                .arg
                                .as_ref()
                                .map(|arg| self.span(arg.span))
                                .transpose()?,
                        });
                    }
                    if let Some(arg) = &pseudo.arg {
                        match &arg.kind {
                            PseudoClassSelectorArgKind::SelectorList(selectors) => self.selectors(selectors, depth + 1)?,
                            PseudoClassSelectorArgKind::CompoundSelectorList(selectors) => {
                                for selector in &selectors.selectors {
                                    self.compound(selector, depth + 1)?;
                                }
                            }
                            PseudoClassSelectorArgKind::RelativeSelectorList(selectors) => {
                                for selector in &selectors.selectors {
                                    self.complex(&selector.complex_selector, depth + 1)?;
                                }
                            }
                            PseudoClassSelectorArgKind::Nth(nth) => {
                                if let Some(selectors) = nth.matcher.as_ref().and_then(|matcher| matcher.selector.as_ref()) {
                                    self.selectors(selectors, depth + 1)?;
                                }
                            }
                            PseudoClassSelectorArgKind::Ident(_)
                            | PseudoClassSelectorArgKind::LanguageRangeList(_)
                            | PseudoClassSelectorArgKind::Number(_) => {}
                            _ => bail!("raw CSS pseudo-class argument at {:?}; cannot safely transform CSS Modules", self.span(arg.span)?),
                        }
                    }
                }
                SimpleSelector::PseudoElement(pseudo) => {
                    if let Some(arg) = &pseudo.arg {
                        match &arg.kind {
                            PseudoElementSelectorArgKind::CompoundSelector(selector) => self.compound(selector, depth + 1)?,
                            PseudoElementSelectorArgKind::CompoundSelectorList(selectors) => {
                                for selector in &selectors.selectors {
                                    self.compound(selector, depth + 1)?;
                                }
                            }
                            PseudoElementSelectorArgKind::Ident(_)
                            | PseudoElementSelectorArgKind::IdentList(_) => {}
                            PseudoElementSelectorArgKind::TokenSeq(_) => bail!("raw CSS pseudo-element argument at {:?}; cannot safely transform CSS Modules", self.span(arg.span)?),
                        }
                    }
                }
                SimpleSelector::Type(_)
                | SimpleSelector::Attribute(_)
                | SimpleSelector::Nesting(_) => {}
                SimpleSelector::SassPlaceholder(_) => {
                    bail!("Sass selectors are unsupported in CSS Modules")
                }
            }
        }
        Ok(())
    }

    fn composition(
        &mut self,
        declaration: &Declaration<'_>,
        selector: Option<Range<usize>>,
    ) -> eyre::Result<()> {
        let span = self.span(declaration.span)?;
        ensure!(
            !declaration.value_is_raw && declaration.important.is_none(),
            "raw/important composes declaration at {span:?}"
        );
        let mut names = Vec::new();
        let mut source = CompositionSource::Local;
        let mut values = declaration.value.iter();
        while let Some(value) = values.next() {
            let ComponentValue::InterpolableIdent(ident) = value else {
                bail!("unsupported composes name at {span:?}");
            };
            let name = self.name(ident)?;
            if name.decoded == "from" {
                source = match values.next() {
                    Some(ComponentValue::InterpolableIdent(ident))
                        if self.name(ident)?.decoded == "global" =>
                    {
                        CompositionSource::Global
                    }
                    Some(ComponentValue::InterpolableStr(InterpolableStr::Literal(value))) => {
                        CompositionSource::File(Name {
                            decoded: own_decoded(value.value),
                            span: self.span(value.span)?,
                        })
                    }
                    _ => bail!("unsupported composes source at {span:?}"),
                };
                ensure!(
                    values.next().is_none(),
                    "trailing composes tokens at {span:?}"
                );
                break;
            }
            names.push(name);
        }
        ensure!(!names.is_empty(), "empty composes declaration at {span:?}");
        self.syntax.compositions.push(Composition {
            span,
            selector,
            names,
            source,
        });
        Ok(())
    }
}

pub(crate) fn reject_unfinished(path: &Path, source: &str) -> eyre::Result<()> {
    let syntax = parse(path, source)?;
    bail!(
        "{}: CSS Modules transformation is not implemented (Oxc CSS {PARSER_REVISION}; {} selector names, {} scope markers, {} composition requests); refusing unscoped output",
        path.display(), syntax.names.len(), syntax.scopes.len(), syntax.compositions.len()
    )
}

pub(crate) struct Edit {
    pub(crate) span: Range<usize>,
    pub(crate) replacement: String,
}

fn checked_span(source: &str, span: Range<usize>) -> eyre::Result<Range<usize>> {
    ensure!(
        source.get(span.clone()).is_some(),
        "invalid CSS UTF-8 byte span {span:?}"
    );
    Ok(span)
}

/// URL edits and future module edits share original UTF-8 coordinates.
pub(crate) fn apply_edits(source: &str, mut edits: Vec<Edit>) -> eyre::Result<String> {
    edits.sort_by_key(|edit| (edit.span.start, edit.span.end));
    let mut output = String::new();
    let mut start = 0;
    for edit in edits {
        let span = checked_span(source, edit.span)?;
        ensure!(span.start >= start, "overlapping CSS edits at {span:?}");
        output.push_str(&source[start..span.start]);
        output.push_str(&edit.replacement);
        start = span.end;
    }
    output.push_str(&source[start..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> eyre::Result<ModuleSyntax> {
        super::parse(Path::new("unicode.module.css"), source)
    }

    #[test]
    fn owns_decoded_names_with_original_utf8_spans_in_nested_selectors() {
        let source = r".caf\e9, #你好 { & > .\31 23:is(.nested, :global(.theme)) { color: red } }";
        let syntax = parse(source).unwrap();
        let names: Vec<_> = syntax
            .names
            .iter()
            .map(|selector| {
                (
                    &selector.kind,
                    selector.name.decoded.as_str(),
                    &source[selector.name.span.clone()],
                )
            })
            .collect();
        insta::assert_debug_snapshot!(names, @r#"
        [
            (
                Class,
                "café",
                "caf\\e9",
            ),
            (
                Id,
                "你好",
                "你好",
            ),
            (
                Class,
                "123",
                "\\31 23",
            ),
            (
                Class,
                "nested",
                "nested",
            ),
            (
                Class,
                "theme",
                "theme",
            ),
        ]
        "#);
        assert_eq!(syntax.scopes.len(), 1);
        assert_eq!(syntax.scopes[0].kind, ScopeKind::Global);
        assert_eq!(&source[syntax.scopes[0].span.clone()], ":global(.theme)");
        assert!(syntax.scopes[0].argument.is_some());
    }

    #[test]
    fn owns_scope_markers_composition_names_and_requests_without_resolving_them() {
        let source = r#"
        :local .button { composes: b\61 se other; }
        :global .theme { composes: shared from global; }
        .next { composes: b\61 se from "./base.module.css"; }
        "#;
        let syntax = parse(source).unwrap();
        assert_eq!(syntax.scopes[0].kind, ScopeKind::Local);
        assert_eq!(syntax.scopes[0].argument, None);
        assert_eq!(syntax.scopes[1].kind, ScopeKind::Global);
        let compositions = &syntax.compositions;
        assert_eq!(compositions.len(), 3);
        assert_eq!(compositions[0].names[0].decoded, "base");
        assert_eq!(compositions[0].names[1].decoded, "other");
        assert_eq!(compositions[0].source, CompositionSource::Local);
        assert_eq!(compositions[1].source, CompositionSource::Global);
        let CompositionSource::File(request) = &compositions[2].source else {
            panic!("missing file request")
        };
        assert_eq!(request.decoded, "./base.module.css");
        assert_eq!(&source[request.span.clone()], r#""./base.module.css""#);
        assert_eq!(&source[compositions[2].selector.clone().unwrap()], ".next");
        assert!(source[compositions[2].span.clone()].starts_with("composes:"));
    }

    #[test]
    fn zero_escaped_selectors_have_replacement_character_identity() {
        let source = r".\0, .\000000, .\0 x, .�, .�x, #\0, #� {}";
        let syntax = parse(source).unwrap();
        let names: Vec<_> = syntax
            .names
            .iter()
            .map(|selector| {
                (
                    &selector.kind,
                    selector.name.decoded.as_str(),
                    &source[selector.name.span.clone()],
                )
            })
            .collect();
        insta::assert_debug_snapshot!(names, @r#"
        [
            (
                Class,
                "�",
                "\\0",
            ),
            (
                Class,
                "�",
                "\\000000",
            ),
            (
                Class,
                "�x",
                "\\0 x",
            ),
            (
                Class,
                "�",
                "�",
            ),
            (
                Class,
                "�x",
                "�x",
            ),
            (
                Id,
                "�",
                "\\0",
            ),
            (
                Id,
                "�",
                "�",
            ),
        ]
        "#);
    }

    #[test]
    fn zero_escaped_composition_names_and_requests_match_replacement_characters() {
        let source = r#"
        .a { composes: \0  �; }
        .b { composes: \000000  � from global; }
        .c { composes: \0  � from "./\0.module.css"; }
        .d { composes: � from "./�.module.css"; }
        "#;
        let syntax = parse(source).unwrap();
        for composition in &syntax.compositions {
            for name in &composition.names {
                assert_eq!(name.decoded, "�");
            }
        }
        let CompositionSource::File(escaped) = &syntax.compositions[2].source else {
            panic!("missing escaped file request")
        };
        let CompositionSource::File(replacement) = &syntax.compositions[3].source else {
            panic!("missing replacement file request")
        };
        assert_eq!(escaped.decoded, "./�.module.css");
        assert_eq!(escaped.decoded, replacement.decoded);
        assert_eq!(&source[escaped.span.clone()], r#""./\0.module.css""#);
    }

    #[test]
    fn traverses_selector_bearing_pseudos_and_nested_at_rules() {
        let source = "@media screen { .a:has(> .b):nth-child(2n of #c)::slotted(.d) { & .e {} } }";
        let syntax = parse(source).unwrap();
        assert_eq!(
            syntax
                .names
                .iter()
                .map(|name| name.name.decoded.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "d", "e"]
        );
        let syntax = parse("@scope (.root) to (.limit) { @nest .parent & { .child {} } }").unwrap();
        assert_eq!(
            syntax
                .names
                .iter()
                .map(|name| name.name.decoded.as_str())
                .collect::<Vec<_>>(),
            ["root", "limit", "parent", "child"]
        );
    }

    #[test]
    fn rejects_malformed_recovered_and_raw_selector_boundaries() {
        for source in [
            ".a[ { color: red }",
            ".a, { color: red }",
            ".ok { color red; }",
            "50% { color: red }",
            ".a:unknown(.hidden) {}",
            ".a::unknown(.hidden) {}",
            ":local(.hidden) {}",
            "@supports selector(.hidden) { .a {} }",
            ".a { composes: base from; }",
        ] {
            let error = format!("{:#}", parse(source).unwrap_err());
            assert!(
                error.contains("unicode.module.css: CSS Modules syntax"),
                "{source}: {error}"
            );
            assert!(
                !error.contains("transformation is not implemented"),
                "{error}"
            );
        }
    }

    #[test]
    fn recovery_and_raw_nodes_never_return_partial_owned_facts() {
        for (source, diagnostic) in [
            (".ok:nth-of-type(2n of .hidden) {}", "Oxc CSS recovery at"),
            (".ok {} 50% { color: red }", "raw CSS selector at"),
            (
                ".ok:unknown(.hidden) {}",
                "raw CSS pseudo-class argument at",
            ),
            (
                "@unknown .hidden { .ok {} }",
                "unsupported CSS Modules at-rule prelude at",
            ),
        ] {
            let error = format!("{:#}", parse(source).unwrap_err());
            assert!(error.contains(diagnostic), "{source}: {error}");
        }
        let error = super::reject_unfinished(Path::new("empty.module.css"), "").unwrap_err();
        assert!(error
            .to_string()
            .contains("transformation is not implemented"));
    }

    #[test]
    fn edits_use_original_utf8_offsets_and_reject_overlaps_and_bad_spans() {
        let source = "你好 .class url(a)";
        assert_eq!(
            apply_edits(
                source,
                vec![
                    Edit {
                        span: 14..20,
                        replacement: "url(b)".into()
                    },
                    Edit {
                        span: 8..13,
                        replacement: "scoped".into()
                    },
                ]
            )
            .unwrap(),
            "你好 .scoped url(b)"
        );
        for spans in [
            vec![(1, 2)],
            vec![(4, 2)],
            vec![(0, 99)],
            vec![(7, 12), (10, 15)],
        ] {
            assert!(apply_edits(
                source,
                spans
                    .into_iter()
                    .map(|(start, end)| Edit {
                        span: start..end,
                        replacement: String::new()
                    })
                    .collect()
            )
            .is_err());
        }
    }
}
