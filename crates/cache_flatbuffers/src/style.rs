use eyre::{ensure, Result};

use crate::{hash128, style_wire as wire, SharedBytes, CHECKSUM_BYTES};

const SCHEMA: u32 = 1;
const EXTRACTOR: u32 = 1;

#[cfg(test)]
#[path = "style_tests.rs"]
mod tests;

/// Checked source-local stylesheet facts. Object APIs are cold construction only.
#[derive(Clone, Debug)]
pub struct WireStyle {
    bytes: SharedBytes,
}

impl WireStyle {
    pub fn encode(
        input: &[u8],
        source: &str,
        config: [u8; 16],
        facts: wire::FactsT,
    ) -> Result<Self> {
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let facts = facts.pack(&mut builder);
        builder.finish(facts, None);
        let records = builder.finished_data();
        let mut outer = flatbuffers::FlatBufferBuilder::new();
        let facts = outer.create_vector(records);
        let root = wire::Style::create(
            &mut outer,
            &wire::StyleArgs {
                schema: SCHEMA,
                extractor: EXTRACTOR,
                source_hash: Some(&wire::Hash128::new(&hash128(input))),
                prepared_hash: Some(&wire::Hash128::new(&hash128(source.as_bytes()))),
                config_hash: Some(&wire::Hash128::new(&config)),
                records_hash: Some(&wire::Hash128::new(&hash128(records))),
                facts: Some(facts),
            },
        );
        wire::finish_style_buffer(&mut outer, root);
        let mut bytes = outer.finished_data().to_vec();
        bytes.extend_from_slice(&hash128(outer.finished_data()));
        Self::from_backing(
            SharedBytes::snapshot(bytes.as_slice())?,
            input,
            source,
            config,
        )
    }

    pub fn from_backing(
        bytes: SharedBytes,
        input: &[u8],
        source: &str,
        config: [u8; 16],
    ) -> Result<Self> {
        let end = bytes
            .len()
            .checked_sub(CHECKSUM_BYTES)
            .ok_or_else(|| eyre::eyre!("truncated stylesheet facts"))?;
        let (payload, checksum) = bytes.split_at(end);
        ensure!(payload.len() >= 8, "truncated stylesheet root");
        ensure!(
            hash128(payload).as_slice() == checksum,
            "style checksum mismatch"
        );
        ensure!(
            wire::style_buffer_has_identifier(payload),
            "style identifier mismatch"
        );
        let root = flatbuffers::root_with_opts::<wire::Style<'_>>(
            &crate::options(payload.len()),
            payload,
        )?;
        ensure!(
            root.schema() == SCHEMA && root.extractor() == EXTRACTOR,
            "style version mismatch"
        );
        ensure!(
            root.source_hash().bytes().iter().eq(hash128(input))
                && root
                    .prepared_hash()
                    .bytes()
                    .iter()
                    .eq(hash128(source.as_bytes()))
                && root.config_hash().bytes().iter().eq(config),
            "style source/preparation/config mismatch"
        );
        let records = root.facts().bytes();
        ensure!(
            root.records_hash().bytes().iter().eq(hash128(records)),
            "style extraction provenance mismatch"
        );
        let facts = flatbuffers::root_with_opts::<wire::Facts<'_>>(
            &crate::options(records.len()),
            records,
        )?;
        validate(facts, source)?;
        Ok(Self { bytes })
    }

    pub fn facts(&self) -> wire::Facts<'_> {
        // SAFETY: both table graphs were structurally and semantically checked
        // before owning these immutable bytes.
        unsafe {
            let root = flatbuffers::root_unchecked::<wire::Style<'_>>(
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
    let span = |span: &wire::Span| -> Result<()> {
        let start = usize::try_from(span.start())?;
        let end = usize::try_from(span.end())?;
        ensure!(
            start <= end && source.get(start..end).is_some(),
            "invalid stylesheet UTF-8 span {start}..{end}"
        );
        Ok(())
    };
    let contains = |outer: &wire::Span, inner: &wire::Span| {
        outer.start() <= inner.start() && inner.end() <= outer.end()
    };
    let name = |name: wire::Name<'_>| -> Result<()> {
        span(name.span())?;
        ensure!(
            name.span().start() < name.span().end()
                && !name.decoded().is_empty()
                && !name.decoded().contains('\0'),
            "invalid stylesheet name"
        );
        Ok(())
    };
    let mut last = 0;
    for url in facts.urls() {
        span(url.span())?;
        ensure!(
            last <= url.span().start() && url.span().start() < url.span().end(),
            "unordered stylesheet URLs"
        );
        last = url.span().end();
    }
    for selector in facts.names() {
        ensure!(
            matches!(
                selector.kind(),
                wire::SelectorKind::Class | wire::SelectorKind::Id
            ),
            "unknown stylesheet selector kind"
        );
        name(selector.name())?;
    }
    for selector in facts.selectors() {
        span(selector)?;
    }
    for scope in facts.scopes() {
        ensure!(
            matches!(
                scope.kind(),
                wire::ScopeKind::Local | wire::ScopeKind::Global
            ),
            "unknown stylesheet scope kind"
        );
        span(scope.span())?;
        span(scope.owner())?;
        ensure!(
            contains(scope.owner(), scope.span()),
            "scope outside selector"
        );
        if let Some(argument) = scope.argument() {
            span(argument)?;
            ensure!(
                contains(scope.span(), argument)
                    && argument.end() >= argument.start() + 2
                    && !scope.alternatives().is_empty()
                    && scope.commas().len() + 1 == scope.alternatives().len(),
                "invalid stylesheet functional scope"
            );
            for alternative in scope.alternatives() {
                span(alternative)?;
                ensure!(
                    argument.start() < alternative.start() && alternative.end() < argument.end(),
                    "scope alternative outside argument"
                );
            }
            for (index, comma) in scope.commas().iter().enumerate() {
                span(comma)?;
                ensure!(
                    comma.end() == comma.start() + 1
                        && source.as_bytes()[comma.start() as usize] == b','
                        && scope.alternatives().get(index).end() <= comma.start()
                        && comma.end() <= scope.alternatives().get(index + 1).start(),
                    "invalid stylesheet scope comma"
                );
            }
        } else {
            ensure!(
                scope.alternatives().is_empty() && scope.commas().is_empty(),
                "bare scope has alternatives"
            );
        }
    }
    for composition in facts.compositions() {
        span(composition.span())?;
        if let Some(selector) = composition.selector() {
            span(selector)?;
        }
        ensure!(
            matches!(
                composition.kind(),
                wire::CompositionKind::Local
                    | wire::CompositionKind::Global
                    | wire::CompositionKind::File
            ) && (composition.kind() == wire::CompositionKind::File)
                == composition.request().is_some(),
            "invalid stylesheet composition source"
        );
        if let Some(request) = composition.request() {
            name(request)?;
            ensure!(
                contains(composition.span(), request.span()),
                "composition request outside declaration"
            );
        }
        ensure!(
            !composition.names().is_empty(),
            "empty stylesheet composition"
        );
        for item in composition.names() {
            name(item)?;
            ensure!(
                contains(composition.span(), item.span()),
                "composition name outside declaration"
            );
        }
    }
    for animation in facts.animations() {
        name(animation.name())?;
        span(animation.span())?;
        ensure!(
            contains(animation.span(), animation.name().span())
                && matches!(
                    animation.local(),
                    wire::Locality::Implicit | wire::Locality::Local | wire::Locality::Global
                ),
            "invalid stylesheet animation"
        );
    }
    Ok(())
}
