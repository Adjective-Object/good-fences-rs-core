use std::collections::HashSet;

use ast_segmenter::{owned::CommonJsGlobal, views::ModuleFacts};
use eyre::ensure;
use oxc_span::Span;

use crate::validate::{contains, span};

pub(super) fn validate(facts: ModuleFacts<'_>, source: &str) -> eyre::Result<()> {
    let surface = facts.commonjs_surface();
    ensure!(
        facts.commonjs() || (surface.accesses().is_empty() && surface.unknown().is_empty()),
        "CommonJS surface on ESM"
    );
    let mut previous = None;
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for unknown in surface.unknown() {
        span(source, unknown)?;
        ensure!(unknown.start < unknown.end, "empty CommonJS unknown site");
        let key = (unknown.start, unknown.end);
        ensure!(
            previous.is_none_or(|p| p < key),
            "unordered CommonJS unknown sites"
        );
        previous = Some(key);
        if let Some(last) = ranges.last_mut().filter(|last| unknown.start <= last.1) {
            last.1 = last.1.max(unknown.end);
        } else {
            ranges.push(key);
        }
    }
    let unknown = |site: Span| {
        let index = ranges.partition_point(|range| range.0 <= site.start);
        index != 0 && ranges[index - 1].1 >= site.end
    };
    let mut covered = HashSet::new();
    let mut previous = None;
    for access in surface.accesses() {
        span(source, access.span())?;
        let reference = facts.references().get(access.reference() as usize);
        ensure!(
            previous.is_none_or(|p| p < access.reference()) && covered.insert(access.reference()),
            "duplicate/unordered CommonJS access"
        );
        previous = Some(access.reference());
        let reference =
            reference.ok_or_else(|| eyre::eyre!("dangling CommonJS access reference"))?;
        ensure!(
            matches!(
                reference.commonjs(),
                Some(CommonJsGlobal::Module | CommonJsGlobal::Exports)
            ) && access.span().start <= reference.span().start
                && reference.span().end < access.span().end
                && contains(access.span(), reference.span()),
            "invalid CommonJS access owner"
        );
        ensure!(
            if access.object_assignment() {
                reference.commonjs() == Some(CommonJsGlobal::Module)
            } else {
                access.names().len() == 1
            },
            "invalid CommonJS access kind"
        );
        let mut previous = reference.span().end;
        for name in access.names() {
            span(source, name.span())?;
            ensure!(
                name.span().start >= previous
                    && name.span().start < name.span().end
                    && contains(access.span(), name.span()),
                "invalid CommonJS name span"
            );
            previous = name.span().end;
            let raw = &source[name.span().start as usize..name.span().end as usize];
            ensure!(
                crate::validate_emission::identifier_matches(raw, name.name())
                    || string_matches(raw, name.name()),
                "invalid CommonJS export name"
            );
        }
    }
    if facts.commonjs() {
        ensure!(
            !facts.emission().uses_this() || !surface.unknown().is_empty(),
            "uncovered CommonJS this"
        );
        let callees: HashSet<_> = facts
            .invocations()
            .iter()
            .filter(|i| !i.construct())
            .map(|i| (i.callee().start, i.callee().end))
            .collect();
        for access in surface.accesses() {
            if callees.contains(&(access.span().start, access.span().end)) {
                ensure!(unknown(access.span()), "uncovered CommonJS receiver call");
            }
        }
        for (index, reference) in facts.references().iter().enumerate() {
            if reference.commonjs().is_some()
                || (reference.binding().is_none() && reference.name() == "arguments")
            {
                ensure!(
                    covered.contains(&(index as u32)) || unknown(reference.span()),
                    "uncovered CommonJS reference"
                );
            }
        }
        for dependency in facts.dependencies() {
            ensure!(unknown(dependency.span()), "uncovered CommonJS dependency");
        }
        for violation in facts.policy_violations() {
            ensure!(unknown(violation.span()), "uncovered CommonJS policy");
        }
        for binding in facts
            .bindings()
            .iter()
            .filter(|b| b.top_level() && b.name() == "module")
        {
            for site in binding.declarations() {
                ensure!(unknown(site), "uncovered CommonJS module shadow");
            }
        }
    }
    Ok(())
}

// Decode only a single string token, without parsing source or allocating on hits.
fn string_matches(raw: &str, name: &str) -> bool {
    let Some(quote @ ('\'' | '"')) = raw.chars().next() else {
        return false;
    };
    let Some(mut raw) = raw.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) else {
        return false;
    };
    let mut expected = name.chars();
    while let Some(mut character) = raw.chars().next() {
        raw = &raw[character.len_utf8()..];
        if character == '\\' {
            let Some(escaped) = raw.chars().next() else {
                return false;
            };
            raw = &raw[escaped.len_utf8()..];
            character = match escaped {
                '\n' | '\u{2028}' | '\u{2029}' => continue,
                '\r' => {
                    raw = raw.strip_prefix('\n').unwrap_or(raw);
                    continue;
                }
                'x' | 'u' => {
                    let (digits, rest) = if escaped == 'u' && raw.starts_with('{') {
                        let Some(end) = raw.find('}') else {
                            return false;
                        };
                        (&raw[1..end], &raw[end + 1..])
                    } else {
                        let count = if escaped == 'x' { 2 } else { 4 };
                        let Some(digits) = raw.get(..count) else {
                            return false;
                        };
                        (digits, &raw[count..])
                    };
                    raw = rest;
                    let Ok(mut code) = u32::from_str_radix(digits, 16) else {
                        return false;
                    };
                    if (0xd800..=0xdbff).contains(&code) {
                        let Some(tail) = raw.strip_prefix("\\u") else {
                            return false;
                        };
                        let Some(digits) = tail.get(..4) else {
                            return false;
                        };
                        let Ok(low) = u32::from_str_radix(digits, 16) else {
                            return false;
                        };
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return false;
                        }
                        code = 0x10000 + ((code - 0xd800) << 10) + low - 0xdc00;
                        raw = &tail[4..];
                    }
                    let Some(decoded) = char::from_u32(code) else {
                        return false;
                    };
                    decoded
                }
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\u{8}',
                'f' => '\u{c}',
                'v' => '\u{b}',
                first @ '0'..='7' => {
                    let mut code = first as u32 - '0' as u32;
                    for _ in 1..if first <= '3' { 3 } else { 2 } {
                        let Some(next @ '0'..='7') = raw.chars().next() else {
                            break;
                        };
                        raw = &raw[1..];
                        code = code * 8 + next as u32 - '0' as u32;
                    }
                    char::from_u32(code).unwrap()
                }
                c => c,
            };
        } else if character == quote || matches!(character, '\n' | '\r') {
            return false;
        }
        if expected.next() != Some(character) {
            return false;
        }
    }
    expected.next().is_none()
}
