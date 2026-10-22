use std::collections::{HashMap, HashSet};

use ast_segmenter::{
    emission::Shorthand,
    owned::{BindingId, DependencyKind},
    views::*,
};
use eyre::{ensure, OptionExt};
use oxc_span::Span;

use crate::validate::ModuleView;
use crate::validate::{contains, span};

fn strict(values: impl IntoIterator<Item = u32>) -> bool {
    let mut previous = None;
    for value in values {
        if previous.is_some_and(|p| p >= value) {
            return false;
        }
        previous = Some(value);
    }
    true
}

fn within(source: &str, outer: Span, inner: Span) -> eyre::Result<()> {
    span(source, inner)?;
    ensure!(
        inner.start < inner.end && contains(outer, inner),
        "invalid emission containment"
    );
    Ok(())
}

pub(crate) fn validate(module: &ModuleView<'_>, source: &str) -> eyre::Result<()> {
    let facts = module.facts();
    let emission = &facts.emission();
    if let Some(directive) = emission.source_map() {
        span(source, directive)?;
        ensure!(
            ast_segmenter::emission::source_mapping_url(
                &source[directive.start as usize..directive.end as usize]
            )
            .is_some(),
            "invalid source map directive"
        );
    }
    execution(module, source)?;
    ensure!(
        emission.segments().len() == module.segments().len(),
        "emission segment count mismatch"
    );
    ensure!(
        emission
            .dynamic_namespaces()
            .iter()
            .map(|n| n.dependency())
            .eq(facts
                .dependencies()
                .iter()
                .filter(|d| d.kind() == DependencyKind::DynamicImport)
                .map(|d| d.id())),
        "invalid dynamic namespace dependency links"
    );
    for observation in emission.dynamic_namespaces() {
        if let Some(properties) = &observation.properties() {
            ensure!(
                properties
                    .iter()
                    .zip(properties.iter().skip(1))
                    .all(|(a, b)| a < b),
                "invalid dynamic namespace properties"
            );
        }
    }
    ensure!(
        emission.nested_bindings()
            == facts
                .bindings()
                .iter()
                .filter(|b| !b.top_level())
                .map(|b| b.id())
                .collect::<Vec<_>>()
            && emission.free_references()
                == facts
                    .references()
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.binding().is_none())
                    .map(|(i, _)| i as u32)
                    .collect::<Vec<_>>(),
        "invalid emission capture links"
    );
    for (&range, recipe) in module.segments().iter().zip(emission.segments()) {
        let pieces = recipe.source().spans();
        ensure!(
            !pieces.is_empty() && pieces.len() <= 4,
            "invalid source pieces"
        );
        for piece in pieces {
            span(source, piece)?;
        }
        ensure!(
            recipe.source().contains(range)
                && pieces
                    .iter()
                    .zip(pieces.iter().skip(1))
                    .all(|(a, b)| a.end <= b.start)
                && (pieces == [range]
                    || (pieces.len() == 2
                        && pieces.get(1) == Some(range)
                        && pieces.get(0).unwrap().end <= range.start)),
            "invalid segment source assembly"
        );
        placement(facts, recipe, range)?;
        namespaces(facts, recipe, source, range)?;
        operations(facts, recipe, source, range)?;
        if facts.commonjs() {
            ensure!(
                !recipe.connector_only()
                    && matches!(recipe.deferred(), DeferredForm::Unsupported)
                    && recipe.placement().declarations().is_empty()
                    && recipe.placement().hoisted().is_empty()
                    && !recipe.placement().instantiable()
                    && !recipe.placement().default_expression()
                    && recipe.placement().forward_references().is_empty()
                    && recipe.placement().eager_reads().is_empty()
                    && recipe.placement().eager_writes().is_empty()
                    && recipe.placement().writes().is_empty()
                    && recipe.names().named().is_empty()
                    && !recipe.names().inference_sensitive()
                    && recipe.namespaces().is_empty()
                    && recipe.operations().iter().all(|o| matches!(
                        o.kind(),
                        OperationKind::Dependency { .. } | OperationKind::SourceHint { .. }
                    )),
                "nonopaque CommonJS emission recipe"
            );
        }
    }
    lazy_candidates(module, source)?;
    Ok(())
}

fn lazy_candidates(module: &ModuleView<'_>, source: &str) -> eyre::Result<()> {
    use ast_segmenter::emission::LazyWrapperKind;
    let facts = module.facts();
    let emission = facts.emission();
    ensure!(
        !facts.commonjs()
            || (emission.lazy_modules().is_empty() && emission.lazy_selectors().is_empty()),
        "nonopaque CommonJS lazy candidates"
    );
    ensure!(
        strict(
            emission
                .lazy_modules()
                .iter()
                .map(|c| c.construction().start)
        ) && strict(
            emission
                .lazy_selectors()
                .iter()
                .map(|c| c.construction().start)
        ),
        "invalid lazy candidate order"
    );
    for candidate in emission.lazy_modules() {
        let call = lazy_construction(
            facts,
            source,
            candidate.segment(),
            candidate.constructor(),
            candidate.construction(),
            candidate.arguments(),
        )?;
        ensure!(!candidate.arguments().is_empty(), "missing lazy callback");
        let binding = facts
            .bindings()
            .iter()
            .find(|b| b.id() == candidate.binding())
            .ok_or_eyre("dangling lazy module binding")?;
        let range = module
            .segments()
            .get(candidate.segment() as usize)
            .copied()
            .ok_or_eyre("dangling lazy module segment")?;
        within(source, range, candidate.declaration())?;
        ensure!(
            !binding.import()
                && contains(candidate.declaration(), candidate.construction())
                && binding
                    .declarations()
                    .iter()
                    .any(|d| d.start == candidate.declaration().start && d.end < call.span().start),
            "invalid lazy module declaration link"
        );
        let dependency = facts
            .dependencies()
            .iter()
            .find(|d| d.id() == candidate.dependency())
            .ok_or_eyre("dangling lazy callback dependency")?;
        within(source, candidate.callback().span(), candidate.import_span())?;
        let callback_context = emission
            .execution()
            .contexts()
            .iter()
            .find(|c| wrapped(source, candidate.callback().span(), c.span()))
            .ok_or_eyre("missing lazy callback context")?;
        ensure!(
            dependency.kind() == DependencyKind::DynamicImport
                && dependency.pattern().is_none()
                && dependency.span() == candidate.import_span()
                && callback_context.kind() == ast_segmenter::execution::ContextKind::Callable
                && dependency.enclosing_function() == Some(callback_context.span()),
            "invalid lazy callback dependency"
        );
        direct_arrow(
            source,
            callback_context.span(),
            None,
            candidate.import_span(),
            None,
        )?;
        ensure!(
            !call.arguments().get(0).unwrap().spread(),
            "spread lazy callback"
        );
        if let Some(properties) = candidate.options_properties() {
            let options = candidate
                .options()
                .ok_or_eyre("missing literal lazy options")?;
            let mut previous = options.span().start;
            for (index, property) in properties.iter().enumerate() {
                within(source, options.span(), property.key())?;
                within(source, options.span(), property.value())?;
                ensure!(
                    previous <= property.key().start
                        && property.key().start < property.key().end
                        && property.value().start < property.value().end
                        && (property.key() == property.value()
                            || property.key().end <= property.value().start),
                    "invalid lazy options property spans"
                );
                let key = &source[property.key().start as usize..property.key().end as usize];
                let name = if key.starts_with(['\'', '"']) {
                    ensure!(
                        key.len() >= 2
                            && key.as_bytes().first() == key.as_bytes().last()
                            && !key.contains('\\'),
                        "unsupported lazy options key"
                    );
                    &key[1..key.len() - 1]
                } else {
                    key
                };
                ensure!(
                    syntax_tokens(key).is_some_and(|t| t == [key])
                        && !identifier_matches(name, "initializer")
                        && !identifier_matches(name, "__proto__"),
                    "unsafe lazy options key"
                );
                ensure!(
                    syntax_tokens(&source[previous as usize..property.key().start as usize])
                        .is_some_and(|t| t == [if index == 0 { "{" } else { "," }]),
                    "incomplete lazy options properties"
                );
                if property.key() != property.value() {
                    ensure!(
                        syntax_tokens(
                            &source[property.key().end as usize..property.value().start as usize]
                        )
                        .is_some_and(|t| t == [":"]),
                        "nonplain lazy options property"
                    );
                }
                let tokens = syntax_tokens(
                    &source[property.value().start as usize..property.value().end as usize],
                )
                .ok_or_eyre("invalid lazy options value")?;
                let mut delimiters = Vec::new();
                for token in tokens {
                    match token {
                        "(" => delimiters.push(")"),
                        "[" => delimiters.push("]"),
                        "{" => delimiters.push("}"),
                        ")" | "]" | "}" => ensure!(
                            delimiters.pop() == Some(token),
                            "unbalanced lazy options value"
                        ),
                        "," => ensure!(!delimiters.is_empty(), "incomplete lazy options value"),
                        "/" => eyre::bail!("unsupported lazy options slash token"),
                        _ => {}
                    }
                }
                ensure!(delimiters.is_empty(), "unbalanced lazy options value");
                previous = property.value().end;
            }
            ensure!(
                syntax_tokens(&source[previous as usize..options.span().end as usize]).is_some_and(
                    |t| if properties.is_empty() {
                        t == ["{", "}"]
                    } else {
                        t == ["}"] || t == [",", "}"]
                    }
                ),
                "incomplete lazy options object"
            );
        }
    }
    for candidate in emission.lazy_selectors() {
        match (candidate.result_binding(), candidate.result_declaration()) {
            (None, None) => {}
            (Some(binding), Some(declaration)) => {
                let binding = facts
                    .bindings()
                    .iter()
                    .find(|b| b.id() == binding)
                    .ok_or_eyre("dangling lazy result binding")?;
                span(source, declaration)?;
                ensure!(
                    emission
                        .segments()
                        .get(candidate.segment() as usize)
                        .is_some_and(|recipe| recipe.source().contains(declaration)),
                    "invalid lazy result declaration owner"
                );
                let name = binding
                    .declarations()
                    .iter()
                    .find(|d| d.start == declaration.start)
                    .ok_or_eyre("invalid lazy result declaration")?;
                ensure!(
                    name.end < candidate.construction().start
                        && declaration.end >= candidate.construction().end,
                    "invalid lazy result initializer"
                );
                let prefix = syntax_tokens(
                    &source[name.end as usize..candidate.construction().start as usize],
                )
                .ok_or_eyre("invalid lazy result initializer prefix")?;
                let suffix = syntax_tokens(
                    &source[candidate.construction().end as usize..declaration.end as usize],
                )
                .ok_or_eyre("invalid lazy result initializer suffix")?;
                ensure!(
                    prefix.first() == Some(&"=")
                        && prefix[1..].iter().all(|token| *token == "(")
                        && suffix.len() + 1 == prefix.len()
                        && suffix.iter().all(|token| *token == ")"),
                    "invalid lazy result initializer boundary"
                );
            }
            _ => eyre::bail!("incomplete lazy result evidence"),
        }
        let call = lazy_construction(
            facts,
            source,
            candidate.segment(),
            candidate.constructor(),
            candidate.construction(),
            candidate.arguments(),
        )?;
        ensure!(
            candidate.arguments().len() >= 2,
            "missing lazy selector arguments"
        );
        let reference = facts
            .references()
            .get(candidate.module_reference() as usize)
            .ok_or_eyre("dangling lazy module reference")?;
        ensure!(
            reference.binding() == Some(candidate.module_binding())
                && reference.read()
                && !reference.write()
                && !reference.type_only()
                && contains(candidate.module_argument().span(), reference.span())
                && !call.arguments().get(0).unwrap().spread()
                && !call.arguments().get(1).unwrap().spread(),
            "invalid lazy selector module link"
        );
        let getter = facts
            .references()
            .get(candidate.getter_reference() as usize)
            .ok_or_eyre("dangling lazy getter reference")?;
        let parameter = facts
            .bindings()
            .iter()
            .find(|b| Some(b.id()) == getter.binding())
            .ok_or_eyre("dangling lazy getter parameter")?;
        within(source, candidate.getter().span(), candidate.property_span())?;
        ensure!(
            getter.read()
                && !getter.write()
                && !getter.type_only()
                && contains(candidate.getter().span(), getter.span())
                && parameter.declarations().len() == 1
                && parameter
                    .declarations()
                    .iter()
                    .all(|d| contains(candidate.getter().span(), d) && d.end < getter.span().start)
                && facts
                    .references()
                    .iter()
                    .filter(|r| !r.type_only() && r.binding() == Some(parameter.id()))
                    .count()
                    == 1
                && getter.span().end < candidate.property_span().start
                && !candidate.property().is_empty()
                && property_token_matches(
                    source,
                    getter.span(),
                    candidate.property_span(),
                    candidate.property(),
                ),
            "invalid lazy getter link"
        );
        let getter_context = emission
            .execution()
            .contexts()
            .iter()
            .find(|c| {
                c.kind() == ast_segmenter::execution::ContextKind::Callable
                    && wrapped(source, candidate.getter().span(), c.span())
            })
            .ok_or_eyre("missing lazy getter context")?;
        ensure!(
            wrapped(source, candidate.module_argument().span(), reference.span()),
            "invalid lazy module argument expression"
        );
        direct_arrow(
            source,
            getter_context.span(),
            parameter.declarations().get(0),
            getter.span(),
            Some(candidate.property_span()),
        )?;
        let constructor = facts
            .references()
            .get(candidate.constructor() as usize)
            .unwrap();
        let imported_name = facts
            .imports()
            .iter()
            .find(|i| Some(i.binding()) == constructor.binding())
            .and_then(|i| match i.imported() {
                ImportedName::Name(name) => Some(name),
                ImportedName::Namespace => None,
            });
        ensure!(
            candidate.kind()
                == LazyWrapperKind::from_name(imported_name.unwrap_or(constructor.name())),
            "invalid lazy wrapper kind hint"
        );
    }
    Ok(())
}

// Only delimiters/trivia may surround an anchored expression. Containment alone
// would accept a nested import/member inside a call, sequence or async arrow.
fn wrapped(source: &str, outer: Span, inner: Span) -> bool {
    if !contains(outer, inner) {
        return false;
    }
    let Some(before) = syntax_tokens(&source[outer.start as usize..inner.start as usize]) else {
        return false;
    };
    let Some(after) = syntax_tokens(&source[inner.end as usize..outer.end as usize]) else {
        return false;
    };
    before.iter().all(|t| *t == "(")
        && after.len() == before.len()
        && after.iter().all(|t| *t == ")")
}

// This is a bounded token-boundary check, not a parser. Anchors come from the
// canonical invocation/operation and execution records; no AST is built on hits.
fn direct_arrow(
    source: &str,
    arrow: Span,
    parameter: Option<Span>,
    returned: Span,
    property: Option<Span>,
) -> eyre::Result<()> {
    within(source, arrow, returned)?;
    let mut header_start = arrow.start;
    let parenthesized = if let Some(parameter) = parameter {
        within(source, arrow, parameter)?;
        ensure!(
            parameter.end < returned.start,
            "invalid lazy parameter order"
        );
        let prefix = syntax_tokens(&source[arrow.start as usize..parameter.start as usize])
            .ok_or_eyre("invalid lazy parameter prefix")?;
        ensure!(
            prefix.is_empty()
                || (prefix.last() == Some(&"(") && type_parameters(&prefix[..prefix.len() - 1])),
            "non-simple or asynchronous lazy parameter"
        );
        header_start = parameter.end;
        !prefix.is_empty()
    } else {
        true
    };
    let header = syntax_tokens(&source[header_start as usize..returned.start as usize])
        .ok_or_eyre("invalid lazy arrow header")?;
    let arrow_token = header
        .iter()
        .rposition(|t| *t == "=>")
        .ok_or_eyre("missing synchronous lazy arrow")?;
    let (parameters, body) = header.split_at(arrow_token);
    ensure!(
        body[1..].iter().all(|t| *t == "("),
        "non-direct lazy arrow return"
    );
    if parameter.is_none() {
        let open = parameters
            .iter()
            .enumerate()
            .find(|(i, t)| **t == "(" && type_parameters(&parameters[..*i]))
            .map(|(i, _)| i)
            .ok_or_eyre("missing lazy callback parameters")?;
        ensure!(
            parameters[open..].starts_with(&["(", ")"])
                && return_annotation(&parameters[open + 2..]),
            "nonempty or asynchronous lazy callback parameters"
        );
    } else if parenthesized {
        // Find the parameter-list close without confusing nested type syntax
        // (including function types) with a default initializer or extra parameter.
        let close = parameters
            .iter()
            .enumerate()
            .find(|(i, t)| **t == ")" && balanced_type(&parameters[..*i], true))
            .map(|(i, _)| i)
            .ok_or_eyre("non-simple lazy getter parameter")?;
        ensure!(
            return_annotation(&parameters[close + 1..]),
            "invalid lazy arrow return annotation"
        );
    } else {
        ensure!(parameters.is_empty(), "non-simple lazy getter parameter");
    }
    let mut depth = body.len() - 1;
    let end = if let Some(property) = property {
        within(source, arrow, property)?;
        ensure!(returned.end < property.start, "invalid lazy member order");
        let member = syntax_tokens(&source[returned.end as usize..property.start as usize])
            .ok_or_eyre("invalid lazy member tokens")?;
        ensure!(
            member.last() == Some(&".")
                && member[..member.len() - 1].iter().all(|t| *t == ")")
                && member.len() - 1 <= depth,
            "non-direct lazy member"
        );
        depth -= member.len() - 1;
        property.end
    } else {
        returned.end
    };
    let suffix = syntax_tokens(&source[end as usize..arrow.end as usize])
        .ok_or_eyre("invalid lazy return suffix")?;
    ensure!(
        suffix.len() == depth && suffix.iter().all(|t| *t == ")"),
        "non-direct lazy arrow expression"
    );
    Ok(())
}

fn return_annotation(tokens: &[&str]) -> bool {
    tokens.is_empty() || (tokens.first() == Some(&":") && balanced_type(&tokens[1..], false))
}

fn type_parameters(tokens: &[&str]) -> bool {
    tokens.is_empty()
        || (tokens.first() == Some(&"<")
            && tokens.last() == Some(&">")
            && balanced_type(tokens, false))
}

fn balanced_type(mut tokens: &[&str], parameter: bool) -> bool {
    if parameter {
        if tokens.last() == Some(&",") {
            tokens = &tokens[..tokens.len() - 1];
        }
        if tokens.first() == Some(&"?") {
            tokens = &tokens[1..];
        }
        if tokens.is_empty() {
            return true;
        }
        if tokens.first() != Some(&":") {
            return false;
        }
        tokens = &tokens[1..];
    }
    if tokens.is_empty() {
        return false;
    }
    let mut stack = Vec::new();
    for &token in tokens {
        match token {
            "(" => stack.push(")"),
            "[" => stack.push("]"),
            "{" => stack.push("}"),
            "<" => stack.push(">"),
            ")" | "]" | "}" | ">" if stack.pop() != Some(token) => return false,
            "=" if stack.is_empty() => return false,
            "," if stack.is_empty() => return false,
            _ => {}
        }
    }
    stack.is_empty()
}

fn syntax_tokens(mut text: &str) -> Option<Vec<&str>> {
    let mut tokens = Vec::new();
    loop {
        text = trivia(text)?;
        let Some(first) = text.chars().next() else {
            return Some(tokens);
        };
        let end = if matches!(first, '\'' | '"' | '`') {
            quoted_token_len(text)?
        } else if text.starts_with("=>") {
            2
        } else if first.is_alphanumeric() || matches!(first, '_' | '$' | '\\') || !first.is_ascii()
        {
            text.char_indices()
                .skip(1)
                .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '$' | '\\')))
                .map_or(text.len(), |(offset, _)| offset)
        } else {
            first.len_utf8()
        };
        tokens.push(&text[..end]);
        text = &text[end..];
    }
}

fn quoted_token_len(text: &str) -> Option<usize> {
    // Template-literal types may nest templates in substitutions. Track lexical
    // modes iteratively so quoted punctuation never becomes header evidence.
    let quote = text.chars().next()?;
    let mut modes = vec![(quote, 0usize)];
    let mut offset = quote.len_utf8();
    while let Some(&(quote, depth)) = modes.last() {
        let rest = &text[offset..];
        let character = rest.chars().next()?;
        offset += character.len_utf8();
        if depth != 0 {
            if matches!(character, '\'' | '"' | '`') {
                modes.push((character, 0));
            } else if character == '{' {
                modes.last_mut()?.1 += 1;
            } else if character == '}' {
                modes.last_mut()?.1 -= 1;
                if modes.last()?.1 == 0 {
                    modes.pop();
                }
            } else if character == '/' {
                let after = trivia(rest)?;
                if after.len() < rest.len() {
                    offset = text.len() - after.len();
                }
            }
        } else if character == '\\' {
            offset += text[offset..].chars().next()?.len_utf8();
        } else if character == quote {
            modes.pop();
            if modes.is_empty() {
                return Some(offset);
            }
        } else if quote == '`' && character == '$' && text[offset..].starts_with('{') {
            offset += 1;
            modes.push(('\0', 1));
        }
    }
    None
}

fn property_token_matches(source: &str, receiver: Span, property: Span, name: &str) -> bool {
    let Some(mut gap) = trivia(&source[receiver.end as usize..property.start as usize]) else {
        return false;
    };
    while let Some(rest) = gap.strip_prefix(')') {
        let Some(rest) = trivia(rest) else {
            return false;
        };
        gap = rest;
    }
    let Some(gap) = gap.strip_prefix('.').and_then(trivia) else {
        return false;
    };
    if !gap.is_empty() {
        return false;
    }
    // All non-ASCII nontrivia characters are conservatively token-continuing.
    // This includes Unicode ID_Continue and escaped suffixes without a lexer.
    if source[property.end as usize..]
        .chars()
        .next()
        .is_some_and(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '$' | '\\')
                || (!c.is_ascii() && !c.is_whitespace() && c != '\u{feff}')
        })
    {
        return false;
    }
    identifier_matches(
        &source[property.start as usize..property.end as usize],
        name,
    )
}

fn trivia(mut text: &str) -> Option<&str> {
    loop {
        text = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if let Some(comment) = text.strip_prefix("/*") {
            text = &comment[comment.find("*/")? + 2..];
        } else if let Some(comment) = text.strip_prefix("//") {
            text = &comment[comment.find(['\n', '\r', '\u{2028}', '\u{2029}'])?..];
        } else {
            return Some(text);
        }
    }
}

// IdentifierName Unicode escapes are decoded without parsing or allocating on hits.
fn identifier_matches(mut raw: &str, name: &str) -> bool {
    let mut expected = name.chars();
    while !raw.is_empty() {
        let character = if let Some(escaped) = raw.strip_prefix("\\u") {
            let (digits, rest) = if let Some(braced) = escaped.strip_prefix('{') {
                let Some(end) = braced.find('}') else {
                    return false;
                };
                (&braced[..end], &braced[end + 1..])
            } else {
                let Some(digits) = escaped.get(..4) else {
                    return false;
                };
                (digits, &escaped[4..])
            };
            if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_hexdigit()) {
                return false;
            }
            let Some(character) = u32::from_str_radix(digits, 16)
                .ok()
                .and_then(char::from_u32)
            else {
                return false;
            };
            raw = rest;
            character
        } else {
            let character = raw.chars().next().unwrap();
            raw = &raw[character.len_utf8()..];
            character
        };
        if expected.next() != Some(character) {
            return false;
        }
    }
    expected.next().is_none()
}

fn lazy_construction<'a>(
    facts: ModuleFacts<'a>,
    source: &str,
    segment: u32,
    constructor: u32,
    construction: Span,
    arguments: Sequence<'a, SourceSlot<'a>>,
) -> eyre::Result<Invocation<'a>> {
    let recipe = facts
        .emission()
        .segments()
        .get(segment as usize)
        .ok_or_eyre("dangling lazy candidate segment")?;
    span(source, construction)?;
    ensure!(
        construction.start < construction.end && recipe.source().contains(construction),
        "invalid lazy construction span"
    );
    let call = facts
        .invocations()
        .iter()
        .find(|i| i.span() == construction && i.construct())
        .ok_or_eyre("dangling lazy construction")?;
    let reference = facts
        .references()
        .get(constructor as usize)
        .ok_or_eyre("dangling lazy constructor reference")?;
    ensure!(
        reference.read()
            && !reference.write()
            && !reference.type_only()
            && contains(call.callee(), reference.span())
            && wrapped(source, call.callee(), reference.span())
            && matches!(call.shape(), CalleeShape::Identifier(id) if id.binding() == reference.binding() && id.name() == reference.name())
            && call.arguments().len() == arguments.len(),
        "invalid lazy constructor link"
    );
    let ranges: Vec<_> = recipe
        .operations()
        .iter()
        .map(|o| o.kind().span(facts, recipe))
        .collect();
    for (argument, original) in arguments.iter().zip(call.arguments()) {
        within(source, construction, argument.span())?;
        ensure!(
            argument.span() == original.span() && !original.spread()
                || (original.spread()
                    && contains(argument.span(), original.span())
                    && argument.span().end == original.span().end
                    && source[argument.span().start as usize..original.span().start as usize]
                        .starts_with("...")),
            "invalid lazy argument span"
        );
        let expected: Vec<_> = ranges
            .iter()
            .enumerate()
            .filter(|&(i, &range)| {
                contains(argument.span(), range)
                    && !recipe
                        .operations()
                        .get(i)
                        .unwrap()
                        .parent()
                        .is_some_and(|p| contains(argument.span(), ranges[p as usize]))
            })
            .map(|(i, _)| i as u32)
            .collect();
        ensure!(
            argument.operations() == expected,
            "invalid lazy source-slot composition"
        );
    }
    Ok(call)
}

fn execution(module: &ModuleView<'_>, source: &str) -> eyre::Result<()> {
    use ast_segmenter::execution::ContextKind;
    let facts = module.facts();
    let execution = &facts.emission().execution();
    if facts.commonjs() {
        ensure!(
            execution.contexts().is_empty()
                && execution.references().is_empty()
                && execution.invocations().is_empty(),
            "nonopaque CommonJS execution facts"
        );
        return Ok(());
    }
    for (index, context) in execution.contexts().iter().enumerate() {
        span(source, context.span())?;
        ensure!(
            context.span().start < context.span().end
                && module
                    .segments()
                    .iter()
                    .any(|&s| contains(s, context.span())),
            "execution context outside segment"
        );
        let preceding = execution.contexts().slice(..index);
        ensure!(
            context.parent()
                == preceding
                    .iter()
                    .rposition(|c| contains(c.span(), context.span()))
                    .map(|i| i as u32)
                && preceding
                    .last()
                    .is_none_or(|p| p.span().start <= context.span().start)
                && preceding.iter().all(|p| {
                    p.span().end <= context.span().start || contains(p.span(), context.span())
                }),
            "invalid execution context parent/order"
        );
        if let Some(call) = context.immediate() {
            ensure!(
                context.kind() == ContextKind::Callable
                    && facts.invocations().get(call as usize).is_some_and(|call| {
                        !call.construct() && contains(call.callee(), context.span())
                    }),
                "invalid immediate execution context"
            );
        }
    }
    ensure!(
        execution.references()
            == facts
                .references()
                .iter()
                .map(|r| execution.context_at(r.span()))
                .collect::<Vec<_>>()
            && execution.invocations()
                == facts
                    .invocations()
                    .iter()
                    .map(|c| execution.context_at(c.span()))
                    .collect::<Vec<_>>(),
        "invalid execution occurrence coverage"
    );
    for context in execution.contexts() {
        if let Some(call) = context.immediate() {
            ensure!(
                execution.invocations().get(call as usize).unwrap() == context.parent(),
                "immediate execution context escapes caller"
            );
        }
    }
    Ok(())
}

fn placement(facts: ModuleFacts<'_>, recipe: SegmentRecipe<'_>, range: Span) -> eyre::Result<()> {
    let placement = &recipe.placement();
    if facts.commonjs() {
        return Ok(());
    }
    let definitions: Vec<_> = facts
        .bindings()
        .iter()
        .filter(|b| {
            b.top_level() && !b.import() && b.declarations().iter().any(|s| contains(range, s))
        })
        .collect();
    ensure!(
        placement.declarations() == definitions.iter().map(|b| b.id()).collect::<Vec<_>>()
            && placement.forward_references()
                == definitions
                    .iter()
                    .filter(|b| {
                        facts
                            .references()
                            .iter()
                            .any(|r| r.binding() == Some(b.id()) && r.span().start < range.start)
                    })
                    .map(|b| b.id())
                    .collect::<Vec<_>>(),
        "invalid emission declaration/forward links"
    );
    ensure!(
        strict(placement.hoisted().iter().map(|b| b.0))
            && placement
                .hoisted()
                .iter()
                .all(|id| definitions.iter().any(|b| b.id() == id && b.function()))
            && placement.instantiable()
                == (!definitions.is_empty() && placement.declarations() == placement.hoisted()),
        "invalid hoisted availability"
    );
    ensure!(
        placement.default_expression()
            == facts
                .exports()
                .iter()
                .any(|e| matches!(e.value(), ExportValue::Expression(s) if contains(range, s))),
        "invalid default-expression placement"
    );
    ensure!(
        placement.eager_reads()
            == facts
                .references()
                .iter()
                .enumerate()
                .filter(|(i, r)| {
                    r.read()
                        && contains(range, r.span())
                        && facts.emission().execution().is_module_evaluation(
                            facts.emission().execution().references().get(*i).unwrap(),
                        )
                })
                .map(|(i, _)| i as u32)
                .collect::<Vec<_>>()
            && placement.eager_writes()
                == facts
                    .references()
                    .iter()
                    .enumerate()
                    .filter(|(i, r)| {
                        r.write()
                            && contains(range, r.span())
                            && facts.emission().execution().is_module_evaluation(
                                facts.emission().execution().references().get(*i).unwrap(),
                            )
                    })
                    .map(|(i, _)| i as u32)
                    .collect::<Vec<_>>()
            && placement.writes()
                == facts
                    .references()
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.write() && contains(range, r.span()))
                    .map(|(i, _)| i as u32)
                    .collect::<Vec<_>>(),
        "invalid eager-read/write links"
    );
    ensure!(
        strict(recipe.names().named().iter().map(|b| b.0))
            && recipe.names().named().iter().all(|id| facts
                .bindings()
                .iter()
                .any(|b| b.id() == id && b.declarations().iter().any(|s| contains(range, s)))),
        "invalid name-sensitive binding links"
    );
    Ok(())
}

fn namespaces(
    facts: ModuleFacts<'_>,
    recipe: SegmentRecipe<'_>,
    source: &str,
    range: Span,
) -> eyre::Result<()> {
    let expected: Vec<_> = facts
        .references()
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            !facts.commonjs()
                && contains(range, r.span())
                && facts
                    .imports()
                    .iter()
                    .any(|i| Some(i.binding()) == r.binding())
        })
        .map(|(i, _)| i as u32)
        .collect();
    ensure!(
        recipe
            .namespaces()
            .iter()
            .map(|u| u.reference())
            .collect::<Vec<_>>()
            == expected,
        "invalid namespace reference links"
    );
    for usage in recipe.namespaces() {
        if let NamespaceAccess::DirectRead { member, property } = &usage.access() {
            within(source, range, *member)?;
            let reference = facts.references().get(usage.reference() as usize).unwrap();
            ensure!(
                member.start == reference.span().start
                    && reference.span().end < member.end
                    && !property.is_empty(),
                "invalid namespace member range"
            );
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum Key {
    Binding(BindingId, u32),
    Reference(u32),
    Namespace(u32),
    Dependency(u32),
    Export(u32),
    Hint(u32),
    Invocation(u32),
}

fn operations(
    facts: ModuleFacts<'_>,
    recipe: SegmentRecipe<'_>,
    source: &str,
    range: Span,
) -> eyre::Result<()> {
    let bindings: HashMap<_, _> = facts.bindings().iter().map(|b| (b.id(), b)).collect();
    let mut keys = HashSet::new();
    let mut ranges = Vec::with_capacity(recipe.operations().len());
    let mut connector = false;
    for operation in recipe.operations() {
        let key = match &operation.kind() {
            OperationKind::Binding {
                binding,
                declaration,
                shorthand,
            } => {
                let binding = bindings
                    .get(&binding)
                    .ok_or_eyre("dangling emission binding")?;
                binding
                    .declarations()
                    .get(*declaration as usize)
                    .ok_or_eyre("dangling emission declaration")?;
                ensure!(!facts.commonjs(), "lexical CJS rewrite");
                shorthand_check(&binding.name(), *shorthand)?;
                Key::Binding(binding.id(), *declaration)
            }
            OperationKind::Reference {
                reference,
                shorthand,
            } => {
                let reference_record = facts
                    .references()
                    .get(*reference as usize)
                    .ok_or_eyre("dangling emission reference")?;
                ensure!(!facts.commonjs(), "lexical CJS rewrite");
                shorthand_check(&reference_record.name(), *shorthand)?;
                Key::Reference(*reference)
            }
            OperationKind::Namespace { usage } => {
                let usage_record = recipe
                    .namespaces()
                    .get(*usage as usize)
                    .ok_or_eyre("dangling emission namespace")?;
                ensure!(
                    matches!(usage_record.access(), NamespaceAccess::DirectRead { .. }),
                    "native namespace has direct-read operation"
                );
                Key::Namespace(*usage)
            }
            OperationKind::Dependency { dependency } => {
                let dependency = facts
                    .dependencies()
                    .iter()
                    .find(|d| d.id() == *dependency)
                    .ok_or_eyre("dangling emission dependency")?;
                ensure!(
                    matches!(
                        dependency.kind(),
                        DependencyKind::Import
                            | DependencyKind::DynamicImport
                            | DependencyKind::Require
                    ),
                    "invalid emission dependency kind"
                );
                if dependency.kind() == DependencyKind::Import {
                    ensure!(dependency.span() == range, "nonstatement import operation");
                    connector = true;
                }
                Key::Dependency(dependency.id().0)
            }
            OperationKind::Export {
                span: wrapper,
                form,
            } => {
                ensure!(
                    !facts.commonjs()
                        && (*wrapper == range
                            || (recipe.source().spans().len() > 1
                                && *wrapper == recipe.source().spans().get(0).unwrap()
                                && matches!(form, ExportForm::Declaration { .. }))),
                    "invalid export wrapper range"
                );
                ensure!(
                    source
                        .get(wrapper.start as usize..wrapper.end as usize)
                        .is_some_and(|s| s.starts_with("export")),
                    "invalid export wrapper text"
                );
                match form {
                    ExportForm::Remove => {
                        ensure!(
                            recipe.placement().declarations().is_empty()
                                && !facts.exports().iter().any(|e| e.span() == *wrapper
                                    && matches!(e.value(), ExportValue::Expression(_))),
                            "removed default value"
                        );
                        connector = true;
                    }
                    ExportForm::Declaration { declaration } => {
                        within(source, *wrapper, *declaration)?;
                        ensure!(
                            wrapper.start < declaration.start && declaration.end == wrapper.end,
                            "invalid export declaration wrapper"
                        );
                        ensure!(
                            !facts.exports().iter().any(|e| contains(*wrapper, e.span())
                                && !matches!(e.value(), ExportValue::Binding(_))),
                            "invalid export declaration facts"
                        );
                    }
                    ExportForm::Default { value, .. } => {
                        ensure!(
                            facts.exports().iter().any(|e| e.span() == *wrapper
                                && e.name().as_deref() == Some("default")
                                && e.value() == ExportValue::Expression(value.span())),
                            "invalid default value link"
                        );
                    }
                }
                Key::Export(wrapper.start)
            }
            OperationKind::SourceHint { span: hint } => {
                span(source, *hint)?;
                ensure!(
                    source_hint(&source[hint.start as usize..hint.end as usize]),
                    "invalid source hint text"
                );
                Key::Hint(hint.start)
            }
            OperationKind::Invocation { invocation, .. } => {
                ensure!(
                    !facts.commonjs() && facts.invocations().get(*invocation as usize).is_some(),
                    "dangling emission invocation"
                );
                Key::Invocation(*invocation)
            }
        };
        ensure!(keys.insert(key), "duplicate emission operation");
        let operation_range = operation.kind().span(facts, recipe);
        span(source, operation_range)?;
        ensure!(
            operation_range.start < operation_range.end
                && recipe.source().contains(operation_range),
            "invalid emission containment"
        );
        ranges.push(operation_range);
    }
    if !facts.commonjs() {
        connector |= source
            .get(range.start as usize..range.end as usize)
            .is_some_and(|s| s.trim() == ";");
        ensure!(
            recipe.connector_only() == connector,
            "invalid connector classification"
        );
    }
    let mut stack: Vec<usize> = Vec::new();
    let mut previous = None;
    for (i, (&operation_range, operation)) in ranges.iter().zip(recipe.operations()).enumerate() {
        let key = (
            operation_range.start,
            std::cmp::Reverse(operation_range.end),
        );
        ensure!(
            previous.is_none_or(|p| p <= key),
            "unordered emission operations"
        );
        previous = Some(key);
        while stack
            .last()
            .is_some_and(|&p| !contains(ranges[p], operation_range))
        {
            let p = stack.pop().expect("nonempty stack");
            ensure!(
                ranges[p].end <= operation_range.start,
                "crossing emission operations"
            );
        }
        ensure!(
            operation.parent() == stack.last().map(|&p| p as u32),
            "invalid emission parent composition"
        );
        stack.push(i);
    }
    for (owner, operation) in recipe.operations().iter().enumerate() {
        match &operation.kind() {
            OperationKind::Export {
                span: outer,
                form: ExportForm::Default { value, .. },
            } => source_slot(source, *outer, *value, owner, &ranges, recipe.operations())?,
            OperationKind::Invocation {
                invocation,
                lowering,
            } => pure(
                facts,
                *invocation,
                *lowering,
                owner,
                recipe,
                source,
                &ranges,
            )?,
            _ => {}
        }
    }
    match &recipe.deferred() {
        DeferredForm::Unsupported => {}
        DeferredForm::Function => ensure!(
            recipe.placement().instantiable() && !recipe.placement().default_expression(),
            "invalid deferred function"
        ),
        DeferredForm::Effect => ensure!(
            recipe.placement().declarations().is_empty()
                && !recipe.placement().default_expression(),
            "invalid deferred effect"
        ),
        DeferredForm::Variable {
            binding,
            initializer,
        } => {
            let binding = bindings
                .get(binding)
                .ok_or_eyre("dangling deferred binding")?;
            ensure!(
                !facts.commonjs()
                    && !binding.import()
                    && !binding.function()
                    && recipe.placement().declarations() == [binding.id()]
                    && !recipe.placement().default_expression(),
                "invalid deferred variable"
            );
            if let Some(slot) = initializer {
                within(source, range, slot.span())?;
                ensure!(
                    binding
                        .declarations()
                        .iter()
                        .any(|s| contains(range, s) && s.end < slot.span().start),
                    "invalid deferred initializer range"
                );
                let expected: Vec<_> = ranges
                    .iter()
                    .enumerate()
                    .filter(|&(i, &range)| {
                        contains(slot.span(), range)
                            && !recipe
                                .operations()
                                .get(i)
                                .unwrap()
                                .parent()
                                .is_some_and(|p| contains(slot.span(), ranges[p as usize]))
                    })
                    .map(|(i, _)| i as u32)
                    .collect();
                ensure!(
                    slot.operations() == expected,
                    "invalid deferred source-slot composition"
                );
            }
        }
    }
    for (binding, info) in &bindings {
        for (i, declaration) in info.declarations().iter().enumerate() {
            if !facts.commonjs() && contains(range, declaration) {
                ensure!(
                    keys.contains(&Key::Binding(*binding, i as u32)),
                    "missing emission declaration"
                );
            }
        }
    }
    for (i, reference) in facts.references().iter().enumerate() {
        if !facts.commonjs() && contains(range, reference.span()) {
            ensure!(
                keys.contains(&Key::Reference(i as u32)),
                "missing emission reference"
            );
        }
    }
    for (i, usage) in recipe.namespaces().iter().enumerate() {
        if matches!(usage.access(), NamespaceAccess::DirectRead { .. }) {
            ensure!(
                keys.contains(&Key::Namespace(i as u32)),
                "missing namespace operation"
            );
        }
    }
    for dependency in facts.dependencies() {
        if contains(range, dependency.span())
            && matches!(
                dependency.kind(),
                DependencyKind::Import | DependencyKind::DynamicImport | DependencyKind::Require
            )
        {
            ensure!(
                keys.contains(&Key::Dependency(dependency.id().0)),
                "missing emission dependency"
            );
        }
    }
    for (i, invocation) in facts.invocations().iter().enumerate() {
        if contains(range, invocation.span()) {
            ensure!(
                keys.contains(&Key::Invocation(i as u32)),
                "missing emission invocation"
            );
        }
    }
    for export in facts.exports() {
        if contains(range, export.span()) {
            ensure!(
                keys.contains(&Key::Export(recipe.source().spans().get(0).unwrap().start)),
                "missing export wrapper"
            );
        }
    }
    Ok(())
}

fn shorthand_check(name: &str, shorthand: Shorthand) -> eyre::Result<()> {
    if shorthand == Shorthand::Proto {
        ensure!(name == "__proto__", "invalid literal proto shorthand");
    }
    Ok(())
}

fn source_hint(text: &str) -> bool {
    text.strip_prefix("//")
        .or_else(|| text.strip_prefix("/*").and_then(|t| t.strip_suffix("*/")))
        .and_then(|b| b.strip_prefix('#').or_else(|| b.strip_prefix('@')))
        .is_some_and(|b| {
            b.trim_start().starts_with("sourceMappingURL=")
                || b.trim_start().starts_with("sourceURL=")
        })
}

fn source_slot<'a>(
    source: &str,
    outer: Span,
    slot: SourceSlot<'_>,
    owner: usize,
    ranges: &[Span],
    operations: Sequence<'a, Operation<'a>>,
) -> eyre::Result<()> {
    within(source, outer, slot.span())?;
    ensure!(
        slot.span() != ranges[owner] && slot.operations().iter().all(|i| i as usize > owner),
        "recursive source-slot composition"
    );
    let expected: Vec<_> = ranges
        .iter()
        .enumerate()
        .filter(|&(i, &range)| {
            contains(slot.span(), range)
                && !operations
                    .get(i)
                    .unwrap()
                    .parent()
                    .is_some_and(|p| contains(slot.span(), ranges[p as usize]))
        })
        .map(|(i, _)| i as u32)
        .collect();
    ensure!(
        slot.operations() == expected,
        "invalid source-slot composition"
    );
    Ok(())
}

fn pure(
    facts: ModuleFacts<'_>,
    invocation: u32,
    lowering: PureLowering<'_>,
    owner: usize,
    recipe: SegmentRecipe<'_>,
    source: &str,
    ranges: &[Span],
) -> eyre::Result<()> {
    let call = facts.invocations().get(invocation as usize).unwrap();
    ensure!(
        lowering.callee().span() == call.callee()
            && lowering.arguments().len() == call.arguments().len(),
        "invalid PURE input links"
    );
    source_slot(
        source,
        call.span(),
        lowering.callee(),
        owner,
        ranges,
        recipe.operations(),
    )?;
    for (slot, argument) in lowering.arguments().iter().zip(call.arguments()) {
        ensure!(slot.span() == argument.span(), "invalid PURE argument link");
        source_slot(
            source,
            call.span(),
            slot,
            owner,
            ranges,
            recipe.operations(),
        )?;
    }
    if let Some(start) = lowering.guard_start() {
        ensure!(
            call.callee().start < start
                && start <= call.callee().end
                && source.is_char_boundary(start as usize),
            "invalid PURE guard boundary"
        );
    }
    ensure!(
        !lowering.crosses_await_guard() || lowering.guard_start().is_some(),
        "await veto without PURE guard"
    );
    if call.deferred().is_none() {
        ensure!(
            lowering.crosses_await_guard()
                == lowering.guard_start().is_some_and(|start| facts
                    .top_level_await()
                    .iter()
                    .any(|s| start <= s.start && s.end <= call.span().end)),
            "invalid module await guard veto"
        );
    }
    if let Some(chain) = &lowering.chain() {
        source_slot(
            source,
            call.callee(),
            chain.base(),
            owner,
            ranges,
            recipe.operations(),
        )?;
        ensure!(
            chain.base().span().start == call.callee().start,
            "invalid PURE chain base"
        );
        let optional = |step: ChainStep<'_>| {
            matches!(
                step,
                ChainStep::Static { optional: true, .. }
                    | ChainStep::Computed { optional: true, .. }
            )
        };
        if chain.steps().iter().any(optional) {
            ensure!(
                lowering.guard_start().is_some_and(
                    |start| chain.base().span().end <= start && start < call.callee().end
                ),
                "invalid member guard boundary"
            );
            if chain.steps().first().is_some_and(optional) {
                ensure!(
                    lowering.guard_start() == Some(chain.base().span().end),
                    "invalid first member guard"
                );
            }
        } else {
            ensure!(
                lowering.guard_start() == call.optional_call().then_some(call.callee().end),
                "invalid unguarded chain boundary"
            );
        }
        let mut end = chain.base().span().end;
        for step in chain.steps() {
            let slot = match step {
                ChainStep::Static { property, .. } => {
                    ensure!(!property.is_empty(), "empty PURE member property");
                    None
                }
                ChainStep::Computed { property, .. } => Some(property),
                ChainStep::Call { suffix } => Some(suffix),
            };
            if let Some(slot) = slot {
                source_slot(
                    source,
                    call.callee(),
                    slot,
                    owner,
                    ranges,
                    recipe.operations(),
                )?;
                ensure!(end <= slot.span().start, "unordered PURE chain slots");
                end = slot.span().end;
            }
        }
        if chain.steps().is_empty() {
            ensure!(
                chain.base().span() == call.callee(),
                "incomplete opaque PURE callee"
            );
        }
    }
    Ok(())
}
