use std::path::Path;

use eyre::{ensure, Result};

use crate::{hash128, type_only_wire as wire, SharedBytes, CHECKSUM_BYTES};

const SCHEMA: u32 = 1;
const EXTRACTOR: u32 = 2;

/// Expanded enum strings, independent of the size of the original source.
pub const TYPE_ONLY_STRING_VALUE_BYTES: usize = 64 * 1024;
pub const TYPE_ONLY_STRING_TOTAL_BYTES: usize = 4 * 1024 * 1024;

#[cfg(test)]
#[path = "type_only_tests.rs"]
mod tests;

/// Immutable, checked original-TypeScript evidence, never solved exports.
#[derive(Clone, Debug)]
pub struct WireTypeOnlyFacts {
    bytes: SharedBytes,
}

impl WireTypeOnlyFacts {
    pub fn encode_in(
        directory: &Path,
        source: &str,
        config: [u8; 16],
        facts: wire::FactsT,
    ) -> Result<Self> {
        let mut expanded = 0usize;
        for value in facts
            .const_enums
            .iter()
            .flat_map(|enumeration| &enumeration.members)
            .filter_map(|member| member.string.as_deref())
        {
            check_string_bytes(&mut expanded, value)?;
        }
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let facts = facts.pack(&mut builder);
        builder.finish(facts, None);
        let records = builder.finished_data();
        let mut outer = flatbuffers::FlatBufferBuilder::new();
        let facts = outer.create_vector(records);
        let root = wire::TypeOnlyFacts::create(
            &mut outer,
            &wire::TypeOnlyFactsArgs {
                schema: SCHEMA,
                extractor: EXTRACTOR,
                source_hash: Some(&wire::Hash128::new(&hash128(source.as_bytes()))),
                config_hash: Some(&wire::Hash128::new(&config)),
                records_hash: Some(&wire::Hash128::new(&hash128(records))),
                facts: Some(facts),
            },
        );
        wire::finish_type_only_facts_buffer(&mut outer, root);
        let mut bytes = outer.finished_data().to_vec();
        bytes.extend_from_slice(&hash128(outer.finished_data()));
        Self::from_backing(
            SharedBytes::snapshot_in(directory, bytes.as_slice())?,
            source,
            config,
        )
    }

    pub fn from_backing(bytes: SharedBytes, source: &str, config: [u8; 16]) -> Result<Self> {
        let end = bytes
            .len()
            .checked_sub(CHECKSUM_BYTES)
            .ok_or_else(|| eyre::eyre!("truncated TypeOnlyFacts"))?;
        let (payload, checksum) = bytes.split_at(end);
        ensure!(payload.len() >= 8, "truncated TypeOnlyFacts root");
        ensure!(
            hash128(payload).as_slice() == checksum,
            "TypeOnlyFacts checksum mismatch"
        );
        ensure!(
            wire::type_only_facts_buffer_has_identifier(payload),
            "TypeOnlyFacts identifier mismatch"
        );
        let root = flatbuffers::root_with_opts::<wire::TypeOnlyFacts<'_>>(
            &crate::options(payload.len()),
            payload,
        )?;
        ensure!(
            root.schema() == SCHEMA && root.extractor() == EXTRACTOR,
            "TypeOnlyFacts version mismatch"
        );
        ensure!(
            root.config_hash().bytes().iter().eq(config)
                && root
                    .source_hash()
                    .bytes()
                    .iter()
                    .eq(hash128(source.as_bytes())),
            "TypeOnlyFacts source/config mismatch"
        );
        let records = root.facts().bytes();
        ensure!(
            root.records_hash().bytes().iter().eq(hash128(records)),
            "TypeOnlyFacts extraction seal mismatch"
        );
        let facts = flatbuffers::root_with_opts::<wire::Facts<'_>>(
            &crate::options(records.len()),
            records,
        )?;
        validate(facts, source)?;
        Ok(Self { bytes })
    }

    pub fn facts(&self) -> wire::Facts<'_> {
        // SAFETY: both immutable table graphs passed structural and semantic validation.
        unsafe {
            let root = flatbuffers::root_unchecked::<wire::TypeOnlyFacts<'_>>(
                &self.bytes[..self.bytes.len() - CHECKSUM_BYTES],
            );
            flatbuffers::root_unchecked::<wire::Facts<'_>>(root.facts().bytes())
        }
    }

    pub fn backing(&self) -> &SharedBytes {
        &self.bytes
    }
}

fn validate(facts: wire::Facts<'_>, source: &str) -> Result<()> {
    let span = |s: &wire::Span| -> Result<()> {
        ensure!(
            s.start() <= s.end() && source.get(s.start() as usize..s.end() as usize).is_some(),
            "invalid TypeOnlyFacts UTF-8 span {}..{}",
            s.start(),
            s.end()
        );
        Ok(())
    };
    let contains = |outer: &wire::Span, inner: &wire::Span| -> Result<()> {
        span(outer)?;
        span(inner)?;
        ensure!(
            outer.start() <= inner.start() && inner.end() <= outer.end(),
            "TypeOnlyFacts child outside owner"
        );
        Ok(())
    };
    let name = |n: wire::Name<'_>| -> Result<()> {
        span(n.span())?;
        ensure!(
            n.span().start() < n.span().end(),
            "invalid TypeOnlyFacts name"
        );
        Ok(())
    };
    let presence = |p| -> Result<()> {
        ensure!(
            matches!(
                p,
                wire::Presence::OnlyType
                    | wire::Presence::RuntimeValue
                    | wire::Presence::ConstEnum
                    | wire::Presence::Unresolved
            ),
            "unknown TypeOnlyFacts presence"
        );
        Ok(())
    };
    let mut locals = std::collections::HashMap::new();
    let mut enum_locals = std::collections::HashSet::new();
    let diagnostic_spans: std::collections::HashSet<_> = facts
        .diagnostics()
        .iter()
        .filter(|d| {
            matches!(
                d.kind(),
                wire::DiagnosticKind::UnsupportedConstant
                    | wire::DiagnosticKind::UnresolvedConstant
                    | wire::DiagnosticKind::CyclicConstant
            )
        })
        .map(|d| (d.span().start(), d.span().end()))
        .collect();
    let mut last = 0;
    for local in facts.locals() {
        name(local.name())?;
        ensure!(
            !local.name().value().is_empty() && !local.name().value().contains('\0'),
            "invalid TypeOnlyFacts local name"
        );
        contains(local.span(), local.name().span())?;
        presence(local.presence())?;
        ensure!(
            last <= local.span().start(),
            "unordered TypeOnlyFacts locals"
        );
        ensure!(
            local.presence() != wire::Presence::OnlyType || local.has_type(),
            "type declaration without type evidence"
        );
        locals
            .entry(local.name().value())
            .and_modify(|(p, t)| {
                *p = merge(*p, local.presence());
                *t |= local.has_type();
            })
            .or_insert((local.presence(), local.has_type()));
        if local.presence() == wire::Presence::ConstEnum {
            enum_locals.insert((
                local.name().value(),
                local.name().span().start(),
                local.name().span().end(),
            ));
        }
        last = local.span().start();
    }
    last = 0;
    for export in facts.exports() {
        name(export.name())?;
        contains(export.span(), export.name().span())?;
        presence(export.presence())?;
        ensure!(
            last <= export.span().start(),
            "unordered TypeOnlyFacts exports"
        );
        last = export.span().start();
        for n in [export.local(), export.request(), export.imported()]
            .into_iter()
            .flatten()
        {
            name(n)?;
            contains(export.span(), n.span())?;
        }
        ensure!(
            export.request().is_some() == export.imported().is_some(),
            "incomplete TypeOnlyFacts export route"
        );
        ensure!(
            !export.star()
                || (export.request().is_some() && export.imported().unwrap().value() == "*"),
            "invalid TypeOnlyFacts star route"
        );
        ensure!(
            !export.explicit_type() || export.presence() == wire::Presence::OnlyType,
            "explicit type export has executable presence"
        );
        if !export.explicit_type() {
            if let Some((presence, has_type)) = export.local().and_then(|n| locals.get(n.value())) {
                ensure!(
                    export.presence() == *presence && (!*has_type || export.has_type()),
                    "TypeOnlyFacts export disagrees with local declarations"
                );
            }
        }
    }
    last = 0;
    for occurrence in facts.occurrences() {
        span(occurrence.span())?;
        ensure!(
            last <= occurrence.span().start(),
            "unordered TypeOnlyFacts occurrences"
        );
        last = occurrence.span().start();
        ensure!(
            matches!(
                occurrence.kind(),
                wire::OccurrenceKind::Import
                    | wire::OccurrenceKind::Reexport
                    | wire::OccurrenceKind::LocalExport
                    | wire::OccurrenceKind::ImportEquals
                    | wire::OccurrenceKind::ExportAssignment
                    | wire::OccurrenceKind::TypeImport
            ),
            "unknown TypeOnlyFacts occurrence kind"
        );
        for n in [
            occurrence.request(),
            occurrence.imported(),
            occurrence.local(),
            occurrence.exported(),
        ]
        .into_iter()
        .flatten()
        {
            name(n)?;
            contains(occurrence.span(), n.span())?;
        }
        ensure!(
            !matches!(
                occurrence.kind(),
                wire::OccurrenceKind::Import
                    | wire::OccurrenceKind::Reexport
                    | wire::OccurrenceKind::ImportEquals
                    | wire::OccurrenceKind::TypeImport
            ) || occurrence.request().is_some(),
            "missing TypeOnlyFacts request"
        );
        ensure!(
            occurrence.kind() != wire::OccurrenceKind::TypeImport || occurrence.explicit_type(),
            "TypeOnlyFacts import type lost explicit syntax"
        );
        let mut previous = 0;
        ensure!(
            occurrence.local().is_some() || occurrence.uses().is_empty(),
            "TypeOnlyFacts uses lack a local identity"
        );
        for usage in occurrence.uses() {
            span(usage.span())?;
            ensure!(
                previous <= usage.span().start()
                    && (usage.type_use() || usage.value_use() || usage.value_as_type()),
                "unordered or empty TypeOnlyFacts use"
            );
            previous = usage.span().end();
        }
    }
    last = 0;
    let mut expanded_string_bytes = 0usize;
    for enumeration in facts.const_enums() {
        name(enumeration.name())?;
        contains(enumeration.span(), enumeration.name().span())?;
        ensure!(
            last <= enumeration.span().start(),
            "unordered TypeOnlyFacts enums"
        );
        ensure!(
            enum_locals.contains(&(
                enumeration.name().value(),
                enumeration.name().span().start(),
                enumeration.name().span().end()
            )),
            "TypeOnlyFacts enum lacks local declaration"
        );
        last = enumeration.span().start();
        let mut previous = enumeration.span().start();
        for member in enumeration.members() {
            name(member.name())?;
            contains(enumeration.span(), member.span())?;
            contains(member.span(), member.name().span())?;
            ensure!(
                previous <= member.span().start(),
                "unordered TypeOnlyFacts members"
            );
            previous = member.span().end();
            if let Some(init) = member.initializer() {
                contains(member.span(), init)?;
            }
            ensure!(
                matches!(
                    member.kind(),
                    wire::ConstantKind::Unresolved
                        | wire::ConstantKind::Number
                        | wire::ConstantKind::String
                ),
                "unknown TypeOnlyFacts constant kind"
            );
            ensure!(
                (member.kind() == wire::ConstantKind::String) == member.string().is_some()
                    && (member.kind() == wire::ConstantKind::Number) == member.number().is_some()
                    && member.number().is_none_or(f64::is_finite),
                "invalid TypeOnlyFacts constant payload"
            );
            if let Some(value) = member.string() {
                check_string_bytes(&mut expanded_string_bytes, value)?;
            }
            if member.kind() == wire::ConstantKind::Unresolved {
                ensure!(
                    diagnostic_spans.contains(&(member.span().start(), member.span().end())),
                    "unresolved TypeOnlyFacts member without diagnostic"
                );
            }
        }
    }
    last = 0;
    for diagnostic in facts.diagnostics() {
        span(diagnostic.span())?;
        ensure!(
            last <= diagnostic.span().start() && !diagnostic.message().is_empty(),
            "invalid TypeOnlyFacts diagnostic"
        );
        last = diagnostic.span().start();
        ensure!(
            matches!(
                diagnostic.kind(),
                wire::DiagnosticKind::UnsupportedConstant
                    | wire::DiagnosticKind::UnresolvedConstant
                    | wire::DiagnosticKind::UnsupportedDeclaration
                    | wire::DiagnosticKind::CyclicConstant
            ),
            "unknown TypeOnlyFacts diagnostic kind"
        );
    }
    Ok(())
}

fn check_string_bytes(total: &mut usize, value: &str) -> Result<()> {
    ensure!(
        value.len() <= TYPE_ONLY_STRING_VALUE_BYTES,
        "TypeOnlyFacts enum string exceeds per-value byte budget"
    );
    *total = total
        .checked_add(value.len())
        .ok_or_else(|| eyre::eyre!("TypeOnlyFacts enum string byte overflow"))?;
    ensure!(
        *total <= TYPE_ONLY_STRING_TOTAL_BYTES,
        "TypeOnlyFacts enum strings exceed aggregate byte budget"
    );
    Ok(())
}

fn merge(a: wire::Presence, b: wire::Presence) -> wire::Presence {
    if a == wire::Presence::RuntimeValue || b == wire::Presence::RuntimeValue {
        wire::Presence::RuntimeValue
    } else if a == wire::Presence::ConstEnum || b == wire::Presence::ConstEnum {
        wire::Presence::ConstEnum
    } else if a == wire::Presence::Unresolved || b == wire::Presence::Unresolved {
        wire::Presence::Unresolved
    } else {
        wire::Presence::OnlyType
    }
}
