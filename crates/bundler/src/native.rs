//! Scope-hoisted ESM, with native module jobs for observable and async seams.
use super::*;
use source_graph::ExportIdentity;
use std::collections::{HashMap, HashSet};

use crate::assembly::{Code, Edit};
use crate::rewrite;
use crate::symbols::Symbols;

#[path = "native_chunks.rs"]
mod chunks;

#[cfg(test)]
thread_local! {
    static GLOBAL_PREPARATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn global_preparation_count() -> usize {
    GLOBAL_PREPARATIONS.with(std::cell::Cell::get)
}

pub(super) fn emit(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    symbols: &Symbols,
) -> eyre::Result<Artifacts<Code>> {
    let prepared = PreparedValues::new(graph, modules, symbols);
    if let Some(output) = single(graph, plan, modules, manifest, &prepared)? {
        return Ok(output);
    }
    chunks::emit(graph, plan, modules, manifest, &prepared)
}

fn single(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
) -> eyre::Result<Option<Artifacts<Code>>> {
    let symbols = prepared.symbols;
    // Prefer whole-tree concatenation; the group planner handles native seams.
    if modules.keys().any(|&file| has_externals(graph, file)) {
        return Ok(None);
    }
    if plan.roots.len() != 1 || plan.chunks.len() != 1 {
        return Ok(None);
    }
    let (RootId::Entry(entry), seed) = plan.roots.first_key_value().unwrap() else {
        return Ok(None);
    };
    let [file] = seed.evaluation_roots.as_slice() else {
        return Ok(None);
    };
    if seed.binding_roots != [GraphNode::Module(*file)] {
        return Ok(None);
    }
    for module in modules.values() {
        let facts = &module.parsed.facts;
        let imports: HashSet<_> = facts.imports.iter().map(|i| i.binding).collect();
        if facts.commonjs
            || !facts.top_level_await.is_empty()
            || facts.dependencies.iter().any(|d| {
                !d.type_only && !matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
            })
            || facts
                .references
                .iter()
                .any(|r| r.write && r.binding.is_some_and(|b| imports.contains(&b)))
        {
            return Ok(None);
        }
    }
    if symbols.exports[file]
        .values()
        .any(|e| matches!(e.identity, ExportIdentity::Namespace(_)))
    {
        return Ok(None);
    }

    // Static dependency occurrences, not path order or the scalar effect DAG,
    // define synchronous ESM evaluation. Gray nodes defer real binding cycles.
    let mut state = BTreeMap::new();
    let mut order = Vec::new();
    let mut stack = vec![(*file, false)];
    while let Some((file, finish)) = stack.pop() {
        if finish {
            state.insert(file, 2);
            order.push(file);
            continue;
        }
        match state.get(&file) {
            Some(1) => return Ok(None),
            Some(2) => continue,
            _ => {}
        }
        state.insert(file, 1);
        stack.push((file, true));
        let module = modules.get(&file).ok_or_eyre("missing native dependency")?;
        for dependency in module
            .parsed
            .facts
            .dependencies
            .iter()
            .rev()
            .filter(|d| !d.type_only)
        {
            stack.push((target_file(graph, file, dependency)?, false));
        }
    }
    ensure!(
        order.len() == modules.len(),
        "native root does not cover retained modules"
    );
    let values = Values::new(prepared);
    let mut output = Code::default();
    let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
    let recipes: BTreeMap<_, _> = plan
        .retention
        .effects
        .iter()
        .filter(|e| !e.summary.rewrites.is_empty())
        .map(|e| (e.key, e.summary.rewrites.as_slice()))
        .collect();
    if let Some(hashbang) = &modules[file].parsed.facts.hashbang {
        output.push_str(
            &modules[file].source[hashbang.span.start as usize..hashbang.span.end as usize],
        );
        output.push_str("\n");
    }
    let mut bodies = Code::default();
    for file in order {
        let Some((hoisted, body)) = body(graph, file, &modules[&file], &values, &live, &recipes)?
        else {
            return Ok(None);
        };
        output.push(hoisted);
        bodies.push(body);
        // Separate statements even when a source lacks a final semicolon/newline.
        bodies.push_str("\n;\n");
    }
    output.push(bodies);
    output.push_str("export {\n");
    for (public, linked) in &symbols.exports[file] {
        output.push_str(&format!(
            "{} as {},\n",
            values
                .name(&linked.identity)
                .ok_or_eyre("missing native export value")?,
            quote(public)
        ));
    }
    output.push_str("};\n");
    let path = PathBuf::from("native.mjs");
    output.default_export = symbols.exports[file].contains_key("default");
    let (chunk, nodes) = plan.chunks.first_key_value().unwrap();
    Ok(Some(Artifacts {
        backend: super::Backend::Native,
        resources: BTreeMap::new(),
        stylesheets: BTreeMap::new(),
        files: BTreeMap::from([(path.clone(), output)]),
        entries: BTreeMap::from([(entry.clone(), path.clone())]),
        // Only the public entry namespace is executable as an independent module.
        modules: BTreeMap::from([(graph.file_path(*file).unwrap().to_owned(), path.clone())]),
        members: nodes
            .iter()
            .map(|node| (node.clone(), path.clone()))
            .collect(),
        chunk_artifacts: BTreeMap::from([(chunk.clone(), BTreeSet::from([path]))]),
        manifest: manifest.clone(),
        cjs_proxies: BTreeMap::new(),
        cjs_namespaces: BTreeMap::new(),
        async_facades: BTreeMap::new(),
        async_modules: BTreeSet::new(),
    }))
}

struct PreparedValues<'s> {
    symbols: &'s Symbols,
    definitions: Vec<crate::symbols::Symbol>,
    defaults: BTreeMap<SegmentKey, String>,
    occupied: HashSet<String>,
}

impl<'s> PreparedValues<'s> {
    fn new(graph: &SourceGraph, modules: &BTreeMap<u32, Module<'_>>, symbols: &'s Symbols) -> Self {
        #[cfg(test)]
        GLOBAL_PREPARATIONS.with(|count| count.set(count.get() + 1));
        let mut definitions: Vec<_> = symbols.definitions.keys().copied().collect();
        definitions.sort_by_key(|&(file, binding)| (graph.file_path(file), binding.0));
        let mut occupied: HashSet<_> = symbols
            .definitions
            .values()
            .map(|d| d.name.clone())
            .collect();
        // Export-only names constrain defaults, but not chunk-local import aliases.
        let exports: HashSet<_> = symbols
            .exports
            .values()
            .flat_map(|exports| exports.keys().map(String::as_str))
            .collect();
        for module in modules.values() {
            occupied.extend(module.parsed.facts.bindings.iter().map(|b| b.name.clone()));
            occupied.extend(
                module
                    .parsed
                    .facts
                    .references
                    .iter()
                    .map(|r| r.name.clone()),
            );
        }
        let mut files: Vec<_> = modules.keys().copied().collect();
        files.sort_by_key(|&file| graph.file_path(file));
        let mut defaults = BTreeMap::new();
        let mut fresh = 0;
        for file in files {
            let module = &modules[&file];
            for export in &module.parsed.facts.exports {
                if let ExportValue::Expression(span) = export.value {
                    let index = module
                        .parsed
                        .segments
                        .partition_point(|s| s.span.end <= span.start);
                    let name = loop {
                        let name = format!("__gf_default_{fresh}");
                        fresh += 1;
                        if !exports.contains(name.as_str()) && occupied.insert(name.clone()) {
                            break name;
                        }
                    };
                    defaults.insert(SegmentKey::new(file, index as u32), name);
                }
            }
        }
        Self {
            symbols,
            definitions,
            defaults,
            occupied,
        }
    }
}

/// Owners borrow the graph-wide exclusions and store only their own additions.
struct OccupiedNames<'s> {
    shared: &'s HashSet<String>,
    local: HashSet<String>,
}

impl<'s> OccupiedNames<'s> {
    fn new(prepared: &'s PreparedValues<'_>) -> Self {
        Self {
            shared: &prepared.occupied,
            local: HashSet::new(),
        }
    }

    fn insert(&mut self, name: String) -> bool {
        !self.shared.contains(&name) && self.local.insert(name)
    }
}

struct Values<'s> {
    symbols: &'s Symbols,
    defaults: &'s BTreeMap<SegmentKey, String>,
    names: HashMap<crate::symbols::Symbol, String>,
    imports: HashMap<(u32, ast_segmenter::owned::BindingId), String>,
    requests: BTreeMap<u32, Code>,
    dynamic: BTreeMap<u32, Code>,
    contexts: HashMap<(u32, ast_segmenter::owned::DependencyId), crate::context::Dispatch>,
    native_defaults: BTreeSet<u32>,
}

impl<'s> Values<'s> {
    fn new(prepared: &'s PreparedValues<'_>) -> Self {
        Self {
            symbols: prepared.symbols,
            defaults: &prepared.defaults,
            names: HashMap::new(),
            imports: HashMap::new(),
            requests: BTreeMap::new(),
            dynamic: BTreeMap::new(),
            contexts: HashMap::new(),
            native_defaults: BTreeSet::new(),
        }
    }

    fn name(&self, identity: &ExportIdentity) -> Option<&str> {
        match identity {
            ExportIdentity::Binding(file, binding) => self
                .names
                .get(&(*file, *binding))
                .map(String::as_str)
                .or_else(|| {
                    self.symbols
                        .definitions
                        .get(&(*file, *binding))
                        .map(|d| d.name.as_str())
                }),
            ExportIdentity::Expression(key) => self.defaults.get(key).map(String::as_str),
            _ => None,
        }
    }
}

fn compatible(
    graph: &SourceGraph,
    file: u32,
    module: &Module<'_>,
    values: &Values<'_>,
    live: &BTreeSet<SegmentKey>,
) -> eyre::Result<bool> {
    use ast_segmenter::emission::NamespaceAccess;
    let facts = &module.parsed.facts;
    let retained: Vec<_> = facts
        .emission
        .segments
        .iter()
        .enumerate()
        .filter(|(index, _)| live.contains(&SegmentKey::new(file, *index as u32)))
        .map(|(_, recipe)| recipe)
        .collect();
    let mut changed = HashSet::new();
    let mut namespaces = HashMap::new();
    for binding in facts.bindings.iter().filter(|binding| binding.top_level) {
        let symbol = (file, binding.id);
        let target = if let Some(name) = values.imports.get(&symbol) {
            Some(name.as_str())
        } else if values.symbols.definitions.contains_key(&symbol) {
            values.name(&ExportIdentity::Binding(file, binding.id))
        } else if let Some(import) = values.symbols.imports.get(&symbol) {
            if let ExportIdentity::Namespace(target) = import.identity {
                namespaces.insert(binding.id, target);
            }
            values.name(&import.identity)
        } else {
            None
        };
        if target.is_some_and(|name| name != binding.name) {
            changed.insert(binding.id);
        }
    }
    let inference_changes = retained.iter().any(|recipe| {
        recipe
            .placement
            .declarations
            .iter()
            .any(|id| changed.contains(id))
            || recipe.placement.writes.iter().any(|&index| {
                facts.references[index as usize]
                    .binding
                    .is_some_and(|id| changed.contains(&id))
            })
    });
    for recipe in retained {
        if recipe.names.named.iter().any(|id| changed.contains(id))
            || (inference_changes && recipe.names.inference_sensitive)
        {
            return Ok(false);
        }
        for usage in &recipe.namespaces {
            let reference = &facts.references[usage.reference as usize];
            let Some(target) = reference.binding.and_then(|id| namespaces.get(&id)) else {
                continue;
            };
            let NamespaceAccess::DirectRead { property, .. } = &usage.access else {
                return Ok(false);
            };
            let Ok(linked) =
                graph.resolve_export(*target, property, source_graph::Namespace::Value)
            else {
                return Ok(false);
            };
            if values.name(&linked.identity).is_none() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn body(
    graph: &SourceGraph,
    file: u32,
    module: &Module<'_>,
    values: &Values<'_>,
    live: &BTreeSet<SegmentKey>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
) -> eyre::Result<Option<(Code, Code)>> {
    use ast_segmenter::emission::{ExportForm, NamespaceAccess, OperationKind};
    if !compatible(graph, file, module, values, live)? {
        return Ok(None);
    }
    let facts = &module.parsed.facts;
    let mut targets = HashMap::new();
    let mut namespaces = HashMap::new();
    for binding in facts.bindings.iter().filter(|binding| binding.top_level) {
        let symbol = (file, binding.id);
        let target = if let Some(name) = values.imports.get(&symbol) {
            Some(name.as_str())
        } else if values.symbols.definitions.contains_key(&symbol) {
            values.name(&ExportIdentity::Binding(file, binding.id))
        } else if let Some(import) = values.symbols.imports.get(&symbol) {
            if let ExportIdentity::Namespace(target) = import.identity {
                namespaces.insert(binding.id, target);
            }
            values.name(&import.identity)
        } else {
            None
        };
        if let Some(name) = target.filter(|name| *name != binding.name) {
            targets.insert(binding.id, name);
        }
    }
    let mut hoisted = Code::default();
    let mut output = Code::default();
    for index in 0..module.parsed.segments.len() {
        let key = SegmentKey::new(file, index as u32);
        if !live.contains(&key) {
            continue;
        }
        let recipe = &facts.emission.segments[index];
        let mut renderer = crate::emit::CachedRenderer::new(
            module.source,
            facts,
            recipe,
            recipes.get(&key).copied().unwrap_or_default(),
        );
        renderer.relocate_urls(module.relocations)?;
        for (operation_index, operation) in recipe.operations.iter().enumerate() {
            let span = operation.kind.span(facts, recipe);
            let replacement = match &operation.kind {
                OperationKind::Binding {
                    binding, shorthand, ..
                } => targets
                    .get(binding)
                    .map(|name| {
                        rewrite::shorthand_replacement(module.source, span, name, *shorthand)
                    })
                    .transpose()?
                    .map(Code::from),
                OperationKind::Reference {
                    reference,
                    shorthand,
                } => facts.references[*reference as usize]
                    .binding
                    .and_then(|id| targets.get(&id))
                    .map(|name| {
                        rewrite::shorthand_replacement(module.source, span, name, *shorthand)
                    })
                    .transpose()?
                    .map(Code::from),
                OperationKind::Namespace { usage } => {
                    let usage = &recipe.namespaces[*usage as usize];
                    if let Some(target) = facts.references[usage.reference as usize]
                        .binding
                        .and_then(|id| namespaces.get(&id))
                    {
                        let NamespaceAccess::DirectRead { property, .. } = &usage.access else {
                            unreachable!()
                        };
                        let linked = graph.resolve_export(
                            *target,
                            property,
                            source_graph::Namespace::Value,
                        )?;
                        Some(Code::from(
                            values
                                .name(&linked.identity)
                                .ok_or_eyre("missing native namespace value")?
                                .to_owned(),
                        ))
                    } else {
                        None
                    }
                }
                OperationKind::Dependency { dependency } => {
                    let dependency = facts
                        .dependencies
                        .iter()
                        .find(|d| d.id == *dependency)
                        .ok_or_eyre("missing original dependency")?;
                    if matches!(
                        dependency.kind,
                        DependencyKind::Import | DependencyKind::ReExport
                    ) {
                        Some(";".into())
                    } else if dependency.type_only
                        || dependency.kind != DependencyKind::DynamicImport
                    {
                        None
                    } else {
                        if let Some(dispatch) = values.contexts.get(&(file, dependency.id)) {
                            renderer.context(dependency, dispatch.clone());
                            continue;
                        }
                        let target = target_file(graph, file, dependency)?;
                        if let Some(expression) = values.dynamic.get(&target) {
                            Some(expression.clone())
                        } else {
                            renderer.edit(
                                operation_index as u32,
                                Edit {
                                    span: dependency.specifier_span,
                                    replacement: values
                                        .requests
                                        .get(&target)
                                        .ok_or_eyre("missing native dynamic target")?
                                        .clone(),
                                },
                            );
                            None
                        }
                    }
                }
                _ => None,
            };
            if let Some(replacement) = replacement {
                renderer.replace(operation_index as u32, replacement);
            }
        }
        for (operation_index, operation) in recipe.operations.iter().enumerate().rev() {
            let OperationKind::Export { form, .. } = &operation.kind else {
                continue;
            };
            let replacement = match form {
                ExportForm::Remove => ";".into(),
                ExportForm::Declaration { declaration } => {
                    renderer.children(*declaration, Some(operation_index as u32))?
                }
                ExportForm::Default {
                    value,
                    hoisted: is_hoisted,
                } => {
                    if values.native_defaults.contains(&file) {
                        continue;
                    }
                    let name = values
                        .defaults
                        .get(&key)
                        .ok_or_eyre("missing native default binding")?;
                    let value = renderer.slot(value)?;
                    if *is_hoisted {
                        hoisted.push(
                            value.surrounded(format!("var {name} = ({{default: "), "}).default;\n"),
                        );
                        ";".into()
                    } else {
                        value.surrounded(format!("const {name} = ({{default: "), "}).default;")
                    }
                }
            };
            renderer.replace(operation_index as u32, replacement);
        }
        output.push(renderer.materialize()?);
        // Retention and PURE rewrites can expose a new ASI continuation.
        output.push_str("\n;\n");
    }
    Ok(Some((hoisted, output)))
}
