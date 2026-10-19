//! Physical ownership for pure declarations, independent of source body seams.
use super::*;
use crate::RetentionEdgeKind;

pub(super) struct Placement {
    pub owners: BTreeMap<SegmentKey, Owner>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Owner {
    Evaluation(u32),
    Bindings(SegmentKey),
}

impl Owner {
    pub fn path(self, graph: &SourceGraph) -> PathBuf {
        match self {
            Self::Evaluation(file) => path(graph, file),
            Self::Bindings(key) => PathBuf::from(format!(
                "native/bindings-{}-{}.mjs",
                digest(graph.file_path(key.file_id).unwrap().to_str().unwrap()),
                key.segment_idx
            )),
        }
    }

    fn request(self, graph: &SourceGraph) -> String {
        quote(&format!(
            "./{}",
            self.path(graph).file_name().unwrap().to_str().unwrap()
        ))
    }
}

impl Placement {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        symbols: &Symbols,
        seams: &BTreeMap<u32, u32>,
        phase_bound: &BTreeSet<u32>,
        surfaces: &BTreeSet<u32>,
        cjs: &commonjs::CommonJs,
    ) -> eyre::Result<Self> {
        let live: BTreeSet<_> = plan.retention.segments.iter().map(|s| s.key).collect();
        let effects: BTreeSet<_> = plan
            .retention
            .effects
            .iter()
            .filter(|s| s.summary.is_effectful())
            .map(|s| s.key)
            .collect();
        let keys: Vec<_> = live.iter().copied().collect();
        let indices: BTreeMap<_, _> = keys.iter().enumerate().map(|(i, &key)| (key, i)).collect();
        let mut constraints = vec![BTreeSet::new(); keys.len()];
        let mut pinned = BTreeSet::new();
        let mut declarations = BTreeSet::new();
        let mut deferred = BTreeMap::new();
        let mut hoisted = BTreeSet::new();
        for (&file, module) in modules {
            if cjs.contains(file) {
                deferred.insert(file, Vec::new());
                continue;
            }
            let allocator = Allocator::default();
            rewrite::count_output_parse();
            let parsed = Parser::new(&allocator, module.source, SourceType::mjs()).parse();
            let mut functions = Functions::default();
            functions.visit_program(&parsed.program);
            deferred.insert(file, functions.0);
            for statement in &parsed.program.body {
                let function = match statement {
                    Statement::FunctionDeclaration(function) => Some(function),
                    Statement::ExportNamedDeclaration(export) => match &export.declaration {
                        Some(Declaration::FunctionDeclaration(function)) => Some(function),
                        _ => None,
                    },
                    Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                            Some(function)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(id) = function.and_then(|function| function.id.as_ref()) {
                    hoisted.insert((file, id.span.start));
                }
            }
        }
        for &key in &live {
            let module = &modules[&key.file_id];
            let span = module.parsed.segments[key.segment_idx as usize].span;
            let facts = &module.parsed.facts;
            let definitions: Vec<_> = facts
                .bindings
                .iter()
                .filter(|b| {
                    b.top_level && !b.import && b.declarations.iter().any(|&s| contains(span, s))
                })
                .collect();
            let expression = facts
                .exports
                .iter()
                .any(|e| matches!(e.value, ExportValue::Expression(s) if contains(span, s)));
            if !definitions.is_empty() || expression {
                declarations.insert(key);
            }
            let instantiable = !definitions.is_empty()
                && definitions.iter().all(|b| {
                    b.declarations
                        .iter()
                        .filter(|&&declaration| contains(span, declaration))
                        .all(|declaration| hoisted.contains(&(key.file_id, declaration.start)))
                });
            if cjs.contains(key.file_id)
                || effects.contains(&key)
                || (phase_bound.contains(&key.file_id) && !instantiable)
                || definitions.iter().any(|b| {
                    facts
                        .references
                        .iter()
                        .any(|r| r.binding == Some(b.id) && r.span.start < span.start)
                })
            {
                pinned.insert(key);
            }
            for binding in &definitions {
                for &declaration in &binding.declarations {
                    let index = module
                        .parsed
                        .segments
                        .partition_point(|s| s.span.end <= declaration.start);
                    let other = SegmentKey::new(key.file_id, index as u32);
                    if let Some(&other) = indices.get(&other) {
                        constraints[indices[&key]].insert(other);
                        constraints[other].insert(indices[&key]);
                    }
                }
            }
            for reference in facts.references.iter().filter(|r| contains(span, r.span)) {
                let Some(binding) = reference.binding.and_then(|id| {
                    facts
                        .bindings
                        .iter()
                        .find(|b| b.id == id && b.top_level && !b.import)
                }) else {
                    continue;
                };
                let dependency =
                    declaration(graph, &ExportIdentity::Binding(key.file_id, binding.id))?.unwrap();
                if reference.write {
                    constraints[indices[&key]].insert(indices[&dependency]);
                    constraints[indices[&dependency]].insert(indices[&key]);
                }
            }
        }
        // Eager reads share their initializer's phase; deferred reads can cross
        // chunks as live imports. This also keeps snapshots ahead of later writes.
        for &key in &live {
            let module = &modules[&key.file_id];
            let span = module.parsed.segments[key.segment_idx as usize].span;
            for reference in module.parsed.facts.references.iter().filter(|r| {
                r.read
                    && contains(span, r.span)
                    && !deferred[&key.file_id].iter().any(|&f| contains(f, r.span))
            }) {
                let Some(binding) = reference.binding.and_then(|id| {
                    module
                        .parsed
                        .facts
                        .bindings
                        .iter()
                        .find(|b| b.id == id && b.top_level)
                }) else {
                    continue;
                };
                if binding.import {
                    if let Some(linked) = symbols.imports.get(&(key.file_id, binding.id)) {
                        if let ExportIdentity::Binding(file, id) = linked.identity {
                            if modules[&file]
                                .parsed
                                .facts
                                .references
                                .iter()
                                .any(|r| r.binding == Some(id) && r.write)
                            {
                                pinned.insert(key);
                            }
                        }
                    }
                } else {
                    let dependency =
                        declaration(graph, &ExportIdentity::Binding(key.file_id, binding.id))?
                            .unwrap();
                    constraints[indices[&key]].insert(indices[&dependency]);
                    constraints[indices[&dependency]].insert(indices[&key]);
                }
            }
        }
        let components = crate::effect_dependencies::components(&constraints);
        let mut component_for = BTreeMap::new();
        for (index, component) in components.iter().enumerate() {
            for &node in component {
                component_for.insert(keys[node], index);
            }
        }
        let movable: BTreeSet<_> = components
            .iter()
            .enumerate()
            .filter_map(|(i, nodes)| {
                (nodes.iter().any(|&n| declarations.contains(&keys[n]))
                    && nodes.iter().all(|&n| !pinned.contains(&keys[n])))
                .then_some(i)
            })
            .collect();
        let transparent: BTreeSet<_> = live
            .iter()
            .copied()
            .filter(|key| {
                movable.contains(&component_for[key])
                    || (!declarations.contains(key) && !effects.contains(key))
            })
            .collect();
        let mut users: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for edge in &plan.retention.edges {
            if edge.kind == RetentionEdgeKind::Binding {
                if let (GraphNode::Segment(from), GraphNode::Segment(to)) = (edge.from, edge.to) {
                    users.entry(to).or_default().push(from);
                }
            }
        }
        let mut public_users: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for &file in surfaces {
            for linked in symbols.exports[&file].values() {
                if let Some(key) = declaration(graph, &linked.identity)? {
                    public_users.entry(key).or_default().insert(seams[&file]);
                }
            }
        }
        for seed in plan.roots.values() {
            for node in &seed.binding_roots {
                if let GraphNode::Segment(key) = node {
                    public_users
                        .entry(*key)
                        .or_default()
                        .insert(seams[&key.file_id]);
                }
            }
        }
        let mut readonly_users: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        for (&(file, binding), linked) in &symbols.imports {
            let facts = &modules[&file].parsed.facts;
            if facts.references.iter().any(|reference| {
                if !reference.write || reference.binding != Some(binding) {
                    return false;
                }
                let index = modules[&file]
                    .parsed
                    .segments
                    .partition_point(|segment| segment.span.end <= reference.span.start);
                live.contains(&SegmentKey::new(file, index as u32))
            }) {
                if let Some(key) = declaration(graph, &linked.identity)? {
                    readonly_users.entry(key).or_default().insert(seams[&file]);
                }
            }
        }
        let mut owners: BTreeMap<_, _> = live
            .iter()
            .map(|&k| (k, Owner::Evaluation(seams[&k.file_id])))
            .collect();
        let mut shared = BTreeMap::new();
        let mut candidates: Vec<_> = movable
            .iter()
            .map(|&i| {
                let mut component: Vec<_> = components[i].iter().map(|&n| keys[n]).collect();
                component.sort_by_key(|key| (graph.file_path(key.file_id), key.segment_idx));
                component
            })
            .collect();
        candidates.sort_by_key(|c| (graph.file_path(c[0].file_id), c[0].segment_idx));
        for component in &candidates {
            let mut reached = BTreeSet::new();
            let mut demand = BTreeSet::new();
            let mut pending = component.clone();
            while let Some(node) = pending.pop() {
                if !reached.insert(node) {
                    continue;
                }
                demand.extend(public_users.get(&node).into_iter().flatten().copied());
                for &user in users.get(&node).into_iter().flatten() {
                    if transparent.contains(&user) {
                        pending.push(user);
                    } else {
                        demand.insert(seams[&user.file_id]);
                    }
                }
            }
            let unique = demand
                .first()
                .filter(|_| demand.len() == 1)
                .copied()
                .filter(|owner| {
                    !cjs.contains(*owner)
                        && !component.iter().any(|key| {
                            readonly_users
                                .get(key)
                                .is_some_and(|users| users.contains(owner))
                        })
                });
            let owner = if let Some(owner) = unique {
                Owner::Evaluation(owner)
            } else {
                *shared
                    .entry(demand)
                    .or_insert(Owner::Bindings(component[0]))
            };
            for &key in component {
                owners.insert(key, owner);
            }
        }
        // Hoisting a function must not activate a later namespace/initializer
        // through its new chunk's transitive static imports.
        let mut anchored = BTreeSet::new();
        loop {
            let mut rejected = Vec::new();
            for component in &candidates {
                let origin = Owner::Evaluation(seams[&component[0].file_id]);
                if owners[&component[0]] == origin
                    || !component
                        .iter()
                        .any(|key| phase_bound.contains(&key.file_id))
                {
                    continue;
                }
                let mut pending = vec![owners[&component[0]]];
                let mut seen = BTreeSet::new();
                let mut activates = false;
                while let Some(owner) = pending.pop() {
                    if !seen.insert(owner) {
                        continue;
                    }
                    if let Owner::Evaluation(_) = owner {
                        activates |= owner != origin;
                        continue;
                    }
                    for (&key, &assigned) in &owners {
                        if assigned != owner {
                            continue;
                        }
                        let module = &modules[&key.file_id];
                        let span = module.parsed.segments[key.segment_idx as usize].span;
                        for reference in module
                            .parsed
                            .facts
                            .references
                            .iter()
                            .filter(|r| contains(span, r.span))
                        {
                            let Some(binding) = reference.binding else {
                                continue;
                            };
                            let identity =
                                if symbols.definitions.contains_key(&(key.file_id, binding)) {
                                    ExportIdentity::Binding(key.file_id, binding)
                                } else if let Some(linked) =
                                    symbols.imports.get(&(key.file_id, binding))
                                {
                                    linked.identity.clone()
                                } else {
                                    continue;
                                };
                            if let Some(target) = declaration(graph, &identity)? {
                                pending.push(owners[&target]);
                            } else if let ExportIdentity::Namespace(file) = identity {
                                activates |=
                                    cjs.contains(file) || Owner::Evaluation(seams[&file]) != origin;
                            } else if matches!(identity, ExportIdentity::CommonJs(..)) {
                                activates = true;
                            }
                        }
                    }
                }
                if activates {
                    rejected.push(component);
                }
            }
            if rejected.is_empty() {
                break;
            }
            for component in rejected {
                anchored.insert(component[0]);
                for &key in component {
                    owners.insert(key, Owner::Evaluation(seams[&key.file_id]));
                }
            }
        }
        // Source-name-sensitive syntax stays lexical. Verify each proposed
        // physical group before applying any ownership change.
        loop {
            let changed: BTreeSet<_> = owners.values().copied().collect();
            let mut rejected = BTreeSet::new();
            for owner in &changed {
                let names = local_names(graph, modules, symbols, &owners, *owner)?;
                let mut values = Values::new(graph, modules, symbols);
                values.names = names;
                values.requests = seams
                    .iter()
                    .map(|(&file, &owner)| (file, request(graph, owner)))
                    .collect();
                for (&symbol @ (file, binding), linked) in &symbols.imports {
                    if matches!(
                        linked.identity,
                        ExportIdentity::Namespace(_) | ExportIdentity::CommonJs(..)
                    ) {
                        let name = &modules[&file]
                            .parsed
                            .facts
                            .bindings
                            .iter()
                            .find(|b| b.id == binding)
                            .ok_or_eyre("missing namespace import binding")?
                            .name;
                        values.imports.insert(symbol, name.clone());
                    }
                }
                let group_live: BTreeSet<_> = owners
                    .iter()
                    .filter(|(_, o)| *o == owner)
                    .map(|(&k, _)| k)
                    .collect();
                for file in group_live
                    .iter()
                    .map(|k| k.file_id)
                    .collect::<BTreeSet<_>>()
                {
                    if cjs.contains(file) {
                        continue;
                    }
                    if body(
                        graph,
                        file,
                        &modules[&file],
                        &values,
                        &group_live,
                        &BTreeMap::new(),
                    )?
                    .is_none()
                    {
                        rejected.insert(*owner);
                    }
                }
            }
            if rejected.is_empty() {
                return Ok(Self { owners });
            }
            let mut progress = false;
            for component in &candidates {
                if !anchored.contains(&component[0]) && rejected.contains(&owners[&component[0]]) {
                    let owner = Owner::Bindings(component[0]);
                    for key in component {
                        progress |= owners[key] != owner;
                        owners.insert(*key, owner);
                    }
                }
            }
            eyre::ensure!(progress, "incompatible native lexical placement");
        }
    }
}

#[derive(Default)]
struct Functions(Vec<Span>);

impl<'a> Visit<'a> for Functions {
    fn visit_function(&mut self, function: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        self.0.push(function.span);
        walk::walk_function(self, function, flags);
    }
    fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
        self.0.push(function.span);
        walk::walk_arrow_function_expression(self, function);
    }
}

fn declaration(graph: &SourceGraph, identity: &ExportIdentity) -> eyre::Result<Option<SegmentKey>> {
    Ok(match identity {
        ExportIdentity::Binding(file, binding) => {
            let facts = graph.file_facts(*file).ok_or_eyre("missing symbol facts")?;
            let binding = facts
                .bindings
                .iter()
                .find(|b| b.id == *binding)
                .ok_or_eyre("missing symbol binding")?;
            let span = *binding
                .declarations
                .first()
                .ok_or_eyre("missing symbol declaration")?;
            let segments = graph
                .file_segments(*file)
                .ok_or_eyre("missing symbol segments")?;
            Some(SegmentKey::new(
                *file,
                segments.partition_point(|s| s.span.end <= span.start) as u32,
            ))
        }
        ExportIdentity::Expression(key) => Some(*key),
        _ => None,
    })
}

fn local_names(
    graph: &SourceGraph,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    owners: &BTreeMap<SegmentKey, Owner>,
    owner: Owner,
) -> eyre::Result<HashMap<crate::symbols::Symbol, String>> {
    let files: BTreeSet<_> = owners
        .iter()
        .filter_map(|(key, &assigned)| (assigned == owner).then_some(key.file_id))
        .collect();
    let mut occupied: BTreeSet<_> = files
        .iter()
        .flat_map(|file| {
            modules[file]
                .parsed
                .facts
                .references
                .iter()
                .filter(|r| r.binding.is_none())
                .map(|r| r.name.clone())
        })
        .collect();
    let mut definitions = Vec::new();
    for &(file, binding) in symbols.definitions.keys() {
        if declaration(graph, &ExportIdentity::Binding(file, binding))?
            .is_some_and(|key| owners.get(&key) == Some(&owner))
        {
            definitions.push((file, binding));
        } else if modules[&file]
            .parsed
            .facts
            .references
            .iter()
            .any(|reference| {
                if reference.binding != Some(binding) {
                    return false;
                }
                let index = modules[&file]
                    .parsed
                    .segments
                    .partition_point(|segment| segment.span.end <= reference.span.start);
                owners.get(&SegmentKey::new(file, index as u32)) == Some(&owner)
            })
        {
            occupied.insert(symbols.definitions[&(file, binding)].name.clone());
        }
    }
    definitions.sort_by_key(|&(file, binding)| (graph.file_path(file), binding.0));
    let mut names = HashMap::new();
    for symbol @ (file, binding) in definitions {
        let original = modules[&file]
            .parsed
            .facts
            .bindings
            .iter()
            .find(|b| b.id == binding)
            .ok_or_eyre("missing relocated symbol binding")?;
        let captured = files.iter().any(|&other| {
            other != file
                && modules[&other]
                    .parsed
                    .facts
                    .bindings
                    .iter()
                    .any(|b| !b.top_level && b.name == original.name)
        });
        let name = if !captured && occupied.insert(original.name.clone()) {
            original.name.clone()
        } else {
            let name = symbols.definitions[&symbol].name.clone();
            occupied.insert(name.clone());
            name
        };
        names.insert(symbol, name);
    }
    Ok(names)
}

#[expect(clippy::too_many_arguments)]
pub(super) fn emit(
    graph: &SourceGraph,
    root: Owner,
    group: &BTreeSet<u32>,
    seams: &BTreeMap<u32, u32>,
    redirects: &BTreeMap<u32, u32>,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    placement: &Placement,
    surfaces: &BTreeSet<u32>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
    cjs: &commonjs::CommonJs,
) -> eyre::Result<String> {
    let live: BTreeSet<_> = placement
        .owners
        .iter()
        .filter(|(_, &o)| o == root)
        .map(|(&k, _)| k)
        .collect();
    let mut values = Values::new(graph, modules, symbols);
    values.names = local_names(graph, modules, symbols, &placement.owners, root)?;
    values.requests = seams
        .iter()
        .map(|(&f, &o)| (f, request(graph, o)))
        .collect();
    for (&file, module) in modules {
        for dependency in module.parsed.facts.dependencies.iter().filter(|d| {
            !d.type_only
                && d.kind == DependencyKind::DynamicImport
                && live.iter().any(|key| {
                    key.file_id == file
                        && contains(
                            module.parsed.segments[key.segment_idx as usize].span,
                            d.span,
                        )
                })
        }) {
            let target = target_file(graph, file, dependency)?;
            if cjs.contains(target) {
                values.dynamic.insert(target, cjs.dynamic(graph, target)?);
            }
        }
    }
    let mut output = String::new();
    if let Owner::Evaluation(file) = root {
        if let Some(name) = values.defaults.iter().find_map(|(key, name)| {
            (key.file_id == file && placement.owners.get(key) == Some(&root)).then_some(name)
        }) {
            output.push_str(&format!(
                "import {{default as {name}}} from {};\n",
                root.request(graph)
            ));
            values.native_defaults.insert(file);
        }
    }
    let mut order = Vec::new();
    let mut pending = match root {
        Owner::Evaluation(file) => vec![(file, false)],
        Owner::Bindings(_) => Vec::new(),
    };
    let mut seen = BTreeSet::new();
    while let Some((file, finish)) = pending.pop() {
        if finish {
            order.push(file);
            continue;
        }
        if !seen.insert(file) {
            continue;
        }
        if !group.contains(&file) {
            if cjs.contains(file) {
                let consumer = match root {
                    Owner::Evaluation(file) => file,
                    Owner::Bindings(_) => unreachable!(),
                };
                output.push_str(&cjs.evaluate(graph, consumer, file)?);
            } else {
                output.push_str(&format!("import {};\n", request(graph, seams[&file])));
            }
            continue;
        }
        pending.push((file, true));
        for dependency in modules[&file]
            .parsed
            .facts
            .dependencies
            .iter()
            .rev()
            .filter(|d| {
                !d.type_only && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
            })
        {
            pending.push((redirects[&target_file(graph, file, dependency)?], false));
        }
    }
    let mut relocated: Vec<_> = live
        .iter()
        .map(|k| k.file_id)
        .filter(|f| !group.contains(f))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    relocated.sort_by_key(|&f| graph.file_path(f));
    relocated.extend(order);
    let mut ordered = Vec::new();
    let owned_files: BTreeSet<_> = relocated.iter().copied().collect();
    let mut visited = BTreeSet::new();
    for &file in &relocated {
        let mut pending = vec![(file, false)];
        while let Some((file, finish)) = pending.pop() {
            if finish {
                ordered.push(file);
                continue;
            }
            if !owned_files.contains(&file) || !visited.insert(file) {
                continue;
            }
            pending.push((file, true));
            for dependency in modules[&file]
                .parsed
                .facts
                .dependencies
                .iter()
                .rev()
                .filter(|d| {
                    !d.type_only
                        && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
                })
            {
                pending.push((target_file(graph, file, dependency)?, false));
            }
        }
    }
    relocated = ordered;
    let mut imported = BTreeSet::new();
    let mut occupied: BTreeSet<_> = values
        .names
        .values()
        .chain(values.defaults.values())
        .cloned()
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
    occupied.extend(symbols.definitions.values().map(|d| d.name.clone()));
    for &file in &relocated {
        for binding in modules[&file]
            .parsed
            .facts
            .bindings
            .iter()
            .filter(|b| b.top_level)
        {
            if !modules[&file].parsed.facts.references.iter().any(|r| {
                r.binding == Some(binding.id)
                    && live.iter().filter(|k| k.file_id == file).any(|k| {
                        contains(
                            modules[&file].parsed.segments[k.segment_idx as usize].span,
                            r.span,
                        )
                    })
            }) {
                continue;
            }
            let identity = if symbols.definitions.contains_key(&(file, binding.id)) {
                ExportIdentity::Binding(file, binding.id)
            } else if let Some(linked) = symbols.imports.get(&(file, binding.id)) {
                linked.identity.clone()
            } else {
                continue;
            };
            let owner = declaration(graph, &identity)?.map(|key| placement.owners[&key]);
            let readonly = binding.import
                && modules[&file]
                    .parsed
                    .facts
                    .references
                    .iter()
                    .any(|r| r.binding == Some(binding.id) && r.write);
            if owner == Some(root) && !readonly {
                continue;
            }
            let alias = symbols
                .definitions
                .get(&(file, binding.id))
                .map(|d| d.name.clone())
                .unwrap_or_else(|| {
                    let mut alias = format!(
                        "__gf_segment_import_{}_{}",
                        digest(graph.file_path(file).unwrap().to_str().unwrap()),
                        binding.id.0
                    );
                    while !occupied.insert(alias.clone()) {
                        alias.push('_');
                    }
                    alias
                });
            if imported.insert(alias.clone()) {
                match &identity {
                    ExportIdentity::Namespace(target) if cjs.contains(*target) => {
                        output.push_str(&cjs.import(
                            graph,
                            *target,
                            &ImportedName::Namespace,
                            &alias,
                        )?);
                    }
                    ExportIdentity::Namespace(target) => output.push_str(&format!(
                        "import * as {alias} from {};\n",
                        request(graph, seams[target])
                    )),
                    ExportIdentity::CommonJs(target, name) => {
                        output.push_str(&cjs.import(
                            graph,
                            *target,
                            &ImportedName::Name(name.clone()),
                            &alias,
                        )?);
                    }
                    _ => {
                        let owner = owner.ok_or_eyre("missing lexical segment owner")?;
                        output.push_str(&format!(
                            "import {{{} as {alias}}} from {};\n",
                            quote(&address(graph, &identity, owner, surfaces, &values)?),
                            owner.request(graph)
                        ));
                    }
                }
            }
            values.imports.insert((file, binding.id), alias);
        }
    }
    let mut bodies = String::new();
    for file in relocated {
        let (hoisted, body) = body(graph, file, &modules[&file], &values, &live, recipes)?
            .ok_or_eyre("relocated lexical group changed compatibility")?;
        output.push_str(&hoisted);
        bodies.push_str(&body);
        bodies.push_str("\n;\n");
    }
    output.push_str(&bodies);
    let mut exported = BTreeSet::new();
    let public_exports = match root {
        Owner::Evaluation(file) => symbols.exports.get(&file),
        Owner::Bindings(_) => None,
    };
    let public = matches!(root, Owner::Evaluation(file) if surfaces.contains(&file));
    for (public_name, linked) in public_exports.into_iter().flatten() {
        if public_name == "default"
            && matches!(linked.identity, ExportIdentity::Expression(key) if values.native_defaults.contains(&key.file_id))
        {
            exported.insert(public_name.clone());
            continue;
        }
        if let ExportIdentity::Namespace(file) = linked.identity {
            if public {
                if cjs.contains(file) {
                    output.push_str(&cjs.reexport(
                        graph,
                        file,
                        &ImportedName::Namespace,
                        public_name,
                    )?);
                } else {
                    output.push_str(&format!(
                        "export * as {} from {};\n",
                        quote(public_name),
                        request(graph, seams[&file])
                    ));
                }
                exported.insert(public_name.clone());
            }
            continue;
        }
        if let ExportIdentity::CommonJs(file, ref name) = linked.identity {
            if public {
                output.push_str(&cjs.reexport(
                    graph,
                    file,
                    &ImportedName::Name(name.clone()),
                    public_name,
                )?);
                exported.insert(public_name.clone());
            }
            continue;
        }
        let key = declaration(graph, &linked.identity)?
            .ok_or_eyre("nonlexical relocated public export")?;
        let owner = placement.owners[&key];
        if owner == root {
            output.push_str(&format!(
                "export {{{} as {}}};\n",
                values
                    .name(&linked.identity)
                    .ok_or_eyre("missing relocated export")?,
                quote(public_name)
            ));
            exported.insert(public_name.clone());
        } else if public {
            output.push_str(&format!(
                "export {{{} as {}}} from {};\n",
                quote(&address(graph, &linked.identity, owner, surfaces, &values)?),
                quote(public_name),
                owner.request(graph)
            ));
            exported.insert(public_name.clone());
        }
    }
    if !public {
        let mut definitions: Vec<_> = symbols.definitions.iter().collect();
        definitions.sort_by_key(|(&(file, binding), _)| (graph.file_path(file), binding.0));
        for (&symbol @ (file, binding), definition) in definitions {
            let identity = ExportIdentity::Binding(file, binding);
            if declaration(graph, &identity)?.is_some_and(|k| placement.owners[&k] == root) {
                let address = address(graph, &identity, root, surfaces, &values)?;
                if !exported.insert(address.clone()) {
                    continue;
                }
                let name = values.names.get(&symbol).unwrap_or(&definition.name);
                output.push_str(&format!("export {{{name} as {}}};\n", quote(&address)));
            }
        }
        for (key, name) in &values.defaults {
            if placement.owners.get(key) == Some(&root) {
                let address = address(
                    graph,
                    &ExportIdentity::Expression(*key),
                    root,
                    surfaces,
                    &values,
                )?;
                if exported.insert(address.clone()) {
                    output.push_str(&format!("export {{{name} as {}}};\n", quote(&address)));
                }
            }
        }
    }
    Ok(output)
}

fn address(
    graph: &SourceGraph,
    identity: &ExportIdentity,
    owner: Owner,
    surfaces: &BTreeSet<u32>,
    values: &Values<'_>,
) -> eyre::Result<String> {
    let symbols = values.symbols;
    let original_exports = match owner {
        Owner::Evaluation(file) => symbols.exports.get(&file),
        Owner::Bindings(_) => None,
    };
    if let Some(exports) = original_exports {
        if let Some((name, _)) = exports.iter().find(|(_, e)| &e.identity == identity) {
            return Ok(name.clone());
        }
    }
    eyre::ensure!(
        !matches!(owner, Owner::Evaluation(file) if surfaces.contains(&file)),
        "missing boundary symbol export"
    );
    match identity {
        ExportIdentity::Binding(file, binding) => {
            let original = &symbols.definitions[&(*file, *binding)].name;
            if !original_exports.is_some_and(|exports| exports.contains_key(original)) {
                return Ok(original.clone());
            }
            let mut name = format!(
                "__gf_segment_binding_{}_{}",
                digest(graph.file_path(*file).unwrap().to_str().unwrap()),
                binding.0
            );
            while original_exports.is_some_and(|exports| exports.contains_key(&name))
                || symbols.definitions.values().any(|d| d.name == name)
                || values.defaults.values().any(|n| *n == name)
            {
                name.push('_');
            }
            Ok(name)
        }
        ExportIdentity::Expression(key) => values
            .defaults
            .get(key)
            .cloned()
            .ok_or_eyre("missing relocated expression address"),
        _ => eyre::bail!("unsupported nonlexical relocated address"),
    }
}
