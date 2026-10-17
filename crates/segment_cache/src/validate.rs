use std::collections::HashSet;

use ast_segmenter::{owned::*, ModuleBody};
use eyre::ensure;
use oxc_span::Span;

use crate::{CacheRecord, ParserConfig, EXTRACTOR_VERSION, OXC_REVISION, SCHEMA_VERSION};

fn span(source: &str, range: Span) -> eyre::Result<()> {
    ensure!(
        range.start <= range.end
            && source
                .get(range.start as usize..range.end as usize)
                .is_some(),
        "invalid UTF-8 source span"
    );
    Ok(())
}

fn contains(outer: Span, inner: Span) -> bool {
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
        let bindings: HashSet<_> = facts.bindings.iter().map(|b| b.id).collect();
        let dependencies: HashSet<_> = facts.dependencies.iter().map(|d| d.id).collect();
        ensure!(
            bindings.len() == facts.bindings.len()
                && dependencies.len() == facts.dependencies.len()
                && sorted(facts.bindings.iter().map(|b| b.id.0))
                && sorted(facts.dependencies.iter().map(|d| d.id.0)),
            "duplicate/unordered local IDs"
        );
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
            let dependency = facts.dependencies.iter().find(|d| d.id == i.dependency);
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
                    facts
                        .dependencies
                        .iter()
                        .any(|d| d.id == dependency && d.kind == DependencyKind::ReExport),
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
        Ok(())
    }
}
