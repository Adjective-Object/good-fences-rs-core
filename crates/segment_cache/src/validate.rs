use std::collections::{HashMap, HashSet};

use ast_segmenter::{owned::*, ModuleBody};
use eyre::ensure;
use oxc_span::Span;

use crate::{CacheRecord, ParserConfig, EXTRACTOR_VERSION, OXC_REVISION, SCHEMA_VERSION};

pub(super) fn span(source: &str, range: Span) -> eyre::Result<()> {
    ensure!(
        range.start <= range.end
            && source
                .get(range.start as usize..range.end as usize)
                .is_some(),
        "invalid UTF-8 source span"
    );
    Ok(())
}

pub(super) fn contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

fn sorted<T: Ord>(values: impl IntoIterator<Item = T>) -> bool {
    let mut previous = None;
    for value in values {
        if previous.as_ref().is_some_and(|p| p > &value) {
            return false;
        }
        previous = Some(value);
    }
    true
}

impl CacheRecord {
    pub(crate) fn validate(&self, source: &str, config: ParserConfig) -> eyre::Result<()> {
        let _span = tracing::info_span!("cache.validate").entered();
        ensure!(
            self.magic == "good-fences-segments"
                && self.schema == SCHEMA_VERSION
                && self.extractor == EXTRACTOR_VERSION
                && self.oxc == OXC_REVISION,
            "cache version mismatch"
        );
        ensure!(
            source.len() <= u32::MAX as usize
                && self.source_len == source.len() as u64
                && self.source_hash == crate::digest(source.as_bytes()),
            "cache source mismatch"
        );
        ensure!(self.config == config, "cache parser configuration mismatch");
        self.provenance.verify(
            source,
            config,
            self.module.body,
            &self.module.segments,
            &self.module.facts,
        )?;
        ensure!(
            !config.definition || config.typescript,
            "invalid parser mode"
        );
        let module = &self.module;
        for s in &module.segments {
            span(source, *s)?;
        }
        ensure!(
            module.segments.windows(2).all(|w| w[0].end <= w[1].start),
            "unordered/overlapping segments"
        );
        match module.body {
            ModuleBody::CommonJs { span: body } => {
                span(source, body)?;
                ensure!(
                    config.commonjs && module.segments == [body],
                    "invalid opaque CJS body"
                );
                ensure!(body.end as usize == source.len(), "incomplete CJS body");
                ensure!(
                    body.start == module.facts.hashbang.as_ref().map_or(0, |h| h.span.end),
                    "invalid CJS body start"
                );
            }
            ModuleBody::Esm => {
                ensure!(!config.commonjs, "invalid ESM body");
                ensure!(
                    module.segments.iter().all(|s| s.start < s.end),
                    "empty ESM segment"
                );
            }
        }
        let facts = &module.facts;
        match (&facts.ecco, module.body) {
            (Some(parsed), ModuleBody::Esm) => {
                parsed.validate()?;
                ensure!(
                    sorted(parsed.roots().iter().map(|r| (r.span.start, r.span.end)))
                        && sorted(parsed.symbols().iter().map(|s| s.id.0)),
                    "unordered Ecco source records"
                );
                let mut occupied = HashSet::new();
                for root in parsed.roots() {
                    span(source, root.span)?;
                    ensure!(root.span.start < root.span.end, "empty Ecco root");
                    let segment = module
                        .segments
                        .iter()
                        .position(|s| contains(*s, root.span))
                        .ok_or_else(|| eyre::eyre!("Ecco root outside segment"))?;
                    ensure!(occupied.insert(segment), "multiple Ecco roots in segment");
                }
                for symbol in parsed.symbols() {
                    span(source, symbol.span)?;
                    ensure!(
                        facts.bindings.iter().any(|binding| binding.top_level
                            && binding.id.0 == symbol.id.0
                            && binding.name == symbol.name
                            && binding.declarations.contains(&symbol.span)),
                        "Ecco symbol does not match source binding"
                    );
                }
                for reference in parsed.external_references() {
                    span(source, reference.span)?;
                    ensure!(
                        module.segments.iter().any(|s| contains(*s, reference.span)),
                        "Ecco reference outside segment"
                    );
                }
                for matched in parsed.definition_matches() {
                    span(source, matched.span)?;
                    ensure!(
                        module.segments.iter().any(|s| contains(*s, matched.span)),
                        "Ecco declaration match outside segment"
                    );
                }
            }
            (None, ModuleBody::CommonJs { .. }) => {}
            _ => eyre::bail!("invalid Ecco module representation"),
        }
        ensure!(
            facts.commonjs == config.commonjs,
            "facts parser format mismatch"
        );
        ensure!(
            (!facts.commonjs || facts.top_level_await.is_empty())
                && sorted(facts.top_level_await.iter().map(|s| (s.start, s.end))),
            "invalid top-level await facts"
        );
        for site in &facts.top_level_await {
            span(source, *site)?;
            ensure!(
                site.start < site.end && module.segments.iter().any(|s| contains(*s, *site)),
                "top-level await outside statement"
            );
        }
        ensure!(
            sorted(
                facts
                    .effects
                    .iter()
                    .map(|s| (s.span.start, s.span.end, s.reason))
            ),
            "unordered effects"
        );
        for effect in &facts.effects {
            span(source, effect.span)?;
            ensure!(
                module.segments.iter().any(|s| contains(*s, effect.span)),
                "effect outside statement"
            );
            if let Some(deferred) = effect.deferred {
                span(source, deferred)?;
                ensure!(contains(deferred, effect.span), "invalid deferred effect");
            }
        }
        ensure!(
            sorted(facts.invocations.iter().map(|c| (c.span.start, c.span.end))),
            "unordered invocations"
        );
        for call in &facts.invocations {
            span(source, call.span)?;
            span(source, call.callee)?;
            ensure!(
                call.span.start < call.span.end
                    && call.callee.start < call.callee.end
                    && contains(call.span, call.callee)
                    && module.segments.iter().any(|s| contains(*s, call.span)),
                "invalid invocation range"
            );
            if let Some(deferred) = call.deferred {
                span(source, deferred)?;
                ensure!(contains(deferred, call.span), "invalid deferred invocation");
            }
            if let Some(pure) = call.pure {
                span(source, pure)?;
                let text = &source[pure.start as usize..pure.end as usize];
                let marker = text
                    .strip_prefix("/*")
                    .and_then(|s| s.strip_suffix("*/"))
                    .map(str::trim);
                ensure!(
                    pure.end <= call.span.start
                        && matches!(marker, Some("@__PURE__" | "#__PURE__")),
                    "invalid PURE attachment"
                );
            }
            let mut previous = call.callee.end;
            for argument in &call.arguments {
                span(source, argument.span)?;
                ensure!(
                    argument.span.start < argument.span.end
                        && contains(call.span, argument.span)
                        && previous <= argument.span.start,
                    "invalid invocation argument"
                );
                previous = argument.span.end;
            }
            use ast_segmenter::invocation::CalleeShape;
            if let CalleeShape::Member { receiver, .. } = &call.shape {
                span(source, *receiver)?;
                ensure!(contains(call.callee, *receiver), "invalid callee receiver");
            }
        }
        let bindings: HashSet<_> = facts.bindings.iter().map(|b| b.id).collect();
        let dependencies: HashMap<_, _> = facts.dependencies.iter().map(|d| (d.id, d)).collect();
        ensure!(
            bindings.len() == facts.bindings.len()
                && dependencies.len() == facts.dependencies.len()
                && sorted(facts.bindings.iter().map(|b| b.id.0))
                && sorted(facts.dependencies.iter().map(|d| d.id.0)),
            "duplicate/unordered local IDs"
        );
        for call in &facts.invocations {
            use ast_segmenter::invocation::CalleeShape;
            let identifier = match &call.shape {
                CalleeShape::Identifier(id) => Some(id),
                CalleeShape::Member {
                    receiver_identifier,
                    ..
                } => receiver_identifier.as_ref(),
                CalleeShape::Other => None,
            };
            ensure!(
                identifier.is_none_or(|id| id.binding.is_none_or(|b| bindings.contains(&b))),
                "dangling callee binding"
            );
        }
        for b in &facts.bindings {
            ensure!(!b.declarations.is_empty(), "binding without declarations");
            ensure!(b.id.0 == b.declarations[0].start, "invalid binding ID");
            ensure!(
                b.declarations
                    .windows(2)
                    .all(|w| (w[0].start, w[0].end) < (w[1].start, w[1].end)),
                "duplicate/unordered declarations"
            );
            for s in &b.declarations {
                span(source, *s)?;
                ensure!(s.start < s.end, "empty declaration");
            }
        }
        ensure!(
            sorted(facts.references.iter().map(|r| (r.span.start, r.span.end))),
            "unordered references"
        );
        for r in &facts.references {
            span(source, r.span)?;
            ensure!(r.span.start < r.span.end, "empty reference");
            ensure!(
                r.binding.is_none_or(|b| bindings.contains(&b)),
                "dangling reference"
            );
            ensure!(
                r.commonjs.is_none() || (config.commonjs && r.binding.is_none() && !r.type_only),
                "invalid wrapper reference"
            );
        }
        for d in &facts.dependencies {
            if let Some(pattern) = &d.pattern {
                ensure!(
                    matches!(
                        d.kind,
                        DependencyKind::DynamicImport | DependencyKind::Require
                    ) && pattern
                        .first()
                        .is_some_and(|p| p.starts_with("./") || p.starts_with("../"))
                        && d.specifier == pattern.join("*"),
                    "invalid relative dependency pattern"
                );
            }
            span(source, d.span)?;
            span(source, d.specifier_span)?;
            ensure!(
                d.id.0 == d.span.start
                    && d.span.start < d.span.end
                    && contains(d.span, d.specifier_span),
                "invalid dependency site"
            );
            if let Some(function) = d.enclosing_function {
                span(source, function)?;
                ensure!(contains(function, d.span), "invalid function containment");
            }
        }
        ensure!(
            sorted(facts.imports.iter().map(|i| i.span.start)),
            "unordered imports"
        );
        for i in &facts.imports {
            span(source, i.span)?;
            span(source, i.imported_span)?;
            ensure!(bindings.contains(&i.binding), "dangling import binding");
            let dependency = dependencies.get(&i.dependency);
            ensure!(
                dependency
                    .is_some_and(|d| d.kind == DependencyKind::Import && contains(d.span, i.span)),
                "invalid import dependency"
            );
            ensure!(
                contains(i.span, i.imported_span),
                "invalid import name range"
            );
        }
        ensure!(
            sorted(
                facts
                    .exports
                    .iter()
                    .map(|e| (e.span.start, e.name_span.map(|s| s.start)))
            ),
            "unordered exports"
        );
        for e in &facts.exports {
            span(source, e.span)?;
            if let Some(name) = e.name_span {
                span(source, name)?;
                ensure!(contains(e.span, name), "invalid export name range");
            }
            match e.value {
                ExportValue::Binding(id) => ensure!(bindings.contains(&id), "dangling export"),
                ExportValue::ReExport { dependency, .. } => ensure!(
                    dependencies
                        .get(&dependency)
                        .is_some_and(|d| d.kind == DependencyKind::ReExport),
                    "dangling re-export"
                ),
                ExportValue::Expression(s) => {
                    span(source, s)?;
                    ensure!(contains(e.span, s), "invalid export expression range");
                }
            }
        }
        ensure!(
            sorted(facts.directives.iter().map(|d| d.span.start)),
            "unordered directives"
        );
        for d in facts.directives.iter().chain(facts.hashbang.iter()) {
            span(source, d.span)?;
        }
        crate::validate_emission::validate(module, source)?;
        Ok(())
    }
}
