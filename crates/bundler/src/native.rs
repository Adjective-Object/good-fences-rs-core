//! Scope-hoisted ESM, with native module jobs for observable and async seams.
use super::*;
use source_graph::ExportIdentity;
use std::collections::{HashMap, HashSet};

use crate::assembly::{Code, Edit};
use crate::rewrite;
use crate::symbols::Symbols;

#[path = "native_chunks.rs"]
mod chunks;

pub(super) fn placement(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    symbols: &Symbols,
    min_chunk_bytes: usize,
) -> eyre::Result<NativePlacement> {
    let prepared = PreparedValues::new(graph, modules, symbols);
    chunks::placement(graph, plan, modules, manifest, &prepared, min_chunk_bytes)
}

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
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
    placements: &BTreeMap<u32, u32>,
    min_chunk_bytes: usize,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts<Code>> {
    let prepared = tracing::info_span!("emit.global_names")
        .in_scope(|| PreparedValues::new(graph, modules, symbols));
    if !placements.is_empty() {
        return chunks::emit_activated(
            graph,
            plan,
            modules,
            manifest,
            &prepared,
            browser,
            require_defaults,
            placements,
            min_chunk_bytes,
            execution,
        );
    }
    let placements = if plan
        .async_activations
        .keys()
        .any(|site| chunks::selected_activation(graph, plan, site))
    {
        BTreeMap::new()
    } else {
        single_parent(graph, plan, modules, symbols)?
    };
    if !placements.is_empty() {
        return chunks::emit_activated(
            graph,
            plan,
            modules,
            manifest,
            &prepared,
            browser,
            require_defaults,
            &placements,
            min_chunk_bytes,
            execution,
        );
    }
    if min_chunk_bytes != 0 || !plan.generated_boundaries.is_empty() {
        return chunks::emit_activated(
            graph,
            plan,
            modules,
            manifest,
            &prepared,
            browser,
            require_defaults,
            &BTreeMap::new(),
            min_chunk_bytes,
            execution,
        );
    }
    ordinary(
        graph,
        plan,
        modules,
        manifest,
        &prepared,
        browser,
        require_defaults,
        execution,
    )
}

fn ordinary(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
    browser: bool,
    require_defaults: &BTreeSet<PathBuf>,
    execution: crate::execution::ExecutionOptions,
) -> eyre::Result<Artifacts<Code>> {
    if let Some(output) = tracing::info_span!("emit.single")
        .in_scope(|| single(graph, plan, modules, manifest, prepared, None))?
    {
        return Ok(output);
    }
    chunks::emit(
        graph,
        plan,
        modules,
        manifest,
        prepared,
        browser,
        require_defaults,
        execution,
    )
}

fn single_parent(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
) -> eyre::Result<BTreeMap<u32, u32>> {
    let mut incoming = BTreeMap::<u32, Vec<(u32, Dependency<'_>)>>::new();
    for (&file, module) in modules {
        for dependency in module
            .parsed
            .facts()
            .dependencies()
            .iter()
            .filter(|d| !d.type_only())
        {
            if dependency.kind() == DependencyKind::DynamicImport
                && !plan.roots.contains_key(&RootId::Async(AsyncSiteId {
                    module: graph.file_path(file).unwrap().to_owned(),
                    offset: dependency.id().0,
                    candidate: None,
                }))
                && dependency.pattern().is_none()
            {
                continue;
            }
            for target in target_files(graph, file, dependency)? {
                incoming.entry(target).or_default().push((file, dependency));
            }
        }
    }
    let mut candidates: Vec<_> = incoming.keys().copied().collect();
    candidates.sort_by_key(|&file| graph.file_path(file));
    let mut placements = BTreeMap::new();
    for target in candidates {
        let uses = &incoming[&target];
        let parent = uses[0].0;
        if target == parent
            || uses.iter().any(|(file, dependency)| {
                *file != parent
                    || dependency.kind() != DependencyKind::DynamicImport
                    || dependency.pattern().is_some()
            })
        {
            continue;
        }
        // Include the complete static island, not just the namespace's body.
        let mut island = BTreeSet::new();
        let mut pending = vec![target];
        while let Some(file) = pending.pop() {
            if !island.insert(file) {
                continue;
            }
            let Some(module) = modules.get(&file) else {
                continue;
            };
            for dependency in module.parsed.facts().dependencies().iter().filter(|d| {
                !d.type_only()
                    && matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
            }) {
                pending.extend(target_files(graph, file, dependency)?);
            }
        }
        if island.contains(&parent)
            || island.iter().any(|file| {
                !modules.contains_key(file)
                    || modules[file].parsed.facts().emission().uses_this()
                    || incoming.get(file).is_some_and(|uses| {
                        uses.iter().any(|(consumer, dependency)| {
                            !island.contains(consumer)
                                && (*file != target
                                    || *consumer != parent
                                    || dependency.kind() != DependencyKind::DynamicImport)
                        })
                    })
            })
            || uses.iter().any(|(file, dependency)| {
                !dynamic_namespace_safe(modules, symbols, *file, *dependency, target)
            })
        {
            continue;
        }

        let candidate: BTreeMap<_, _> = island.iter().map(|&file| (file, parent)).collect();
        if Activations::checked(graph, plan, modules, symbols, &candidate)?.is_none() {
            continue;
        }
        let mut combined = placements.clone();
        combined.extend(candidate);
        if Activations::checked(graph, plan, modules, symbols, &combined)?.is_some() {
            placements = combined;
        }
    }
    Ok(placements)
}

fn dynamic_namespace_safe(
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    file: u32,
    dependency: Dependency<'_>,
    target: u32,
) -> bool {
    let Some(properties) = modules[&file]
        .parsed
        .facts()
        .emission()
        .dynamic_namespaces()
        .iter()
        .find(|n| n.dependency() == dependency.id())
        .and_then(|n| n.properties())
    else {
        return false;
    };
    properties.iter().all(|(name, receiver)| {
        let Some(export) = symbols.exports[&target].get(name) else {
            return false;
        };
        !receiver
            || match export.identity {
                ExportIdentity::Binding(file, id) => {
                    let facts = &modules[&file].parsed.facts();
                    facts
                        .bindings()
                        .iter()
                        .any(|b| b.id() == id && b.function())
                        && !facts
                            .references()
                            .iter()
                            .any(|r| r.binding() == Some(id) && r.write() && !r.type_only())
                }
                _ => false,
            }
    })
}

#[derive(Clone)]
struct Initialization {
    init: String,
    state: String,
    failure: String,
    caught: String,
    namespace: String,
}

impl Initialization {
    fn new(modules: &BTreeMap<u32, Module<'_>>, symbols: &Symbols) -> Self {
        Self::named(modules, symbols, "")
    }

    fn named(modules: &BTreeMap<u32, Module<'_>>, symbols: &Symbols, suffix: &str) -> Self {
        let mut occupied: BTreeSet<_> = modules
            .values()
            .flat_map(|m| {
                m.parsed
                    .facts()
                    .bindings()
                    .iter()
                    .map(|b| b.name().to_owned())
                    .chain(
                        m.parsed
                            .facts()
                            .references()
                            .iter()
                            .map(|r| r.name().to_owned()),
                    )
            })
            .chain(symbols.definitions.values().map(|d| d.name.clone()))
            .chain(symbols.exports.values().flat_map(|e| e.keys().cloned()))
            .collect();
        let mut fresh = |base: &str| {
            let mut name = format!("{base}{suffix}");
            while !occupied.insert(name.clone()) {
                name.push('$');
            }
            name
        };
        Self {
            init: fresh("__gf_init"),
            state: fresh("__gf_init_state"),
            failure: fresh("__gf_init_failure"),
            caught: fresh("__gf_init_caught"),
            namespace: fresh("__gf_namespace"),
        }
    }

    fn guard(&self, body: Code) -> Code {
        let Self {
            init,
            state,
            failure,
            caught,
            ..
        } = self;
        body.surrounded(
            format!(
                "var {state} = 0, {failure};\nvar {init} = () => {{\n\
                 if ({state} === 2) return;\n\
                 if ({state} === 3) throw {failure};\n\
                 if ({state} === 1) throw 'Unsupported deferred ESM reentry';\n\
                 {state} = 1;\ntry {{\n"
            ),
            format!(
                "\n{state} = 2;\n}} catch ({caught}) {{\n\
                 {failure} = {caught}; {state} = 3; throw {caught};\n}}\n}};\n"
            ),
        )
    }
}

/// Only explicitly selected, synchronous islands enter this reconstruction mode.
#[derive(Clone)]
struct Activations {
    initializers: BTreeMap<u32, Initialization>,
    owners: BTreeMap<u32, u32>,
    exports: BTreeSet<u32>,
}

impl Activations {
    fn checked(
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        symbols: &Symbols,
        placements: &BTreeMap<u32, u32>,
    ) -> eyre::Result<Option<Self>> {
        use ast_segmenter::views::DeferredForm;
        if modules.keys().any(|&file| has_externals(graph, file)) {
            return Ok(None);
        }
        let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
        let mut owners = placements.clone();
        let mut pending: Vec<_> = placements.keys().copied().collect();
        let mut seen = BTreeSet::new();
        while let Some(file) = pending.pop() {
            if !seen.insert(file) {
                continue;
            }
            let Some(module) = modules.get(&file) else {
                return Ok(None);
            };
            let facts = &module.parsed.facts();
            if !modules.contains_key(&owners[&file])
                || facts.commonjs()
                || !facts.top_level_await().is_empty()
                || !module.relocations.is_empty()
                || symbols.exports[&file].contains_key("then")
                || symbols.exports[&file]
                    .values()
                    .any(|e| matches!(e.identity, ExportIdentity::Namespace(_)))
                || facts.references().iter().any(|r| {
                    r.write()
                        && r.binding().is_some_and(|id| {
                            facts
                                .bindings()
                                .iter()
                                .any(|b| b.id() == id && (b.constant() || b.import()))
                        })
                })
                || facts
                    .emission()
                    .segments()
                    .iter()
                    .enumerate()
                    .any(|(index, recipe)| {
                        live.contains(&SegmentKey::new(file, index as u32))
                            && matches!(recipe.deferred(), DeferredForm::Unsupported)
                    })
            {
                return Ok(None);
            }
            for recipe in facts.emission().segments() {
                for usage in recipe.namespaces() {
                    let binding = facts
                        .references()
                        .get(usage.reference() as usize)
                        .unwrap()
                        .binding();
                    if binding
                        .and_then(|b| symbols.imports.get(&(file, b)))
                        .is_some_and(|linked| {
                            matches!(linked.identity, ExportIdentity::Namespace(_))
                        })
                        && !matches!(
                            usage.access(),
                            ast_segmenter::views::NamespaceAccess::DirectRead { .. }
                        )
                    {
                        return Ok(None);
                    }
                }
            }
            for dependency in facts.dependencies().iter().filter(|d| !d.type_only()) {
                if !matches!(
                    dependency.kind(),
                    DependencyKind::Import | DependencyKind::ReExport
                ) {
                    return Ok(None);
                }
                let target = target_file(graph, file, dependency)?;
                owners.entry(target).or_insert(target);
                pending.push(target);
            }
        }
        // A public/eager consumer cannot share this deferred evaluation contract.
        for (root, seed) in &plan.roots {
            if matches!(root, RootId::Entry(_))
                && (seed.evaluation_roots.iter().any(|f| seen.contains(f))
                    || seed.binding_roots.iter().any(|n| match n {
                        GraphNode::Module(f) => seen.contains(f),
                        GraphNode::Segment(k) => seen.contains(&k.file_id),
                        _ => false,
                    }))
            {
                return Ok(None);
            }
        }
        let mut edges = BTreeMap::new();
        for (&file, module) in modules {
            let mut targets = Vec::new();
            for dependency in module
                .parsed
                .facts()
                .dependencies()
                .iter()
                .filter(|d| !d.type_only())
            {
                let dependency_targets = target_files(graph, file, dependency)?;
                let activates = dependency_targets.iter().any(|f| seen.contains(f));
                let synchronous = matches!(
                    dependency.kind(),
                    DependencyKind::Import | DependencyKind::ReExport
                );
                if activates
                    && (module.parsed.facts().commonjs()
                        || dependency.pattern().is_some()
                        || (synchronous && !seen.contains(&file))
                        || (!synchronous && dependency.kind() != DependencyKind::DynamicImport))
                {
                    return Ok(None);
                }
                if synchronous && seen.contains(&file) {
                    targets.extend(dependency_targets);
                }
            }
            if seen.contains(&file) {
                edges.insert(file, targets);
            }
        }
        let files: Vec<_> = seen.iter().copied().collect();
        let indices: BTreeMap<_, _> = files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
        let edges: Vec<_> = files
            .iter()
            .map(|f| edges[f].iter().map(|t| indices[t]).collect())
            .collect();
        if crate::effect_dependencies::components(&edges)
            .iter()
            .any(|c| c.len() > 1 || edges[c[0]].contains(&c[0]))
        {
            return Ok(None);
        }
        // Owners must be final physical files, not chains of requested moves.
        if owners
            .values()
            .any(|owner| owners.get(owner).is_some_and(|next| next != owner))
        {
            return Ok(None);
        }
        Ok(Some(Self {
            initializers: files
                .into_iter()
                .map(|file| {
                    let suffix = format!(
                        "_{}",
                        digest(graph.file_path(file).unwrap().to_str().unwrap())
                    );
                    (file, Initialization::named(modules, symbols, &suffix))
                })
                .collect(),
            owners,
            exports: BTreeSet::new(),
        }))
    }

    fn dynamic(&self, graph: &SourceGraph, file: u32, owner: &std::path::Path) -> Code {
        use crate::assembly::RequestKind;
        let init = &self.initializers[&file];
        let target = self.owners[&file];
        if chunks::path(graph, target) == owner {
            format!(
                "(async () => {{ await 0; {}(); return {}; }})()",
                init.init, init.namespace
            )
            .into()
        } else {
            Code::request(chunks::path(graph, target), RequestKind::DynamicImport).surrounded(
                "import(",
                format!(
                    ").then(m => (m[{}](), m[{}]))",
                    quote(&init.init),
                    quote(&init.namespace)
                ),
            )
        }
    }
}

pub(super) fn deferred(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    symbols: &Symbols,
) -> eyre::Result<Option<(Artifacts<Code>, String)>> {
    let initialization = Initialization::new(modules, symbols);
    let prepared = PreparedValues::new(graph, modules, symbols);
    Ok(single(
        graph,
        plan,
        modules,
        manifest,
        &prepared,
        Some(&initialization),
    )?
    .map(|artifacts| (artifacts, initialization.init)))
}

fn single(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    manifest: &DiagnosticManifest,
    prepared: &PreparedValues<'_>,
    initialization: Option<&Initialization>,
) -> eyre::Result<Option<Artifacts<Code>>> {
    let symbols = prepared.symbols;
    // Bounded fast path: a single entrance covers the complete acyclic synchronous
    // closure. Native dependency completion order is unchanged, with no outside
    // activation, namespace, async or independently executable entry boundaries.
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
        let facts = &module.parsed.facts();
        let imports: HashSet<_> = facts.imports().iter().map(|i| i.binding()).collect();
        if facts.commonjs()
            || !facts.top_level_await().is_empty()
            || facts.dependencies().iter().any(|d| {
                !d.type_only()
                    && !matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
            })
            || facts
                .references()
                .iter()
                .any(|r| r.write() && r.binding().is_some_and(|b| imports.contains(&b)))
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
    if initialization.is_some()
        && modules.values().any(|module| {
            let facts = &module.parsed.facts();
            facts.references().iter().any(|r| {
                r.write()
                    && r.binding().is_some_and(|id| {
                        facts
                            .bindings()
                            .iter()
                            .any(|b| b.id() == id && b.constant())
                    })
            })
        })
    {
        // Lowering const to var must not make existing illegal writes succeed.
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
            .facts()
            .dependencies()
            .iter()
            .rev()
            .filter(|d| !d.type_only())
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
    let schedule: Vec<_> = order
        .iter()
        .flat_map(|&file| {
            (0..modules[&file].parsed.segments().len())
                .map(move |index| SegmentKey::new(file, index as u32))
                .filter(|key| live.contains(key))
        })
        .collect();
    let positions: BTreeMap<_, _> = schedule
        .iter()
        .enumerate()
        .map(|(index, &key)| (key, index))
        .collect();
    if plan
        .retention
        .execution
        .constraints
        .iter()
        .any(|constraint| {
            constraint.kind != crate::execution_dependencies::ConstraintKind::EffectOrder
                && positions[&constraint.before] >= positions[&constraint.after]
        })
    {
        return Ok(None);
    }
    let recipes: BTreeMap<_, _> = plan
        .retention
        .effects
        .iter()
        .filter(|e| !e.summary.rewrites.is_empty())
        .map(|e| (e.key, e.summary.rewrites.as_slice()))
        .collect();
    if let Some(hashbang) = &modules[file].parsed.facts().hashbang() {
        output.push_str(
            &modules[file].source[hashbang.span().start as usize..hashbang.span().end as usize],
        );
        output.push_str("\n");
    }
    let mut bodies = Code::default();
    for key in schedule {
        let file = key.file_id;
        let Some((hoisted, body)) = reconstruct(
            graph,
            key,
            &modules[&file],
            &values,
            &recipes,
            initialization.is_some(),
        )?
        else {
            return Ok(None);
        };
        output.push(hoisted);
        bodies.push(body);
        // Separate statements even when a source lacks a final semicolon/newline.
        bodies.push_str("\n;\n");
    }
    output.push(match initialization {
        Some(initialization) => initialization.guard(bodies),
        None => bodies,
    });
    output.push_str("export {\n");
    if let Some(initialization) = initialization {
        output.push_str(&format!("{},\n", initialization.init));
    }
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
    files: Vec<u32>,
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
            occupied.extend(
                module
                    .parsed
                    .facts()
                    .bindings()
                    .iter()
                    .map(|b| b.name().to_owned()),
            );
            occupied.extend(
                module
                    .parsed
                    .facts()
                    .references()
                    .iter()
                    .map(|r| r.name().to_owned()),
            );
        }
        let mut files: Vec<_> = modules.keys().copied().collect();
        files.sort_by_key(|&file| graph.file_path(file));
        let mut defaults = BTreeMap::new();
        let mut fresh = 0;
        for &file in &files {
            let module = &modules[&file];
            for export in module.parsed.facts().exports() {
                if let ExportValue::Expression(span) = export.value() {
                    let index = module
                        .parsed
                        .segments()
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
            files,
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
    dynamic_sites: HashMap<(u32, ast_segmenter::owned::DependencyId), Code>,
    contexts: HashMap<(u32, ast_segmenter::owned::DependencyId), crate::context::Dispatch>,
    native_defaults: BTreeSet<u32>,
    generated: BTreeMap<(u32, u32), Code>,
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
            dynamic_sites: HashMap::new(),
            contexts: HashMap::new(),
            native_defaults: BTreeSet::new(),
            generated: BTreeMap::new(),
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
    use ast_segmenter::views::NamespaceAccess;
    let facts = &module.parsed.facts();
    let retained: Vec<_> = facts
        .emission()
        .segments()
        .iter()
        .enumerate()
        .filter(|(index, _)| live.contains(&SegmentKey::new(file, *index as u32)))
        .map(|(_, recipe)| recipe)
        .collect();
    let mut changed = HashSet::new();
    let mut namespaces = HashMap::new();
    for binding in facts
        .bindings()
        .iter()
        .filter(|binding| binding.top_level())
    {
        let symbol = (file, binding.id());
        let target = if let Some(name) = values.imports.get(&symbol) {
            Some(name.as_str())
        } else if values.symbols.definitions.contains_key(&symbol) {
            values.name(&ExportIdentity::Binding(file, binding.id()))
        } else if let Some(import) = values.symbols.imports.get(&symbol) {
            if let ExportIdentity::Namespace(target) = import.identity {
                namespaces.insert(binding.id(), target);
            }
            values.name(&import.identity)
        } else {
            None
        };
        if target.is_some_and(|name| name != binding.name()) {
            changed.insert(binding.id());
        }
    }
    for recipe in retained {
        let inference_changes = recipe
            .placement()
            .declarations()
            .iter()
            .any(|id| changed.contains(&id))
            || recipe.placement().writes().iter().any(|index| {
                facts
                    .references()
                    .get(index as usize)
                    .unwrap()
                    .binding()
                    .is_some_and(|id| changed.contains(&id))
            });
        if recipe
            .names()
            .named()
            .iter()
            .any(|id| changed.contains(&id))
            || (inference_changes && recipe.names().inference_sensitive())
        {
            return Ok(false);
        }
        for usage in recipe.namespaces() {
            let reference = facts.references().get(usage.reference() as usize).unwrap();
            let Some(target) = reference.binding().and_then(|id| namespaces.get(&id)) else {
                continue;
            };
            let NamespaceAccess::DirectRead { property, .. } = &usage.access() else {
                return Ok(false);
            };
            let Ok(linked) =
                graph.resolve_export(*target, &property, source_graph::Namespace::Value)
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

fn reconstruct(
    graph: &SourceGraph,
    key: SegmentKey,
    module: &Module<'_>,
    values: &Values<'_>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
    deferred: bool,
) -> eyre::Result<Option<(Code, Code)>> {
    use ast_segmenter::views::{ExportForm, NamespaceAccess, OperationKind};
    let file = key.file_id;
    if !compatible(graph, file, module, values, &BTreeSet::from([key]))? {
        return Ok(None);
    }
    let facts = module.parsed.facts();
    let mut targets = HashMap::new();
    let mut namespaces = HashMap::new();
    for binding in facts
        .bindings()
        .iter()
        .filter(|binding| binding.top_level())
    {
        let symbol = (file, binding.id());
        let target = if let Some(name) = values.imports.get(&symbol) {
            Some(name.as_str())
        } else if values.symbols.definitions.contains_key(&symbol) {
            values.name(&ExportIdentity::Binding(file, binding.id()))
        } else if let Some(import) = values.symbols.imports.get(&symbol) {
            if let ExportIdentity::Namespace(target) = import.identity {
                namespaces.insert(binding.id(), target);
            }
            values.name(&import.identity)
        } else {
            None
        };
        if let Some(name) = target.filter(|name| *name != binding.name()) {
            targets.insert(binding.id(), name);
        }
    }
    let mut hoisted = Code::default();
    let mut output = Code::default();
    let index = key.segment_idx as usize;
    let recipe = facts.emission().segments().get(index).unwrap();
    let mut renderer = crate::emit::CachedRenderer::new(
        module.source,
        facts,
        recipe,
        recipes.get(&key).copied().unwrap_or_default(),
    );
    renderer.relocate_urls(module.relocations)?;
    for (operation_index, operation) in recipe.operations().iter().enumerate() {
        let span = operation.kind().span(facts, recipe);
        let replacement = match &operation.kind() {
            OperationKind::Binding {
                binding, shorthand, ..
            } => targets
                .get(&binding)
                .map(|name| rewrite::shorthand_replacement(module.source, span, name, *shorthand))
                .transpose()?
                .map(Code::from),
            OperationKind::Reference {
                reference,
                shorthand,
            } => {
                if let Some(expression) = values.generated.get(&(file, *reference)) {
                    let prefix =
                        rewrite::shorthand_replacement(module.source, span, "", *shorthand)?;
                    Some(expression.clone().surrounded(prefix, ""))
                } else {
                    facts
                        .references()
                        .get(*reference as usize)
                        .unwrap()
                        .binding()
                        .and_then(|id| targets.get(&id))
                        .map(|name| {
                            rewrite::shorthand_replacement(module.source, span, name, *shorthand)
                        })
                        .transpose()?
                        .map(Code::from)
                }
            }
            OperationKind::Namespace { usage } => {
                let usage = recipe.namespaces().get(*usage as usize).unwrap();
                if let Some(target) = facts
                    .references()
                    .get(usage.reference() as usize)
                    .unwrap()
                    .binding()
                    .and_then(|id| namespaces.get(&id))
                {
                    let NamespaceAccess::DirectRead { property, .. } = &usage.access() else {
                        unreachable!()
                    };
                    let linked =
                        graph.resolve_export(*target, &property, source_graph::Namespace::Value)?;
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
                    .dependencies()
                    .iter()
                    .find(|d| d.id() == *dependency)
                    .ok_or_eyre("missing original dependency")?;
                if matches!(
                    dependency.kind(),
                    DependencyKind::Import | DependencyKind::ReExport
                ) {
                    Some(";".into())
                } else if dependency.type_only()
                    || dependency.kind() != DependencyKind::DynamicImport
                {
                    None
                } else {
                    if let Some(dispatch) = values.contexts.get(&(file, dependency.id())) {
                        renderer.context(dependency, dispatch.clone());
                        continue;
                    }
                    if let Some(request) = values.dynamic_sites.get(&(file, dependency.id())) {
                        renderer.edit(
                            operation_index as u32,
                            Edit {
                                span: dependency.specifier_span(),
                                replacement: request.clone(),
                            },
                        );
                        continue;
                    }
                    let target = target_file(graph, file, dependency)?;
                    if let Some(expression) = values.dynamic.get(&target) {
                        Some(expression.clone())
                    } else {
                        renderer.edit(
                            operation_index as u32,
                            Edit {
                                span: dependency.specifier_span(),
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
    for (operation_index, operation) in recipe.operations().iter().enumerate().rev() {
        let OperationKind::Export { form, .. } = &operation.kind() else {
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
                let value = renderer.slot(*value)?;
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
    if deferred {
        use ast_segmenter::views::DeferredForm;
        match &recipe.deferred() {
            DeferredForm::Unsupported => return Ok(None),
            DeferredForm::Function => {
                hoisted.push(renderer.materialize()?);
                hoisted.push_str("\n;\n");
            }
            DeferredForm::Variable {
                binding,
                initializer,
            } => {
                let definition = facts
                    .bindings()
                    .iter()
                    .find(|b| b.id() == *binding)
                    .ok_or_eyre("missing deferred variable")?;
                let name = values
                    .name(&ExportIdentity::Binding(file, *binding))
                    .ok_or_eyre("missing deferred variable name")?;
                hoisted.push_str(&format!("var {name};\n"));
                if let Some(initializer) = initializer {
                    output.push(
                        renderer
                            .slot(*initializer)?
                            .surrounded(format!("{name} = ("), ");\n"),
                    );
                } else if !definition.function_scoped() {
                    output.push_str(&format!("{name} = void 0;\n"));
                }
            }
            DeferredForm::Effect => output.push(renderer.materialize()?),
        }
    } else {
        output.push(renderer.materialize()?);
    }
    // Retention and PURE rewrites can expose a new ASI continuation.
    output.push_str("\n;\n");
    Ok(Some((hoisted, output)))
}
