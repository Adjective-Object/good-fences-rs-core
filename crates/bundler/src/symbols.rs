//! Canonical source identities; allocated names are not yet source rewrites.
use std::collections::{BTreeMap, BTreeSet, HashMap};

use ast_segmenter::owned::BindingId;
use ast_segmenter::views::{ExportValue, ImportedName};
use eyre::{ensure, OptionExt};
use source_graph::{ExportIdentity, GraphNode, LinkedExport, Namespace, SegmentKey, SourceGraph};

use crate::chunks::ChunkPlan;

pub(super) type Symbol = (u32, BindingId);

pub(super) struct Definition {
    /// Lexical allocation only; never substitute this for an observable `.name`.
    pub name: String,
}

pub(super) struct Symbols {
    pub definitions: HashMap<Symbol, Definition>,
    /// Read targets only; writes still address the read-only local import.
    pub imports: HashMap<Symbol, LinkedExport>,
    pub exports: BTreeMap<u32, BTreeMap<String, LinkedExport>>,
}

fn reserved(name: &str) -> bool {
    "await break case catch class const continue debugger default delete do else enum \
     export extends false finally for function if implements import in instanceof \
     interface let new null package private protected public return static super \
     switch this throw true try typeof var void while with yield eval arguments"
        .split_whitespace()
        .any(|keyword| keyword == name)
}

pub(crate) fn valid_rename_target(name: &str) -> bool {
    !name.is_empty()
        && !reserved(name)
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || byte == b'_'
                || byte == b'$'
                || (index > 0 && byte.is_ascii_digit())
        })
}

fn allocate_name(
    preferred: &str,
    occupied: &mut BTreeSet<String>,
    originals: &BTreeSet<String>,
    fresh: &mut usize,
) -> String {
    let unavailable = |name: &str| occupied.contains(name) || reserved(name);
    let mut name = preferred.to_owned();
    if unavailable(&name) {
        loop {
            name = format!("__gf_link_{fresh}");
            *fresh += 1;
            if !unavailable(&name) && !originals.contains(&name) {
                break;
            }
        }
    }
    occupied.insert(name.clone());
    name
}

impl Symbols {
    pub fn link(graph: &SourceGraph, plan: &ChunkPlan) -> eyre::Result<Self> {
        ensure!(
            plan.revision == graph.revision(),
            "stale symbol partition revision"
        );
        graph.validate_links()?;
        let retained: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
        let generated_constructors: std::collections::HashSet<_> = plan.lazy_modules.modules().iter()
            .filter(|module| plan.async_activations.values().any(|activation| {
                matches!(activation.origin, crate::chunks::AsyncOrigin::Lazy { declaration, module_candidate, .. }
                    if declaration == module.owner && module_candidate == module.candidate)
            }))
            .filter_map(|module| graph.file_facts(module.owner.file_id)
                .and_then(|facts| facts.references().get(module.constructor_reference as usize))
                .and_then(|reference| reference.binding())
                .map(|binding| (module.owner.file_id, binding)))
            .collect();
        let route_retained = |linked: &LinkedExport| {
            linked.route.iter().all(|node| match node {
                GraphNode::Segment(key) => retained.contains(key),
                _ => super::link::retained(plan, *node),
            })
        };
        let mut files = plan.retention.modules.clone();
        files.sort_by_key(|&file| graph.file_path(file));
        // Reserve nested names and free references across sources: relocating a
        // definition must neither capture a global nor be captured in a closure.
        let mut reserved = BTreeSet::new();
        let mut originals = BTreeSet::new();
        for &file in &files {
            let facts = graph
                .file_facts(file)
                .ok_or_eyre("missing owned link facts")?;
            for binding in facts.bindings() {
                originals.insert(binding.name().to_owned());
                // Top-level imports disappear after checked read substitution;
                // reserving their aliases needlessly renames observable functions.
                if !binding.top_level() || facts.commonjs() {
                    reserved.insert(binding.name().to_owned());
                }
            }
            reserved.extend(
                facts
                    .references()
                    .iter()
                    .filter(|r| r.binding().is_none())
                    .map(|r| r.name().to_owned()),
            );
        }
        let mut result = Self {
            definitions: HashMap::new(),
            imports: HashMap::new(),
            exports: BTreeMap::new(),
        };
        let mut fresh = 0;
        for &file in &files {
            let facts = graph
                .file_facts(file)
                .ok_or_eyre("missing owned link facts")?;
            let segments = graph
                .file_segments(file)
                .ok_or_eyre("missing source segments")?;
            let mut bindings: Vec<_> = facts
                .bindings()
                .iter()
                .filter(|b| {
                    b.top_level()
                        && !b.type_only()
                        && !b.ambient()
                        && !facts.commonjs()
                        && graph.runtime_import(file, b.id())
                        && !plan.lazy_modules.consumes_binding(file, b.id())
                })
                .collect();
            bindings.sort_by_key(|b| b.id().0);
            for binding in bindings {
                let mut declarations = BTreeSet::new();
                for span in binding.declarations() {
                    let index = segments.partition_point(|s| s.span.end <= span.start);
                    ensure!(
                        segments
                            .get(index)
                            .is_some_and(|s| super::link::contains(s.span, span)),
                        "binding declaration outside source segments"
                    );
                    let key = SegmentKey::new(file, index as u32);
                    if retained.contains(&key) {
                        declarations.insert(key);
                    }
                }
                if declarations.is_empty() {
                    continue;
                }
                if binding.import() {
                    let source_used = if plan.lazy_modules.modules().is_empty() {
                        crate::cjs::binding_used(graph, &plan.retention, file, binding.id())
                    } else {
                        facts
                            .references()
                            .iter()
                            .enumerate()
                            .any(|(index, reference)| {
                                let segment = SegmentKey::new(
                                    file,
                                    segments
                                        .partition_point(|s| s.span.end <= reference.span().start)
                                        as u32,
                                );
                                !reference.type_only()
                                    && reference.binding() == Some(binding.id())
                                    && retained.contains(&segment)
                                    && !plan
                                        .lazy_modules
                                        .consumed_constructor(segment, index as u32)
                            })
                            || facts.exports().iter().any(|export| {
                                !export.type_only()
                                    && graph.runtime_export(file, export.name())
                                    && export.value() == ExportValue::Binding(binding.id())
                                    && retained.contains(&SegmentKey::new(
                                        file,
                                        segments
                                            .partition_point(|s| s.span.end <= export.span().start)
                                            as u32,
                                    ))
                            })
                    };
                    if plan.precise_dynamic_imports
                        && !source_used
                        && !generated_constructors.contains(&(file, binding.id()))
                    {
                        continue;
                    }
                    let import = facts
                        .imports()
                        .iter()
                        .find(|i| i.binding() == binding.id())
                        .ok_or_eyre("missing owned import")?;
                    let dependency = facts
                        .dependencies()
                        .iter()
                        .find(|d| d.id() == import.dependency())
                        .ok_or_eyre("missing import dependency")?;
                    let mut linked =
                        if let Some(source_graph::DependencyTarget::External(specifier)) =
                            graph.dependency_target(file, dependency.id())
                        {
                            let node = GraphNode::Unlinked {
                                file_id: file,
                                dependency: dependency.id(),
                            };
                            LinkedExport {
                                identity: ExportIdentity::External(
                                    specifier.clone(),
                                    match &import.imported() {
                                        ImportedName::Name(name) => Some((*name).to_owned()),
                                        ImportedName::Namespace => None,
                                    },
                                ),
                                declaration: node,
                                route: vec![node],
                            }
                        } else {
                            let target = super::link::target_file(graph, file, dependency)?;
                            match &import.imported() {
                                ImportedName::Name(name) => {
                                    graph.resolve_export(target, &name, Namespace::Value)?
                                }
                                ImportedName::Namespace => LinkedExport {
                                    identity: ExportIdentity::Namespace(target),
                                    declaration: GraphNode::Module(target),
                                    route: vec![GraphNode::Module(target)],
                                },
                            }
                        };
                    linked
                        .route
                        .extend(declarations.into_iter().map(GraphNode::Segment));
                    ensure!(
                        route_retained(&linked),
                        "canonical import route is not retained"
                    );
                    result.imports.insert((file, binding.id()), linked);
                    continue;
                }
                let name = allocate_name(&binding.name(), &mut reserved, &originals, &mut fresh);
                result
                    .definitions
                    .insert((file, binding.id()), Definition { name });
            }
        }
        let files: BTreeSet<_> = files.into_iter().collect();
        for (file, names) in super::link::export_names(graph, plan) {
            if !files.contains(&file) {
                continue;
            }
            let facts = graph.file_facts(file).ok_or_eyre("missing export facts")?;
            let mut exports = BTreeMap::new();
            for name in names {
                let linked = match graph.resolve_export(file, &name, Namespace::Value) {
                    Ok(linked) => linked,
                    Err(error)
                        if !facts
                            .exports()
                            .iter()
                            .any(|e| e.name() == Some(name.as_str())) =>
                    {
                        // ResolveExport omits ambiguous/cycle-only star names.
                        ensure!(
                            error.to_string().starts_with("ambiguous ")
                                || error.to_string().starts_with("missing "),
                            "{error:#}"
                        );
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                if route_retained(&linked) {
                    exports.insert(name, linked);
                }
            }
            result.exports.insert(file, exports);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_avoids_keywords_collisions_and_future_source_names() {
        let mut occupied = BTreeSet::from(["__gf_link_1".into()]);
        let originals = BTreeSet::from(["__gf_link_0".into()]);
        let mut fresh = 0;
        let names: Vec<_> = ["class", "private", "value", "value"]
            .into_iter()
            .map(|name| allocate_name(name, &mut occupied, &originals, &mut fresh))
            .collect();
        assert_eq!(
            names,
            ["__gf_link_2", "__gf_link_3", "value", "__gf_link_4"]
        );
    }
}
