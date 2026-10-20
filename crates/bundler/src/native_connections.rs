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
    modules: &BTreeMap<u32, Module<'_>>,
    prepared: &PreparedValues<'_>,
    owners: &BTreeMap<SegmentKey, Owner>,
    seams: &BTreeMap<u32, u32>,
) -> eyre::Result<BTreeMap<Owner, OwnerNames>> {
    let symbols = prepared.symbols;
    let defaults = Values::new(prepared);
    let mut generated = OccupiedNames::new(prepared);
    let mut references: BTreeMap<Owner, HashMap<crate::symbols::Symbol, bool>> = BTreeMap::new();
    for (&file, module) in modules {
        for reference in &module.parsed.facts.references {
            let Some(binding) = reference.binding else {
                continue;
            };
            let index = module
                .parsed
                .segments
                .partition_point(|s| s.span.end <= reference.span.start);
            if let Some(&owner) = owners.get(&SegmentKey::new(file, index as u32)) {
                *references
                    .entry(owner)
                    .or_default()
                    .entry((file, binding))
                    .or_default() |= reference.write;
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
        let locals = segments::local_names(graph, modules, symbols, owners, owner)?;
        let mut occupied: BTreeSet<_> = locals.values().cloned().collect();
        occupied.extend(defaults.defaults.values().cloned());
        for &(file, binding) in references.get(&owner).into_iter().flatten().map(|(s, _)| s) {
            if let Some(definition) = symbols.definitions.get(&(file, binding)) {
                occupied.insert(definition.name.clone());
            }
        }
        let mut files: Vec<_> = owners
            .iter()
            .filter_map(|(key, &assigned)| (assigned == owner).then_some(key.file_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        files.sort_by_key(|&file| graph.file_path(file));
        for &file in &files {
            let facts = &modules[&file].parsed.facts;
            occupied.extend(
                facts
                    .emission
                    .free_references
                    .iter()
                    .map(|&index| facts.references[index as usize].name.clone()),
            );
        }
        let mut bindings = Vec::new();
        for file in files {
            for binding in modules[&file]
                .parsed
                .facts
                .bindings
                .iter()
                .filter(|b| b.top_level)
            {
                let Some(&written) = references
                    .get(&owner)
                    .and_then(|references| references.get(&(file, binding.id)))
                else {
                    continue;
                };
                let identity = if symbols.definitions.contains_key(&(file, binding.id)) {
                    ExportIdentity::Binding(file, binding.id)
                } else if let Some(linked) = symbols.imports.get(&(file, binding.id)) {
                    linked.identity.clone()
                } else {
                    continue;
                };
                let target = segments::declaration(graph, &identity)?.map(|key| owners[&key]);
                // Writes through source imports must remain native import writes.
                if target == Some(owner) && !(binding.import && written) {
                    continue;
                }
                let alias = if let Some(definition) = symbols.definitions.get(&(file, binding.id)) {
                    definition.name.clone()
                } else if occupied.insert(binding.name.clone()) {
                    binding.name.clone()
                } else {
                    let mut alias = format!(
                        "__gf_segment_import_{}_{}",
                        digest(graph.file_path(file).unwrap().to_str().unwrap()),
                        binding.id.0
                    );
                    while !generated.insert(alias.clone()) {
                        alias.push('_');
                    }
                    alias
                };
                bindings.push(BindingConnection {
                    symbol: (file, binding.id),
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
            native_requests: HashMap::new(),
            addresses: HashMap::new(),
            demanded: BTreeMap::new(),
        };
        // Ordered evaluation skeleton: after actual binding/phase/namespace links
        // are assembled, transparent private connectors are reduced over that
        // typed graph; final activation validation checks the reduced output.
        for file in seams.values().copied().collect::<BTreeSet<_>>() {
            if cjs.contains(file) {
                continue;
            }
            let requests = result
                .evaluation
                .entry(Owner::Evaluation(file))
                .or_default();
            for dependency in modules[&file].parsed.facts.dependencies.iter().filter(|d| {
                !d.type_only && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
            }) {
                if let Some(DependencyTarget::External(specifier)) =
                    graph.dependency_target(file, dependency.id)
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
        // Unknown native export-star/name validation is not a canonical lexical
        // link. Placement keeps these jobs intact; cached native recipes retain
        // their typed external requests and original linking clauses.
        if modules.keys().any(|&file| has_externals(graph, file)) {
            result.plan_native_requests(graph, modules, symbols, placement, cjs)?;
            return Ok(result);
        }
        let defaults = Values::new(prepared);
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
                .facts
                .dependencies
                .iter()
                .filter(|d| !d.type_only && d.pattern.is_none())
            {
                let index = module
                    .parsed
                    .segments
                    .partition_point(|s| s.span.end <= dependency.span.start);
                if !placement
                    .owners
                    .contains_key(&SegmentKey::new(file, index as u32))
                {
                    continue;
                }
                let kind = match dependency.kind {
                    DependencyKind::Import => RequestKind::Import,
                    DependencyKind::ReExport => RequestKind::ReExport,
                    DependencyKind::DynamicImport => RequestKind::DynamicImport,
                    DependencyKind::Require => RequestKind::Require,
                    DependencyKind::ImportType => {
                        eyre::bail!("type request in executable JavaScript")
                    }
                };
                let request = if let Some(DependencyTarget::External(specifier)) =
                    graph.dependency_target(file, dependency.id)
                {
                    NativeRequest {
                        code: Code::external(specifier.clone(), kind),
                        whole: false,
                    }
                } else {
                    let target = target_file(graph, file, dependency)?;
                    if cjs.contains(target) {
                        NativeRequest {
                            code: if dependency.kind == DependencyKind::DynamicImport {
                                cjs.dynamic(graph, target)?
                            } else {
                                cjs.static_dependency(
                                    graph,
                                    file,
                                    target,
                                    dependency,
                                    &module.parsed.facts,
                                    symbols,
                                )?
                            },
                            whole: true,
                        }
                    } else {
                        NativeRequest {
                            code: Code::request(self.namespaces[&target].clone(), kind),
                            whole: false,
                        }
                    }
                };
                self.native_requests.insert((file, dependency.id), request);
            }
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

    fn address(&self, identity: &ExportIdentity) -> eyre::Result<&str> {
        self.addresses
            .get(identity)
            .map(String::as_str)
            .ok_or_eyre("missing planned binding address")
    }
}
