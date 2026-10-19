//! Physical ownership for pure declarations, independent of source body seams.
use super::*;
use crate::RetentionEdgeKind;

pub(super) struct Placement {
    pub owners: BTreeMap<SegmentKey, u32>,
    pub changed: BTreeSet<u32>,
}

impl Placement {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
        symbols: &Symbols,
        seams: &BTreeMap<u32, u32>,
        protected: &BTreeSet<u32>,
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
        let mut transparent = BTreeSet::new();
        let mut movable = BTreeSet::new();
        for &key in &live {
            let module = &modules[&key.file_id];
            if protected.contains(&key.file_id) || cjs.contains(key.file_id) {
                continue;
            }
            let span = module.parsed.segments[key.segment_idx as usize].span;
            let facts = &module.parsed.facts;
            let definitions: Vec<_> = facts
                .bindings
                .iter()
                .filter(|b| {
                    b.top_level && !b.import && b.declarations.iter().any(|&s| contains(span, s))
                })
                .collect();
            if definitions.is_empty() {
                if !effects.contains(&key) {
                    transparent.insert(key);
                }
                continue;
            }
            if effects.contains(&key)
                || facts.dependencies.iter().any(|d| contains(span, d.span) && !d.type_only)
                || definitions.iter().any(|b| {
                    b.declarations.iter().any(|&s| !contains(span, s))
                        || facts.references.iter().any(|r| {
                            r.binding == Some(b.id) && (r.write || r.span.start < span.start)
                        })
                })
                // Writes through a relocated closure must never become writes
                // to a native import. Keep its defining lexical environment.
                || facts.references.iter().any(|r| r.write && contains(span, r.span))
            {
                continue;
            }
            movable.insert(key);
            transparent.insert(key);
        }
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
        let mut owners: BTreeMap<_, _> = live.iter().map(|&k| (k, seams[&k.file_id])).collect();
        for key in movable {
            let mut reached = BTreeSet::new();
            let mut demand = BTreeSet::new();
            let mut pending = vec![key];
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
            if let Some(&owner) = demand.first().filter(|_| demand.len() == 1) {
                if !cjs.contains(owner)
                    && !protected.contains(&owner)
                    && !readonly_users
                        .get(&key)
                        .is_some_and(|users| users.contains(&owner))
                {
                    owners.insert(key, owner);
                }
            }
        }
        // Source-name-sensitive syntax stays lexical. Verify each proposed
        // physical group before applying any ownership change.
        loop {
            let changed: BTreeSet<_> = owners
                .iter()
                .filter(|(k, owner)| **owner != seams[&k.file_id])
                .flat_map(|(k, &owner)| [seams[&k.file_id], owner])
                .collect();
            let mut rejected = BTreeSet::new();
            for owner in &changed {
                let names = local_names(graph, modules, symbols, &owners, *owner)?;
                let mut values = Values::new(graph, modules, symbols);
                values.names = names;
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
                return Ok(Self { owners, changed });
            }
            for (key, owner) in &mut owners {
                if rejected.contains(owner) || rejected.contains(&seams[&key.file_id]) {
                    *owner = seams[&key.file_id];
                }
            }
        }
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
    owners: &BTreeMap<SegmentKey, u32>,
    owner: u32,
) -> eyre::Result<HashMap<crate::symbols::Symbol, String>> {
    let mut occupied: BTreeSet<_> = modules
        .values()
        .flat_map(|m| {
            m.parsed
                .facts
                .bindings
                .iter()
                .filter(|b| !b.top_level)
                .map(|b| b.name.clone())
                .chain(
                    m.parsed
                        .facts
                        .references
                        .iter()
                        .filter(|r| r.binding.is_none())
                        .map(|r| r.name.clone()),
                )
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
        let name = if occupied.insert(original.name.clone()) {
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
    root: u32,
    group: &BTreeSet<u32>,
    seams: &BTreeMap<u32, u32>,
    redirects: &BTreeMap<u32, u32>,
    modules: &BTreeMap<u32, Module<'_>>,
    symbols: &Symbols,
    placement: &Placement,
    surfaces: &BTreeSet<u32>,
    recipes: &BTreeMap<SegmentKey, &[crate::effects::InvocationRewrite]>,
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
    let mut output = String::new();
    let mut order = Vec::new();
    let mut pending = vec![(root, false)];
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
            output.push_str(&format!("import {};\n", request(graph, seams[&file])));
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
    for &file in &relocated {
        output.push_str(&format!("import {};\n", request(graph, seams[&file])));
    }
    relocated.extend(order);
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
            let Some(key) = declaration(graph, &identity)? else {
                eyre::bail!("namespace/CJS values cannot cross a relocated lexical group");
            };
            let owner = placement.owners[&key];
            if owner == root {
                continue;
            }
            let exported = address(graph, &identity, owner, surfaces, &values)?;
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
                output.push_str(&format!(
                    "import {{{} as {alias}}} from {};\n",
                    quote(&exported),
                    request(graph, owner)
                ));
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
    for (public, linked) in &symbols.exports[&root] {
        let key = declaration(graph, &linked.identity)?
            .ok_or_eyre("nonlexical relocated public export")?;
        let owner = placement.owners[&key];
        if owner == root {
            output.push_str(&format!(
                "export {{{} as {}}};\n",
                values
                    .name(&linked.identity)
                    .ok_or_eyre("missing relocated export")?,
                quote(public)
            ));
            exported.insert(public.clone());
        } else if surfaces.contains(&root) {
            output.push_str(&format!(
                "export {{{} as {}}} from {};\n",
                quote(&address(graph, &linked.identity, owner, surfaces, &values)?),
                quote(public),
                request(graph, owner)
            ));
            exported.insert(public.clone());
        }
    }
    if !surfaces.contains(&root) {
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
    owner: u32,
    surfaces: &BTreeSet<u32>,
    values: &Values<'_>,
) -> eyre::Result<String> {
    let symbols = values.symbols;
    if let Some((name, _)) = symbols.exports[&owner]
        .iter()
        .find(|(_, e)| &e.identity == identity)
    {
        return Ok(name.clone());
    }
    eyre::ensure!(!surfaces.contains(&owner), "missing boundary symbol export");
    match identity {
        ExportIdentity::Binding(file, binding) => {
            let original = &symbols.definitions[&(*file, *binding)].name;
            if !symbols.exports[&owner].contains_key(original) {
                return Ok(original.clone());
            }
            let mut name = format!(
                "__gf_segment_binding_{}_{}",
                digest(graph.file_path(*file).unwrap().to_str().unwrap()),
                binding.0
            );
            while symbols.exports[&owner].contains_key(&name)
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
