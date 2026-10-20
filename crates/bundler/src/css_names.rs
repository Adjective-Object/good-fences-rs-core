//! Versioned, checkout-independent CSS selector identities and export surfaces.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use eyre::{ensure, WrapErr};
use sha2::{Digest, Sha256};

use crate::css_syntax::{self, Edit, Name};

pub(crate) const REVISION: &str = "css-local-v1";
pub(crate) const TRANSFORM_REVISION: &str = "css-composes-v3";

#[derive(Default)]
pub(crate) struct Registry {
    identities: BTreeMap<String, (String, String)>,
}

#[derive(Default)]
pub(crate) struct PreparedNames {
    pub mappings: BTreeMap<String, String>,
    pub edits: Vec<Edit>,
    pub ids: BTreeMap<String, String>,
}

impl Registry {
    pub(crate) fn prepare(
        &mut self,
        root: &Path,
        path: &Path,
        source: &str,
    ) -> eyre::Result<PreparedNames> {
        self.prepare_with(root, path, source, scoped_name)
    }

    fn prepare_with(
        &mut self,
        root: &Path,
        path: &Path,
        source: &str,
        hash: impl Fn(&str, &str) -> String,
    ) -> eyre::Result<PreparedNames> {
        let syntax = css_syntax::parse(path, source)?;
        let relative = path.strip_prefix(root)?;
        let relative = relative
            .components()
            .map(|component| match component {
                Component::Normal(value) => value
                    .to_str()
                    .ok_or_else(|| eyre::eyre!("non-UTF-8 CSS Modules path")),
                _ => eyre::bail!("CSS Modules path is not normalized: {}", path.display()),
            })
            .collect::<eyre::Result<Vec<_>>>()?
            .join("/");
        let mut prepared = PreparedNames::default();
        let mut exports: BTreeMap<String, &Name> = BTreeMap::new();
        for selector in &syntax.names {
            if !selector.local {
                continue;
            }
            let name = &selector.name;
            let export = camel_case(&name.decoded);
            ensure!(
                !export.is_empty(),
                "{}: unrepresentable CSS Modules export for {:?} at {:?}",
                path.display(),
                name.decoded,
                name.span
            );
            if let Some(previous) = exports.insert(export.clone(), name) {
                ensure!(
                    previous.decoded == name.decoded,
                    "{}: CSS Modules export collision {:?}: {:?} at {:?} and {:?} at {:?}",
                    path.display(),
                    export,
                    previous.decoded,
                    previous.span,
                    name.decoded,
                    name.span
                );
            }
            let scoped = hash(&relative, &name.decoded);
            let identity = (relative.clone(), name.decoded.clone());
            if let Some(previous) = self.identities.insert(scoped.clone(), identity.clone()) {
                ensure!(
                    previous == identity,
                    "CSS Modules scoped-name collision {scoped:?}: {previous:?} and {identity:?} at {:?}",
                    name.span
                );
            }
            prepared.mappings.insert(export, scoped.clone());
            if selector.kind == css_syntax::SelectorKind::Id {
                prepared.ids.insert(name.decoded.clone(), scoped.clone());
            }
            prepared.edits.push(Edit {
                span: name.span.clone(),
                replacement: scoped,
            });
        }
        prepared.edits = Self::scope_edits(source, &syntax, &prepared.edits)
            .wrap_err_with(|| format!("{}: CSS Modules scopes", path.display()))?;
        Self::compose(source, &syntax, &mut prepared)
            .wrap_err_with(|| format!("{}: CSS Modules composes", path.display()))?;
        let transformed = css_syntax::apply_edits(source, prepared.edits.clone())?;
        css_syntax::parse(path, &transformed).map_err(|error| {
            eyre::eyre!(
                "{}: unsupported scope result for original selector spans {:?}: {error:#}",
                path.display(),
                syntax.selectors
            )
        })?;
        Ok(prepared)
    }

    fn compose(
        source: &str,
        syntax: &css_syntax::ModuleSyntax,
        prepared: &mut PreparedNames,
    ) -> eyre::Result<()> {
        use css_syntax::{CompositionSource, SelectorKind};

        if syntax.compositions.is_empty() {
            return Ok(());
        }
        let own: BTreeMap<_, _> = syntax
            .names
            .iter()
            .filter(|name| name.local)
            .map(|name| {
                let name = name.name.decoded.as_str();
                (name, prepared.mappings[&camel_case(name)].clone())
            })
            .collect();
        let mut dependencies: BTreeMap<&str, Vec<(&Name, bool)>> = BTreeMap::new();
        for composition in &syntax.compositions {
            let span = &composition.span;
            ensure!(
                !matches!(composition.source, CompositionSource::File(_)),
                "cross-file composition is unsupported at {span:?}"
            );
            let selector = composition.selector.as_ref().ok_or_else(|| {
                eyre::eyre!(
                    "illegal placement at {span:?}; composes requires a single local class rule"
                )
            })?;
            let names: Vec<_> = syntax
                .names
                .iter()
                .filter(|name| {
                    selector.start <= name.name.span.start && name.name.span.end <= selector.end
                })
                .collect();
            let selector_edits = prepared
                .edits
                .iter()
                .filter(|edit| selector.start <= edit.span.start && edit.span.end <= selector.end)
                .map(|edit| Edit {
                    span: edit.span.start - selector.start..edit.span.end - selector.start,
                    replacement: edit.replacement.clone(),
                })
                .collect();
            let transformed = css_syntax::apply_edits(&source[selector.clone()], selector_edits)?;
            let mut input = cssparser::ParserInput::new(&transformed);
            let mut parser = cssparser::Parser::new(&mut input);
            let single_class = matches!(parser.next(), Ok(cssparser::Token::Delim('.')))
                && matches!(parser.next(), Ok(cssparser::Token::Ident(_)))
                && parser.is_exhausted();
            ensure!(
                !composition.nested
                    && single_class
                    && names.len() == 1
                    && names[0].local
                    && names[0].kind == SelectorKind::Class,
                "illegal placement/multi-target selector at {selector:?}, declaration {span:?}; composes requires a single local class rule"
            );
            let global = composition.source == CompositionSource::Global;
            let target = dependencies
                .entry(names[0].name.decoded.as_str())
                .or_default();
            for name in &composition.names {
                ensure!(
                    global || own.contains_key(name.decoded.as_str()),
                    "unknown local name {:?} at {:?}, declaration {span:?}",
                    name.decoded,
                    name.span
                );
                target.push((name, global));
            }
            prepared.edits.push(Edit {
                span: span.clone(),
                replacement: String::new(),
            });
        }

        let mut resolved: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for &root in own.keys() {
            if resolved.contains_key(root) {
                continue;
            }
            let mut stack = vec![(root, 0)];
            let mut active = BTreeSet::from([root]);
            while let Some((name, index)) = stack.last_mut() {
                let edges = dependencies.get(name).map_or(&[][..], Vec::as_slice);
                if let Some((dependency, global)) = edges.get(*index) {
                    *index += 1;
                    let next = dependency.decoded.as_str();
                    if *global || resolved.contains_key(next) {
                        continue;
                    }
                    ensure!(
                        active.insert(next),
                        "composition cycle {} -> {next} at {:?}",
                        stack
                            .iter()
                            .map(|(name, _)| *name)
                            .collect::<Vec<_>>()
                            .join(" -> "),
                        dependency.span
                    );
                    stack.push((next, 0));
                    continue;
                }
                let mut tokens = vec![own[name].clone()];
                let mut seen = BTreeSet::from([tokens[0].clone()]);
                for (dependency, global) in edges {
                    let values = if *global {
                        std::slice::from_ref(&dependency.decoded)
                    } else {
                        &resolved[dependency.decoded.as_str()]
                    };
                    for token in values {
                        if seen.insert(token.clone()) {
                            tokens.push(token.clone());
                        }
                    }
                }
                resolved.insert(name, tokens);
                active.remove(name);
                stack.pop();
            }
        }
        for (name, tokens) in resolved {
            prepared.mappings.insert(camel_case(name), tokens.join(" "));
        }
        Ok(())
    }

    fn scope_edits(
        source: &str,
        syntax: &css_syntax::ModuleSyntax,
        names: &[Edit],
    ) -> eyre::Result<Vec<Edit>> {
        use std::ops::Range;

        fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
            outer.start <= inner.start && inner.end <= outer.end
        }

        fn trivia(source: &str) -> eyre::Result<String> {
            let mut input = cssparser::ParserInput::new(source);
            let mut parser = cssparser::Parser::new(&mut input);
            let mut comments = String::new();
            loop {
                let start = parser.position().byte_index();
                match parser.next_including_whitespace_and_comments() {
                    Ok(cssparser::Token::Comment(_)) => {
                        comments.push_str(&source[start..parser.position().byte_index()])
                    }
                    Ok(cssparser::Token::WhiteSpace(_)) => {}
                    Err(error)
                        if matches!(error.kind, cssparser::BasicParseErrorKind::EndOfInput) =>
                    {
                        break
                    }
                    _ => eyre::bail!("ambiguous CSS scope selector trivia"),
                }
            }
            Ok(comments)
        }

        fn render(
            source: &str,
            syntax: &css_syntax::ModuleSyntax,
            names: &[Edit],
            region: &Range<usize>,
            depth: usize,
        ) -> eyre::Result<Vec<String>> {
            ensure!(
                depth < 128,
                "CSS Modules scope expansion nesting limit exceeded at {region:?}"
            );
            let scopes: Vec<_> = syntax
                .scopes
                .iter()
                .filter(|scope| scope.owner == *region)
                .collect();
            let children: Vec<_> = syntax
                .selectors
                .iter()
                .filter(|child| {
                    *child != region
                        && contains(region, child)
                        && !scopes.iter().any(|scope| contains(&scope.span, child))
                        && !syntax.selectors.iter().any(|parent| {
                            parent != region
                                && parent != *child
                                && contains(region, parent)
                                && contains(parent, child)
                        })
                })
                .collect();
            let mut edits: Vec<_> = names
                .iter()
                .filter(|edit| {
                    contains(region, &edit.span)
                        && !scopes.iter().any(|scope| contains(&scope.span, &edit.span))
                        && !children.iter().any(|child| contains(child, &edit.span))
                })
                .cloned()
                .collect();
            for child in children {
                edits.push(Edit {
                    span: child.clone(),
                    replacement: render(source, syntax, names, child, depth + 1)?.join(", "),
                });
            }
            let mut variants = vec![edits];
            for scope in scopes {
                let alternatives = if let Some(argument) = &scope.argument {
                    let mut alternatives = Vec::new();
                    for (index, selector) in scope.alternatives.iter().enumerate() {
                        let start = index
                            .checked_sub(1)
                            .map_or(argument.start + 1, |previous| scope.commas[previous].end);
                        let end = scope
                            .commas
                            .get(index)
                            .map_or(argument.end - 1, |comma| comma.start);
                        for text in render(source, syntax, names, selector, depth + 1)? {
                            if !scope.first_in_compound {
                                let mut input = cssparser::ParserInput::new(&text);
                                let mut parser = cssparser::Parser::new(&mut input);
                                ensure!(
                                    !matches!(
                                        parser.next(),
                                        Ok(cssparser::Token::Ident(_)
                                            | cssparser::Token::Delim('*' | '|'))
                                    ),
                                    "ambiguous CSS scope type selector at {:?}",
                                    scope.span
                                );
                            }
                            alternatives.push(format!(
                                "{}{text}{}",
                                trivia(&source[start..selector.start])?,
                                trivia(&source[selector.end..end])?
                            ));
                        }
                    }
                    alternatives
                } else {
                    vec![String::new()]
                };
                ensure!(
                    variants.len().saturating_mul(alternatives.len()) <= 4096,
                    "CSS Modules selector expansion limit exceeded at {:?}",
                    scope.span
                );
                variants = variants
                    .into_iter()
                    .flat_map(|edits| {
                        alternatives.iter().map(move |replacement| {
                            let mut edits = edits.clone();
                            edits.push(Edit {
                                span: scope.span.clone(),
                                replacement: replacement.clone(),
                            });
                            edits
                        })
                    })
                    .collect();
            }
            variants
                .into_iter()
                .map(|edits| {
                    css_syntax::apply_edits(
                        &source[region.clone()],
                        edits
                            .into_iter()
                            .map(|edit| Edit {
                                span: edit.span.start - region.start..edit.span.end - region.start,
                                replacement: edit.replacement,
                            })
                            .collect(),
                    )
                })
                .collect()
        }

        let mut regions = syntax.selectors.clone();
        regions.sort_by_key(|span| (span.start, std::cmp::Reverse(span.end)));
        regions.dedup();
        let mut edits: Vec<Edit> = Vec::new();
        for region in regions {
            if edits
                .last()
                .is_some_and(|edit| contains(&edit.span, &region))
            {
                continue;
            }
            edits.push(Edit {
                replacement: render(source, syntax, names, &region, 0)?.join(", "),
                span: region,
            });
        }
        Ok(edits)
    }
}

fn camel_case(name: &str) -> String {
    let mut output = String::new();
    let mut upper = false;
    let chars: Vec<_> = name.chars().collect();
    for (index, &ch) in chars.iter().enumerate() {
        if matches!(ch, '-' | '_') {
            upper = true;
        } else {
            let boundary = ch.is_uppercase()
                && index > 0
                && (chars[index - 1].is_lowercase()
                    || chars[index - 1].is_numeric()
                    || chars[index - 1].is_uppercase()
                        && chars.get(index + 1).is_some_and(|ch| ch.is_lowercase()));
            if !output.is_empty() && (upper || boundary) {
                output.extend(ch.to_uppercase());
            } else {
                output.extend(ch.to_lowercase());
            }
            upper = false;
        }
    }
    output
}

fn scoped_name(path: &str, symbol: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(REVISION.as_bytes());
    digest.update([0]);
    for value in [path, symbol] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut name = String::from("_");
    for index in 0..10 {
        let bit = index * 6;
        let pair = u16::from(digest[bit / 8]) << 8 | u16::from(digest[bit / 8 + 1]);
        name.push(alphabet[((pair >> (10 - bit % 8)) & 63) as usize] as char);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_camel_case_handles_word_boundaries_without_dropping_nonidentifiers() {
        for (input, expected) in [
            ("my-class-name", "myClassName"),
            ("my_class_name", "myClassName"),
            ("myClassName", "myClassName"),
            ("--XMLHttp-request", "xmlHttpRequest"),
            ("_Upper_NAME", "upperName"),
            ("class", "class"),
            ("123", "123"),
            ("a+b", "a+b"),
            ("你好", "你好"),
        ] {
            assert_eq!(camel_case(input), expected);
        }
    }

    #[test]
    fn names_depend_only_on_relative_path_and_decoded_symbol() {
        let prepare = |root: &str, source| {
            Registry::default()
                .prepare(
                    Path::new(root),
                    &Path::new(root).join("a.module.css"),
                    source,
                )
                .unwrap()
        };
        let a = prepare("/checkout/one", r".my-class, #my-class, .caf\e9 {}");
        let b = prepare("/checkout/two", ".café {color:red} #my-class {}");
        assert_eq!(a.mappings, b.mappings);
        assert_eq!(&a.edits[0].replacement[1..], &a.edits[1].replacement[1..]);
        assert_ne!(
            scoped_name("a.module.css", "café"),
            scoped_name("b.module.css", "café")
        );
        for name in a.mappings.values() {
            assert_eq!(name.len(), 11);
            assert!(name.starts_with('_'));
            assert!(name
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-')));
        }
    }

    #[test]
    fn scopes_follow_ast_boundaries_and_expand_lists_without_changing_specificity() {
        for (source, expected, exports) in [
            (
                ":global(.theme) :local(.button) {}",
                ".theme ._button {}",
                vec!["button"],
            ),
            (
                ":global .theme > .other, .button {}",
                " .theme > .other, ._button {}",
                vec!["button"],
            ),
            (
                ":global .theme :local .button + #target {}",
                " .theme  ._button + #_target {}",
                vec!["button", "target"],
            ),
            (
                ":global(.a, .b) > :local(.c, .d) {}",
                ".a > ._c, .a > ._d, .b > ._c, .b > ._d {}",
                vec!["c", "d"],
            ),
            (
                ":global(:local(.a, .b)) .c {}",
                "._a ._c, ._b ._c {}",
                vec!["a", "b", "c"],
            ),
            (
                ".root:is(:global(.a, .b), :local(.c)):not(:global #d, .e):has(> :local(.f)) {}",
                "._root:is(.a, .b, ._c):not( #d, ._e):has(> ._f) {}",
                vec!["c", "e", "f", "root"],
            ),
            (
                ".x:has(> :global(.a,.b)) {}",
                "._x:has(> .a, > .b) {}",
                vec!["x"],
            ),
            (
                ".x:has(+ :local(.a,.b)) {}",
                "._x:has(+ ._a, + ._b) {}",
                vec!["a", "b", "x"],
            ),
            (
                ".x:has(~ /*keep*/ :global(.a,.b)) {}",
                "._x:has(~ /*keep*/ .a, ~ /*keep*/ .b) {}",
                vec!["x"],
            ),
            (
                ".a:global(.b).c { & > :global(.d) :local(.e) {} }",
                "._a.b._c { & > .d ._e {} }",
                vec!["a", "c", "e"],
            ),
            (
                r":glo\62 al(.外部) :l\6f cal(.caf\e9, #你好) {}",
                ".外部 ._café, .外部 #_你好 {}",
                vec!["café", "你好"],
            ),
            (
                ".a:global( /*keep*/ .b , .c /*last*/ ).d {}",
                "._a/*keep*/.b._d, ._a.c/*last*/._d {}",
                vec!["a", "d"],
            ),
            (
                ":GLOBAL(.foo-bar, .---) .fooBar {}",
                ".foo-bar ._fooBar, .--- ._fooBar {}",
                vec!["fooBar"],
            ),
            (
                ":local(/*before*/.a/*one*/,/*two*/.b/*after*/) {}",
                "/*before*/._a/*one*/, /*two*/._b/*after*/ {}",
                vec!["a", "b"],
            ),
            (
                ".a::slotted(:global(.b)):nth-child(2n of :global .c, .d) .e {}",
                "._a::slotted(.b):nth-child(2n of  .c, ._d) ._e {}",
                vec!["a", "d", "e"],
            ),
        ] {
            let prepared = Registry::default()
                .prepare(
                    Path::new("/project"),
                    Path::new("/project/a.module.css"),
                    source,
                )
                .unwrap();
            assert_eq!(
                prepared
                    .mappings
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                exports,
                "{source}"
            );
            let mut output = css_syntax::apply_edits(source, prepared.edits).unwrap();
            for (export, scoped) in prepared.mappings {
                output = output.replace(&scoped, &format!("_{export}"));
            }
            assert_eq!(output, expected, "{source}");
        }
    }

    #[test]
    fn scopes_preserve_at_rules_attributes_strings_and_unrelated_identifiers() {
        let source = r#"@charset "UTF-8"; @layer base; @media screen { @supports (display: grid) and selector(:global(.theme) > .a) { @layer base { :local(.a)[class=".a"][id="target"]::before { content: ":local(.a) #target"; --a: a; color: red } } } } @scope (:global(.root)) to (.limit) { @nest & :local(.a) {} }"#;
        let prepared = Registry::default()
            .prepare(
                Path::new("/project"),
                Path::new("/project/a.module.css"),
                source,
            )
            .unwrap();
        let mut output = css_syntax::apply_edits(source, prepared.edits).unwrap();
        for (export, scoped) in prepared.mappings {
            output = output.replace(&scoped, &format!("_{export}"));
        }
        assert_eq!(
            output,
            r#"@charset "UTF-8"; @layer base; @media screen { @supports (display: grid) and selector(.theme > ._a) { @layer base { ._a[class=".a"][id="target"]::before { content: ":local(.a) #target"; --a: a; color: red } } } } @scope (.root) to (._limit) { @nest & ._a {} }"#
        );
    }

    #[test]
    fn scopes_reject_empty_recovered_raw_and_ambiguous_selectors_with_original_spans() {
        for source in [
            ":local() {}",
            ":global() {}",
            ":local(/*empty*/) {}",
            ":local {}",
            ":global /*empty*/ {}",
            ":global > .a {}",
            ":local(.a,) {}",
            ":local(.a; .b) {}",
            ":local(.a {} .b) {}",
            ":local(.a:unknown(.hidden)) {}",
            ":global(.a:nth-of-type(2n of .b)) {}",
            ".a:local(div) {}",
            ".a:global(:local(div)) {}",
            ":local(div):global(span) {}",
            ":global(.a)div {}",
            ":local(> .a) {}",
            "@supports selector(:local(.a,.b)) { .c {} }",
            ".a::slotted(:local(.b,.c)) {}",
            ".a:host(:local(.b,.c)) {}",
            ".a:host-context(:global(.b .c)) {}",
        ] {
            let error = Registry::default()
                .prepare(
                    Path::new("/project"),
                    Path::new("/project/a.module.css"),
                    source,
                )
                .err();
            assert!(error.is_some(), "accepted {source}");
        }
        let error = format!(
            "{:#}",
            Registry::default()
                .prepare(
                    Path::new("/project"),
                    Path::new("/project/a.module.css"),
                    ".你好:local(.a:unknown(.hidden)) {}",
                )
                .err()
                .unwrap()
        );
        assert!(
            error.contains("a.module.css") && error.contains("24..33"),
            "{error}"
        );
        let source = format!("{} {{}}", ":global(.a,.b)".repeat(13));
        let error = format!(
            "{:#}",
            Registry::default()
                .prepare(
                    Path::new("/project"),
                    Path::new("/project/a.module.css"),
                    &source
                )
                .err()
                .unwrap()
        );
        assert!(
            error.contains("a.module.css") && error.contains("expansion limit exceeded at"),
            "{error}"
        );
    }

    #[test]
    fn composition_resolves_forward_chains_in_declaration_order_with_own_token_first() {
        let source = r#"
:local(.button) { composes: mid base mid; color: red; composes: shared base from global; }
.unused { composes: button; color: blue }
.mid { composes: b\61 se; composes: shared shared from global; }
.base { color: green }
.button { composes: base; }
:local .global-only { composes: 你好 你好 other from global; }
"#;
        let prepared = Registry::default()
            .prepare_with(
                Path::new("/project"),
                Path::new("/project/style.module.css"),
                source,
                |_, name| format!("_{name}"),
            )
            .unwrap();
        assert_eq!(prepared.mappings["base"], "_base");
        assert_eq!(prepared.mappings["mid"], "_mid _base shared");
        assert_eq!(
            prepared.mappings["button"],
            "_button _mid _base shared base"
        );
        assert_eq!(
            prepared.mappings["unused"],
            "_unused _button _mid _base shared base"
        );
        assert_eq!(prepared.mappings["globalOnly"], "_global-only 你好 other");
        let css = css_syntax::apply_edits(source, prepared.edits).unwrap();
        assert!(!css.contains("composes"));
        assert!(css.contains("color: red") && css.contains("color: blue"));
    }

    #[test]
    fn composition_errors_include_original_paths_and_spans_even_for_unused_names() {
        for (source, message, span) in [
            (".a { composes: missing }", "unknown local name", "15..22"),
            (".a { composes: a }", "a -> a", "15..16"),
            (
                ".a { composes: b } .b { composes: a }",
                "a -> b -> a",
                "34..35",
            ),
            (".a,.b { composes: a }", "multi-target", "0..5"),
            (".a.b { composes: a }", "multi-target", "0..4"),
            (".a:hover { composes: a }", "multi-target", "0..8"),
            (
                ":global(.a) { composes: a from global }",
                "illegal placement",
                "0..11",
            ),
            ("#a { composes: a }", "illegal placement", "0..2"),
            (".a { .b { composes: a } }", "illegal placement", "5..7"),
            (".a { & { composes: a } }", "illegal placement", "5..6"),
            ("@font-face { composes: a }", "illegal placement", "13..24"),
            (
                ".a { composes: a from './b.module.css' }",
                "cross-file composition is unsupported",
                "5..38",
            ),
        ] {
            let error = format!(
                "{:#}",
                Registry::default()
                    .prepare(
                        Path::new("/project"),
                        Path::new("/project/style.module.css"),
                        source,
                    )
                    .err()
                    .unwrap()
            );
            assert!(
                error.contains("style.module.css")
                    && error.contains(message)
                    && error.contains(span),
                "{source}: {error}"
            );
        }
    }

    #[test]
    fn rejects_export_and_detected_hash_collisions_and_future_syntax() {
        let root = Path::new("/project");
        let path = root.join("a.module.css");
        let error = Registry::default()
            .prepare(root, &path, ".foo-bar, #fooBar {}")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("export collision")
                && error.contains("foo-bar")
                && error.contains("fooBar")
                && error.contains("1..8")
                && error.contains("11..17"),
            "{error}"
        );
        let error = Registry::default()
            .prepare_with(root, &path, ".a, .b {}", |_, _| "_collision".into())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("scoped-name collision"), "{error}");
        let mut registry = Registry::default();
        registry
            .prepare_with(root, &path, ".a, #a {}", |_, _| "_collision".into())
            .unwrap();
        assert!(registry
            .prepare_with(root, &root.join("b.module.css"), ".a {}", |_, _| {
                "_collision".into()
            })
            .is_err());
        for source in [".a {composes: b}", ".--- {}"] {
            assert!(
                Registry::default().prepare(root, &path, source).is_err(),
                "{source}"
            );
        }
    }
}
