//! Concrete links are derived only after segment ownership is fixed.
use super::*;
use segments::{Owner, Placement};

#[derive(Clone)]
pub(super) struct BindingConnection {
    pub symbol: crate::symbols::Symbol,
    identity: ExportIdentity,
    owner: Option<Owner>,
    pub alias: String,
}

/// Placement compatibility and emission must consume identical lexical targets.
pub(super) struct OwnerNames {
    pub locals: HashMap<crate::symbols::Symbol, String>,
    pub bindings: Vec<BindingConnection>,
}

pub(super) fn name_plan(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    modules: &BTreeMap<u32, Module<'_>>,
    prepared: &PreparedValues<'_>,
    owners: &BTreeMap<SegmentKey, Owner>,
    seams: &BTreeMap<u32, u32>,
    definitions: &BTreeMap<Owner, Vec<crate::symbols::Symbol>>,
) -> eyre::Result<BTreeMap<Owner, OwnerNames>> {
    let symbols = prepared.symbols;
    let defaults = Values::new(prepared);
    let mut generated = OccupiedNames::new(prepared);
    let mut references: BTreeMap<Owner, HashMap<crate::symbols::Symbol, bool>> = BTreeMap::new();
    let generated_references: BTreeSet<_> = plan
        .generated_boundaries
        .values()
        .map(|boundary| {
            let GraphNode::Segment(consumer) = graph.edges()[boundary.source_edge].from else {
                unreachable!()
            };
            (consumer.file_id, boundary.reference)
        })
        .collect();
    for (&file, module) in modules {
        for (index, reference) in module.parsed.facts().references().iter().enumerate() {
            if generated_references.contains(&(file, index as u32)) {
                continue;
            }
            let Some(binding) = reference.binding() else {
                continue;
            };
            let index = module
                .parsed
                .segments()
                .partition_point(|s| s.span.end <= reference.span().start);
            if let Some(&owner) = owners.get(&SegmentKey::new(file, index as u32)) {
                *references
                    .entry(owner)
                    .or_default()
                    .entry((file, binding))
                    .or_default() |= reference.write();
            }
        }
    }
    let mut result = BTreeMap::new();
    for owner in owners
        .values()
        .copied()
        .chain(seams.values().copied().map(Owner::Evaluation))
        .collect::<BTreeSet<_>>()
    {
        let locals = segments::local_names(
            modules,
            symbols,
            owners,
            owner,
            definitions
                .get(&owner)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )?;
        let mut occupied: BTreeSet<_> = locals.values().cloned().collect();
        occupied.extend(defaults.defaults.values().cloned());
        for &(file, binding) in references.get(&owner).into_iter().flatten().map(|(s, _)| s) {
            if let Some(definition) = symbols.definitions.get(&(file, binding)) {
                occupied.insert(definition.name.clone());
            }
        }
        let owner_files: BTreeSet<_> = owners
            .iter()
            .filter_map(|(key, &assigned)| (assigned == owner).then_some(key.file_id))
            .collect();
        let files: Vec<_> = prepared
            .files
            .iter()
            .copied()
            .filter(|file| owner_files.contains(file))
            .collect();
        for &file in &files {
            let facts = &modules[&file].parsed.facts();
            occupied.extend(facts.emission().free_references().iter().map(|index| {
                facts
                    .references()
                    .get(index as usize)
                    .unwrap()
                    .name()
                    .to_owned()
            }));
        }
        let mut bindings = Vec::new();
        for file in files {
            for binding in modules[&file]
                .parsed
                .facts()
                .bindings()
                .iter()
                .filter(|b| b.top_level())
            {
                let Some(&written) = references
                    .get(&owner)
                    .and_then(|references| references.get(&(file, binding.id())))
                else {
                    continue;
                };
                let identity = if symbols.definitions.contains_key(&(file, binding.id())) {
                    ExportIdentity::Binding(file, binding.id())
                } else if let Some(linked) = symbols.imports.get(&(file, binding.id())) {
                    linked.identity.clone()
                } else {
                    continue;
                };
                let target = segments::declaration(graph, &identity)?.map(|key| owners[&key]);
                // Writes through source imports must remain native import writes.
                if target == Some(owner) && !(binding.import() && written) {
                    continue;
                }
                let alias = if let Some(definition) = symbols.definitions.get(&(file, binding.id()))
                {
                    definition.name.clone()
                } else if occupied.insert(binding.name().to_owned()) {
                    binding.name().to_owned()
                } else {
                    let mut alias = format!(
                        "__gf_segment_import_{}_{}",
                        digest(graph.file_path(file).unwrap().to_str().unwrap()),
                        binding.id().0
                    );
                    while !generated.insert(alias.clone()) {
                        alias.push('_');
                    }
                    alias
                };
                bindings.push(BindingConnection {
                    symbol: (file, binding.id()),
                    identity,
                    owner: target,
                    alias,
                });
            }
        }
        result.insert(owner, OwnerNames { locals, bindings });
    }
    Ok(result)
}

pub(super) struct NativeRequest {
    pub code: Code,
    pub whole: bool,
}

pub(super) struct Connections {
    pub names: BTreeMap<Owner, HashMap<crate::symbols::Symbol, String>>,
    pub bindings: BTreeMap<Owner, Vec<BindingConnection>>,
    pub imports: BTreeMap<Owner, Code>,
    pub exports: BTreeMap<Owner, Code>,
    pub evaluation: BTreeMap<Owner, Code>,
    pub namespaces: BTreeMap<u32, PathBuf>,
    pub projections: BTreeMap<PathBuf, Code>,
    pub async_facades: BTreeMap<AsyncSiteId, PathBuf>,
    pub native_exports: BTreeMap<SegmentKey, Code>,
    pub native_requests: HashMap<(u32, ast_segmenter::owned::DependencyId), NativeRequest>,
    addresses: HashMap<ExportIdentity, String>,
    demanded: BTreeMap<Owner, Vec<ExportIdentity>>,
}

impl Connections {
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        graph: &SourceGraph,
        modules: &BTreeMap<u32, Module<'_>>,
        prepared: &PreparedValues<'_>,
        seams: &BTreeMap<u32, u32>,
        placement: &Placement,
        surfaces: &BTreeSet<u32>,
        cjs: &commonjs::CommonJs,
        activations: Option<&Activations>,
        plan: &ChunkPlan,
    ) -> eyre::Result<Self> {
        let symbols = prepared.symbols;
        let mut result = Self {
            names: placement
                .names
                .iter()
                .map(|(&owner, names)| (owner, names.locals.clone()))
                .collect(),
            bindings: placement
                .names
                .iter()
                .map(|(&owner, names)| (owner, names.bindings.clone()))
                .collect(),
            imports: BTreeMap::new(),
            exports: BTreeMap::new(),
            evaluation: BTreeMap::new(),
            namespaces: seams
                .iter()
                .map(|(&file, &seam)| (file, path(graph, seam)))
                .collect(),
            projections: BTreeMap::new(),
            async_facades: BTreeMap::new(),
            native_exports: BTreeMap::new(),
            native_requests: HashMap::new(),
            addresses: HashMap::new(),
            demanded: BTreeMap::new(),
        };
        if let Some(activations) = activations {
            for (&owner, bindings) in &mut result.bindings {
                for binding in bindings {
                    if let ExportIdentity::Namespace(target) = binding.identity {
                        if let Some(initialization) = activations.initializers.get(&target) {
                            if owner == Owner::Evaluation(activations.owners[&target]) {
                                binding.alias = initialization.namespace.clone();
                            }
                        }
                    }
                }
            }
        }
        // Ordered evaluation skeleton: after actual binding/phase/namespace links
        // are assembled, transparent private connectors are reduced over that
        // typed graph; final activation validation checks the reduced output.
        for file in seams.values().copied().collect::<BTreeSet<_>>() {
            if cjs.contains(file) || activations.is_some_and(|a| a.initializers.contains_key(&file))
            {
                continue;
            }
            let requests = result
                .evaluation
                .entry(Owner::Evaluation(file))
                .or_default();
            for dependency in modules[&file]
                .parsed
                .facts()
                .dependencies()
                .iter()
                .filter(|d| {
                    !d.type_only()
                        && matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport)
                })
            {
                if let Some(DependencyTarget::External(specifier)) =
                    graph.dependency_target(file, dependency.id())
                {
                    requests.push(
                        Code::external(specifier.clone(), RequestKind::Import)
                            .surrounded("import ", ";\n"),
                    );
                    continue;
                }
                let target = target_file(graph, file, dependency)?;
                if cjs.contains(target) {
                    requests.push(cjs.evaluate(graph, file, target)?);
                } else if seams[&target] != file {
                    requests.push(Code::evaluation(path(graph, seams[&target])));
                }
            }
        }
        result.plan_async_projections(graph, plan, symbols, prepared, placement, cjs)?;
        // Unknown native export-star/name validation is not a canonical lexical
        // link. Placement keeps these jobs intact; cached native recipes retain
        // their typed external requests and original linking clauses.
        if modules.keys().any(|&file| has_externals(graph, file)) {
            result.plan_pruned_exports(graph, modules, symbols, prepared, placement, cjs)?;
            result.plan_native_requests(graph, modules, symbols, placement, cjs)?;
            return Ok(result);
        }
        let defaults = Values::new(prepared);
        for boundary in plan.generated_boundaries.values() {
            let binding = modules[&boundary.target.file_id]
                .parsed
                .facts()
                .emission()
                .segments()
                .get(boundary.target.segment_idx as usize)
                .unwrap()
                .placement()
                .declarations()
                .get(0)
                .unwrap();
            let identity = ExportIdentity::Binding(boundary.target.file_id, binding);
            let GraphNode::Segment(consumer) = graph.edges()[boundary.source_edge].from else {
                unreachable!()
            };
            let owner = placement.owners[&boundary.target];
            if placement.owners[&consumer] != owner {
                result.demand(graph, symbols, &defaults, owner, &identity)?;
            }
        }
        let owners: BTreeSet<_> = placement
            .owners
            .values()
            .copied()
            .chain(seams.values().copied().map(Owner::Evaluation))
            .collect();
        for owner in owners {
            let mut phases: Vec<_> = placement
                .phase_dependencies
                .get(&owner)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            phases.sort_by_key(|owner| owner.path(graph));
            for dependency in phases {
                result
                    .evaluation
                    .entry(owner)
                    .or_default()
                    .push(Code::evaluation(dependency.path(graph)));
            }
            let demands: Vec<_> = result.bindings[&owner]
                .iter()
                .filter_map(|binding| {
                    binding
                        .owner
                        .map(|target| (target, binding.identity.clone()))
                })
                .collect();
            for (target, identity) in demands {
                result.demand(graph, symbols, &defaults, target, &identity)?;
            }
        }
        // Public projections demand canonical values just like lexical users;
        // their name maps never include the private addresses demanded above.
        for &file in surfaces {
            if cjs.contains(file) {
                continue;
            }
            if let Some(activations) = activations {
                for &file in activations.initializers.keys() {
                    for linked in symbols.exports[&file].values() {
                        if let Some(key) = segments::declaration(graph, &linked.identity)? {
                            let owner = placement.owners[&key];
                            if owner != Owner::Evaluation(activations.owners[&file]) {
                                result.demand(
                                    graph,
                                    symbols,
                                    &defaults,
                                    owner,
                                    &linked.identity,
                                )?;
                            }
                        }
                    }
                }
            }
            for linked in symbols.exports[&file].values() {
                if let Some(key) = segments::declaration(graph, &linked.identity)? {
                    let owner = placement.owners[&key];
                    if owner != Owner::Evaluation(seams[&file]) {
                        result.demand(graph, symbols, &defaults, owner, &linked.identity)?;
                    }
                }
            }
        }
        for &file in surfaces {
            let owner = Owner::Evaluation(seams[&file]);
            let private = result
                .demanded
                .get(&owner)
                .into_iter()
                .flatten()
                .any(|identity| {
                    !symbols.exports[&file]
                        .values()
                        .any(|linked| &linked.identity == identity)
                });
            if !private || cjs.contains(file) {
                continue;
            }
            let route = PathBuf::from(format!(
                "native/namespace-{}.mjs",
                digest(graph.file_path(file).unwrap().to_str().unwrap())
            ));
            let mut output = Code::evaluation(owner.path(graph));
            for name in symbols.exports[&file].keys() {
                output.push(
                    Code::request(owner.path(graph), RequestKind::ReExport).surrounded(
                        format!("export {{{} as {}}} from ", quote(name), quote(name)),
                        ";\n",
                    ),
                );
            }
            output.default_export = symbols.exports[&file].contains_key("default");
            result.namespaces.insert(file, route.clone());
            result.projections.insert(route, output);
        }
        for (site, activation) in &plan.async_activations {
            if !selected_activation(graph, plan, site) {
                if let GraphNode::Module(target) = graph.edges()[activation.source_edge].to {
                    result.async_facades.insert(
                        site.clone(),
                        cjs.namespaces
                            .get(graph.file_path(target).unwrap())
                            .unwrap_or(&result.namespaces[&target])
                            .clone(),
                    );
                }
            }
        }
        for (&owner, bindings) in &result.bindings {
            let mut output = Code::default();
            let mut imported = BTreeSet::new();
            for binding in bindings {
                if !imported.insert(&binding.alias) {
                    continue;
                }
                let alias = &binding.alias;
                output.push(match &binding.identity {
                    ExportIdentity::Namespace(file) if cjs.contains(*file) => {
                        cjs.import(graph, *file, &ImportedName::Namespace, alias)?
                    }
                    ExportIdentity::Namespace(file)
                        if activations.is_some_and(|a| a.initializers.contains_key(file)) =>
                    {
                        let activations = activations.unwrap();
                        if owner == Owner::Evaluation(activations.owners[file]) {
                            Code::default()
                        } else {
                            let namespace = &activations.initializers[file].namespace;
                            Code::request(
                                path(graph, activations.owners[file]),
                                RequestKind::Import,
                            )
                            .surrounded(format!("import {{{namespace} as {alias}}} from "), ";\n")
                        }
                    }
                    ExportIdentity::Namespace(file) => {
                        Code::request(result.namespaces[file].clone(), RequestKind::Import)
                            .surrounded(format!("import * as {alias} from "), ";\n")
                    }
                    ExportIdentity::CommonJs(file, name) => {
                        cjs.import(graph, *file, &ImportedName::Name(name.clone()), alias)?
                    }
                    identity => Code::request(
                        binding
                            .owner
                            .ok_or_eyre("missing connection owner")?
                            .path(graph),
                        RequestKind::Import,
                    )
                    .surrounded(
                        format!(
                            "import {{{} as {alias}}} from ",
                            quote(result.address(identity)?)
                        ),
                        ";\n",
                    ),
                });
            }
            result.imports.insert(owner, output);
        }
        for owner in placement
            .owners
            .values()
            .copied()
            .chain(seams.values().copied().map(Owner::Evaluation))
            .collect::<BTreeSet<_>>()
        {
            if matches!(owner, Owner::Evaluation(file) if cjs.contains(file)) {
                continue;
            }
            let mut values = Values::new(prepared);
            values.names = result.names[&owner].clone();
            let mut output = Code::default();
            let mut exported = BTreeSet::new();
            if let Owner::Evaluation(file) = owner {
                for (name, linked) in &symbols.exports[&file] {
                    if let Some(key) = segments::declaration(graph, &linked.identity)? {
                        if placement.owners[&key] == owner {
                            // The source default-expression recipe already exports
                            // its native snapshot; importing that name is nonexecuting.
                            if name != "default"
                                || !matches!(linked.identity, ExportIdentity::Expression(_))
                            {
                                output.push_str(&format!(
                                    "export {{{} as {}}};\n",
                                    values
                                        .name(&linked.identity)
                                        .ok_or_eyre("missing local public value")?,
                                    quote(name)
                                ));
                            }
                            exported.insert(name.clone());
                        } else if surfaces.contains(&file) {
                            output.push(
                                Code::request(
                                    placement.owners[&key].path(graph),
                                    RequestKind::ReExport,
                                )
                                .surrounded(
                                    format!(
                                        "export {{{} as {}}} from ",
                                        quote(result.address(&linked.identity)?),
                                        quote(name)
                                    ),
                                    ";\n",
                                ),
                            );
                            exported.insert(name.clone());
                        }
                    } else if surfaces.contains(&file) {
                        output.push(match &linked.identity {
                            ExportIdentity::Namespace(target) if cjs.contains(*target) => {
                                cjs.reexport(graph, *target, &ImportedName::Namespace, name)?
                            }
                            ExportIdentity::Namespace(target) => Code::request(
                                result.namespaces[target].clone(),
                                RequestKind::ReExport,
                            )
                            .surrounded(format!("export * as {} from ", quote(name)), ";\n"),
                            ExportIdentity::CommonJs(target, imported) => cjs.reexport(
                                graph,
                                *target,
                                &ImportedName::Name(imported.clone()),
                                name,
                            )?,
                            _ => eyre::bail!("unsupported public connection"),
                        });
                        exported.insert(name.clone());
                    }
                }
            }
            for identity in result.demanded.get(&owner).into_iter().flatten() {
                let address = result.address(identity)?;
                if exported.insert(address.to_owned()) {
                    output.push_str(&format!(
                        "export {{{} as {}}};\n",
                        values.name(identity).ok_or_eyre("missing internal value")?,
                        quote(address)
                    ));
                }
            }
            output.default_export = exported.contains("default");
            result.exports.insert(owner, output);
        }
        result.plan_native_requests(graph, modules, symbols, placement, cjs)?;
        Ok(result)
    }

    fn plan_native_requests(
        &mut self,
        graph: &SourceGraph,
        modules: &BTreeMap<u32, Module<'_>>,
        symbols: &Symbols,
        placement: &Placement,
        cjs: &commonjs::CommonJs,
    ) -> eyre::Result<()> {
        for (&file, module) in modules {
            if cjs.contains(file) {
                continue;
            }
            for dependency in module
                .parsed
                .facts()
                .dependencies()
                .iter()
                .filter(|d| !d.type_only() && d.pattern().is_none())
            {
                let index = module
                    .parsed
                    .segments()
                    .partition_point(|s| s.span.end <= dependency.span().start);
                if !placement
                    .owners
                    .contains_key(&SegmentKey::new(file, index as u32))
                {
                    continue;
                }
                let kind = match dependency.kind() {
                    DependencyKind::Import => RequestKind::Import,
                    DependencyKind::ReExport => RequestKind::ReExport,
                    DependencyKind::DynamicImport => RequestKind::DynamicImport,
                    DependencyKind::Require => RequestKind::Require,
                    DependencyKind::ImportType => {
                        eyre::bail!("type request in executable JavaScript")
                    }
                };
                let request = if let Some(DependencyTarget::External(specifier)) =
                    graph.dependency_target(file, dependency.id())
                {
                    NativeRequest {
                        code: Code::external(specifier.clone(), kind),
                        whole: false,
                    }
                } else {
                    let target = target_file(graph, file, dependency)?;
                    if cjs.contains(target) {
                        NativeRequest {
                            code: if dependency.kind() == DependencyKind::DynamicImport {
                                cjs.dynamic(graph, target)?
                            } else {
                                cjs.static_dependency(
                                    graph,
                                    file,
                                    target,
                                    dependency,
                                    module.parsed.facts(),
                                    symbols,
                                )?
                            },
                            whole: true,
                        }
                    } else if dependency.kind() == DependencyKind::Import
                        && module.parsed.facts().imports().iter().any(|import| {
                            import.dependency() == dependency.id()
                                && !symbols.imports.contains_key(&(file, import.binding()))
                        })
                    {
                        let facts = module.parsed.facts();
                        let mut code = Code::evaluation(self.namespaces[&target].clone());
                        for import in facts.imports().iter().filter(|import| {
                            import.dependency() == dependency.id()
                                && symbols.imports.contains_key(&(file, import.binding()))
                        }) {
                            let binding = facts
                                .bindings()
                                .iter()
                                .find(|binding| binding.id() == import.binding())
                                .ok_or_eyre("missing native import binding")?;
                            let clause = match import.imported().to_owned() {
                                ImportedName::Namespace => format!("* as {}", binding.name()),
                                ImportedName::Name(name) => {
                                    format!("{{{} as {}}}", quote(&name), binding.name())
                                }
                            };
                            code.push(
                                Code::request(self.namespaces[&target].clone(), kind)
                                    .surrounded(format!("import {clause} from "), ";\n"),
                            );
                        }
                        NativeRequest { code, whole: true }
                    } else {
                        NativeRequest {
                            code: Code::request(
                                if dependency.kind() == DependencyKind::DynamicImport {
                                    self.async_facades
                                        .get(&AsyncSiteId {
                                            module: graph.file_path(file).unwrap().to_owned(),
                                            offset: dependency.id().0,
                                            candidate: None,
                                        })
                                        .unwrap_or(&self.namespaces[&target])
                                        .clone()
                                } else {
                                    self.namespaces[&target].clone()
                                },
                                kind,
                            ),
                            whole: false,
                        }
                    }
                };
                self.native_requests
                    .insert((file, dependency.id()), request);
            }
        }
        Ok(())
    }

    fn plan_async_projections(
        &mut self,
        graph: &SourceGraph,
        plan: &ChunkPlan,
        symbols: &Symbols,
        prepared: &PreparedValues<'_>,
        placement: &Placement,
        cjs: &commonjs::CommonJs,
    ) -> eyre::Result<()> {
        let defaults = Values::new(prepared);
        for (site, activation) in &plan.async_activations {
            let GraphNode::Module(target) = graph.edges()[activation.source_edge].to else {
                continue;
            };
            if !selected_activation(graph, plan, site) {
                self.async_facades.insert(
                    site.clone(),
                    cjs.namespaces
                        .get(graph.file_path(target).unwrap())
                        .unwrap_or(&self.namespaces[&target])
                        .clone(),
                );
                continue;
            }
            let route = PathBuf::from(format!(
                "native/async-{}-{}.mjs",
                digest(site.module.to_str().unwrap()),
                site.offset
            ));
            let mut output = Code::evaluation(self.namespaces[&target].clone());
            for root in &activation.roots {
                let RootId::AsyncExport { name, .. } = root else {
                    continue;
                };
                let linked = graph.resolve_export(target, name, source_graph::Namespace::Value)?;
                if let Some(key) = segments::declaration(graph, &linked.identity)? {
                    let owner = placement.owners[&key];
                    self.demand(graph, symbols, &defaults, owner, &linked.identity)?;
                    output.push(
                        Code::request(owner.path(graph), RequestKind::ReExport).surrounded(
                            format!(
                                "export {{{} as {}}} from ",
                                quote(self.address(&linked.identity)?),
                                quote(name)
                            ),
                            ";\n",
                        ),
                    );
                } else if let ExportIdentity::Namespace(file) = linked.identity {
                    output.push(
                        Code::request(self.namespaces[&file].clone(), RequestKind::ReExport)
                            .surrounded(format!("export * as {} from ", quote(name)), ";\n"),
                    );
                } else {
                    eyre::bail!("non-native selected dynamic export");
                }
                output.default_export |= name == "default";
            }
            self.async_facades.insert(site.clone(), route.clone());
            self.projections.insert(route, output);
        }
        Ok(())
    }

    fn plan_pruned_exports(
        &mut self,
        graph: &SourceGraph,
        modules: &BTreeMap<u32, Module<'_>>,
        symbols: &Symbols,
        prepared: &PreparedValues<'_>,
        placement: &Placement,
        cjs: &commonjs::CommonJs,
    ) -> eyre::Result<()> {
        let defaults = Values::new(prepared);
        for &key in placement.owners.keys() {
            let file = key.file_id;
            let module = &modules[&file];
            let facts = module.parsed.facts();
            let span = module
                .parsed
                .segments()
                .get(key.segment_idx as usize)
                .unwrap()
                .span;
            if !facts
                .emission()
                .segments()
                .get(key.segment_idx as usize)
                .unwrap()
                .connector_only()
            {
                continue;
            }
            let exports = facts
                .exports()
                .iter()
                .filter(|export| !export.type_only() && contains(span, export.span()))
                .collect::<Vec<_>>();
            if !exports.iter().any(|export| {
                if export
                    .name()
                    .is_some_and(|name| !symbols.exports[&file].contains_key(name))
                {
                    return true;
                }
                let ExportValue::Binding(binding) = export.value() else {
                    return false;
                };
                facts
                    .imports()
                    .iter()
                    .any(|import| import.binding() == binding)
                    && !symbols.imports.contains_key(&(file, binding))
            }) {
                continue;
            }
            let mut output = Code::default();
            for export in exports {
                if export
                    .name()
                    .is_some_and(|name| !symbols.exports[&file].contains_key(name))
                {
                    continue;
                }
                let name = export.name().ok_or_eyre("unnamed pruned local export")?;
                let linked = graph.resolve_export(file, name, source_graph::Namespace::Value)?;
                if let Some(declaration) = segments::declaration(graph, &linked.identity)? {
                    let owner = placement.owners[&declaration];
                    if owner == placement.owners[&key] {
                        let local = match linked.identity {
                            ExportIdentity::Binding(source, binding) => modules[&source]
                                .parsed
                                .facts()
                                .bindings()
                                .iter()
                                .find(|candidate| candidate.id() == binding)
                                .ok_or_eyre("missing original export binding")?
                                .name()
                                .to_owned(),
                            _ => defaults
                                .name(&linked.identity)
                                .ok_or_eyre("missing native export value")?
                                .to_owned(),
                        };
                        output.push_str(&format!("export {{{} as {}}};\n", local, quote(name),));
                    } else {
                        self.demand(graph, symbols, &defaults, owner, &linked.identity)?;
                        output.push(
                            Code::request(owner.path(graph), RequestKind::ReExport).surrounded(
                                format!(
                                    "export {{{} as {}}} from ",
                                    quote(self.address(&linked.identity)?),
                                    quote(name)
                                ),
                                ";\n",
                            ),
                        );
                    }
                } else {
                    output.push(match &linked.identity {
                        ExportIdentity::Namespace(target) if cjs.contains(*target) => {
                            cjs.reexport(graph, *target, &ImportedName::Namespace, name)?
                        }
                        ExportIdentity::Namespace(target) => {
                            Code::request(self.namespaces[target].clone(), RequestKind::ReExport)
                                .surrounded(format!("export * as {} from ", quote(name)), ";\n")
                        }
                        ExportIdentity::CommonJs(target, imported) => cjs.reexport(
                            graph,
                            *target,
                            &ImportedName::Name(imported.clone()),
                            name,
                        )?,
                        ExportIdentity::External(specifier, Some(imported)) => {
                            Code::external(specifier.clone(), RequestKind::ReExport).surrounded(
                                format!("export {{{} as {}}} from ", quote(imported), quote(name)),
                                ";\n",
                            )
                        }
                        ExportIdentity::External(specifier, None) => {
                            Code::external(specifier.clone(), RequestKind::ReExport)
                                .surrounded(format!("export * as {} from ", quote(name)), ";\n")
                        }
                        _ => eyre::bail!("unsupported pruned native export"),
                    });
                }
                output.default_export |= name == "default";
            }
            self.native_exports.insert(key, output);
        }
        Ok(())
    }

    fn demand(
        &mut self,
        graph: &SourceGraph,
        symbols: &Symbols,
        defaults: &Values<'_>,
        owner: Owner,
        identity: &ExportIdentity,
    ) -> eyre::Result<()> {
        if self.addresses.contains_key(identity) {
            return Ok(());
        }
        // An unchanged native job already supplies these names. New private
        // addresses are an internal transport surface, never a namespace API.
        let original = match owner {
            Owner::Evaluation(file) => symbols.exports.get(&file),
            Owner::Bindings(_) => None,
        };
        let public = original.and_then(|exports| {
            exports
                .iter()
                .find(|(_, linked)| &linked.identity == identity)
                .map(|(name, _)| name.clone())
        });
        let address = if let Some(name) = public {
            name
        } else {
            let mut name = match identity {
                ExportIdentity::Binding(file, binding) => format!(
                    "__gf_segment_binding_{}_{}",
                    digest(graph.file_path(*file).unwrap().to_str().unwrap()),
                    binding.0
                ),
                ExportIdentity::Expression(key) => defaults
                    .defaults
                    .get(key)
                    .cloned()
                    .ok_or_eyre("missing expression address")?,
                _ => eyre::bail!("nonlexical internal address"),
            };
            while original.is_some_and(|exports| exports.contains_key(&name)) {
                name.push('_');
            }
            name
        };
        self.addresses.insert(identity.clone(), address);
        self.demanded
            .entry(owner)
            .or_default()
            .push(identity.clone());
        Ok(())
    }

    pub(super) fn address(&self, identity: &ExportIdentity) -> eyre::Result<&str> {
        self.addresses
            .get(identity)
            .map(String::as_str)
            .ok_or_eyre("missing planned binding address")
    }
}
