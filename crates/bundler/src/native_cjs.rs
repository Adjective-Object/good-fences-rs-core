//! Opaque availability is pure; native interop jobs own demanded snapshots.
use super::*;
use crate::assembly::{Code, RequestKind};
use ast_segmenter::owned::{ImportedName, ModuleFacts};

struct Value {
    path: PathBuf,
    name: String,
    exported: String,
}

pub(super) struct CommonJs {
    runtime: String,
    runtime_required: bool,
    values: BTreeMap<NodeId, Value>,
    factories: BTreeMap<u32, Value>,
    demands: BTreeMap<(u32, u32), BTreeSet<NodeId>>,
    pub members: BTreeMap<NodeId, PathBuf>,
    pub files: BTreeMap<PathBuf, Code>,
    pub proxies: BTreeMap<PathBuf, (PathBuf, BTreeSet<String>)>,
    pub namespaces: BTreeMap<PathBuf, PathBuf>,
    pub entries: BTreeMap<u32, PathBuf>,
}

impl CommonJs {
    pub fn new(
        graph: &SourceGraph,
        plan: &ChunkPlan,
        modules: &BTreeMap<u32, Module<'_>>,
    ) -> eyre::Result<Self> {
        let occupied: BTreeSet<_> = modules
            .values()
            .flat_map(|m| {
                m.parsed
                    .facts
                    .bindings
                    .iter()
                    .map(|b| b.name.as_str())
                    .chain(m.parsed.facts.references.iter().map(|r| r.name.as_str()))
            })
            .collect();
        let prefix = (0..)
            .map(|n| format!("__gf_cjs_{n}_"))
            .find(|p| occupied.iter().all(|name| !name.starts_with(p)))
            .unwrap();
        let mut result = Self {
            runtime: format!("{prefix}runtime"),
            runtime_required: modules.iter().any(|(&file, module)| {
                module.parsed.facts.commonjs
                    || module.parsed.facts.dependencies.iter().any(|dependency| {
                        dependency.pattern.is_some()
                            && plan.retention.segments.iter().any(|segment| {
                                segment.key.file_id == file
                                    && contains(
                                        module.parsed.segments[segment.key.segment_idx as usize]
                                            .span,
                                        dependency.span,
                                    )
                            })
                    })
            }),
            values: BTreeMap::new(),
            factories: BTreeMap::new(),
            demands: BTreeMap::new(),
            members: BTreeMap::new(),
            files: BTreeMap::new(),
            proxies: BTreeMap::new(),
            namespaces: BTreeMap::new(),
            entries: BTreeMap::new(),
        };
        for (&file, module) in modules.iter().filter(|(_, m)| m.parsed.facts.commonjs) {
            let original = graph.file_path(file).unwrap().to_owned();
            let body = NodeId::Segment {
                module: original.clone(),
                index: 0,
            };
            let chunk = plan.chunk_for(&body).ok_or_eyre("missing opaque owner")?;
            let path = PathBuf::from(format!("native/cjs-{}.cjs", digest(&format!("{chunk:?}"))));
            result.factories.insert(
                file,
                Value {
                    path: path.clone(),
                    name: format!("{prefix}{}", digest(original.to_str().unwrap())),
                    exported: format!("{prefix}{}", digest(original.to_str().unwrap())),
                },
            );
            result.members.insert(body, path.clone());
            result
                .members
                .insert(NodeId::Module(original.clone()), path);

            // Equal availability roots alone do not permit evaluating a deferred
            // consumer. Merge only adjacent facade parts with equal demand seams.
            let requests: Vec<_> = crate::cjs::requests(graph, &plan.retention, file)
                .into_iter()
                .filter(|(from, _)| direct_request(graph, *from, file))
                .collect();
            for (from, imported) in &requests {
                let consumer = match from {
                    GraphNode::Module(file) => *file,
                    GraphNode::Segment(key) => key.file_id,
                    GraphNode::Unlinked { .. } => unreachable!(),
                };
                result
                    .demands
                    .entry((consumer, file))
                    .or_default()
                    .insert(Self::node(graph, file, imported));
            }
            let mut previous: Option<(BTreeSet<String>, PathBuf)> = None;
            let mut nodes = vec![NodeId::CommonJsProxy {
                module: original.clone(),
                name: None,
            }];
            nodes.extend(
                module
                    .surface
                    .iter()
                    .filter(|n| n.as_str() != "default")
                    .map(|name| NodeId::CommonJsProxy {
                        module: original.clone(),
                        name: Some(name.clone()),
                    }),
            );
            nodes.push(NodeId::CommonJsNamespace(original.clone()));
            for (index, node) in nodes.into_iter().enumerate() {
                if plan.chunk_for(&node).is_none() {
                    continue;
                }
                let mut demands = BTreeSet::new();
                for (from, request) in &requests {
                    let demanded = match &node {
                        NodeId::CommonJsProxy { name: None, .. } => true,
                        NodeId::CommonJsProxy {
                            name: Some(name), ..
                        } => *request == ImportedName::Name(name.clone()),
                        NodeId::CommonJsNamespace(_) => *request == ImportedName::Namespace,
                        _ => unreachable!(),
                    };
                    if demanded {
                        let consumer = match from {
                            GraphNode::Module(file) => *file,
                            GraphNode::Segment(key) => key.file_id,
                            GraphNode::Unlinked { .. } => unreachable!(),
                        };
                        demands.insert(format!("module:{:?}", graph.file_path(consumer).unwrap()));
                    }
                }
                if !matches!(node, NodeId::CommonJsProxy { name: Some(_), .. }) {
                    for (root, seed) in &plan.roots {
                        if seed.evaluation_roots.contains(&file)
                            || seed.binding_roots.contains(&GraphNode::Module(file))
                        {
                            demands.insert(format!("root:{root:?}"));
                        }
                    }
                }
                let path = if let Some((old, path)) = &previous {
                    if *old == demands && !matches!(node, NodeId::CommonJsNamespace(_)) {
                        path.clone()
                    } else {
                        interop_path(&original, index)
                    }
                } else {
                    interop_path(&original, index)
                };
                previous = Some((demands, path.clone()));
                result.members.insert(node.clone(), path.clone());
                let exported = match &node {
                    NodeId::CommonJsProxy {
                        name: Some(name), ..
                    } => name.clone(),
                    _ => "default".into(),
                };
                result.values.insert(
                    node,
                    Value {
                        path,
                        name: format!("{prefix}v{}_{}", digest(original.to_str().unwrap()), index),
                        exported,
                    },
                );
            }
        }
        for (&file, module) in modules.iter().filter(|(_, m)| m.parsed.facts.commonjs) {
            let original = graph.file_path(file).unwrap().to_owned();
            if let Ok(default) = result.value(graph, file, &ImportedName::Name("default".into())) {
                let mut path = default.path.clone();
                for name in &module.surface {
                    if result
                        .value(graph, file, &ImportedName::Name(name.clone()))?
                        .path
                        != default.path
                    {
                        path = interop_path(&original, "proxy");
                    }
                }
                result
                    .proxies
                    .insert(original.clone(), (path, module.surface.clone()));
            }
            if let Ok(value) = result.value(graph, file, &ImportedName::Namespace) {
                result.namespaces.insert(original, value.path.clone());
            }
        }
        Ok(result)
    }

    pub fn contains(&self, file: u32) -> bool {
        self.factories.contains_key(&file)
    }

    pub fn add_runtime(
        &self,
        files: &mut BTreeMap<PathBuf, Code>,
        chunk_artifacts: &mut BTreeMap<ChunkId, BTreeSet<PathBuf>>,
    ) -> eyre::Result<()> {
        if !self.runtime_required {
            return Ok(());
        }
        let esm_only = self.factories.is_empty();
        let path = PathBuf::from(if esm_only {
            "native/context-intrinsics.mjs"
        } else {
            RUNTIME_PATH
        });
        // First dependency of every job, including entries that never touch CJS.
        for (file, source) in files.iter_mut() {
            let header = if file.extension().is_some_and(|ext| ext == "cjs") {
                Code::request(path.clone(), RequestKind::Require)
                    .surrounded(format!("\nconst {} = require(", self.runtime), ");\n")
            } else {
                Code::request(path.clone(), RequestKind::Import)
                    .surrounded(format!("\nimport {} from ", self.runtime), ";\n")
            };
            source.prepend_after_hashbang(header);
        }
        let source = Code::from(if esm_only {
            include_str!("context_intrinsics.mjs")
        } else {
            include_str!("runtime.cjs")
        });
        ensure!(
            files.insert(path.clone(), source).is_none(),
            "runtime path collision"
        );
        for paths in chunk_artifacts.values_mut() {
            if !paths.is_empty() {
                paths.insert(path.clone());
            }
        }
        Ok(())
    }

    fn node(graph: &SourceGraph, file: u32, name: &ImportedName) -> NodeId {
        let module = graph.file_path(file).unwrap().to_owned();
        match name {
            ImportedName::Name(name) => NodeId::CommonJsProxy {
                module,
                name: (name != "default").then(|| name.clone()),
            },
            ImportedName::Namespace => NodeId::CommonJsNamespace(module),
        }
    }

    fn value(&self, graph: &SourceGraph, file: u32, name: &ImportedName) -> eyre::Result<&Value> {
        self.values
            .get(&Self::node(graph, file, name))
            .ok_or_else(|| {
                eyre::eyre!(
                    "missing demanded CommonJS facade part {name:?} in {}",
                    graph.file_path(file).unwrap().display()
                )
            })
    }

    pub fn initialize(&self, graph: &SourceGraph, file: u32) -> eyre::Result<Code> {
        Ok(Code::evaluation(
            self.value(graph, file, &ImportedName::Name("default".into()))?
                .path
                .clone(),
        ))
    }

    pub fn dynamic(&self, graph: &SourceGraph, file: u32) -> eyre::Result<Code> {
        let value = self.value(graph, file, &ImportedName::Namespace)?;
        Ok(
            Code::request(value.path.clone(), RequestKind::DynamicImport).surrounded(
                format!("{}.apply({}.then, import(", self.runtime, self.runtime),
                "), [ns => ns.default])",
            ),
        )
    }

    pub fn context_dynamic(
        &self,
        graph: &SourceGraph,
        file: u32,
        dependency: &Dependency,
        namespaces: &BTreeMap<u32, PathBuf>,
    ) -> eyre::Result<Option<crate::context::Dispatch>> {
        let Some(DependencyTarget::Context(candidates)) =
            graph.dependency_target(file, dependency.id)
        else {
            return Ok(None);
        };
        let mut requests = BTreeMap::new();
        for (request, target) in candidates {
            let target = graph.file_id(target).ok_or_eyre("missing context target")?;
            let expression = if self.contains(target) {
                self.dynamic(graph, target)?
            } else {
                Code::request(namespaces[&target].clone(), RequestKind::DynamicImport)
                    .surrounded("import(", ")")
            };
            requests.insert(request.clone(), expression);
        }
        Ok(Some(crate::context::Dispatch {
            requests,
            dynamic: true,
            runtime: self.runtime.clone(),
        }))
    }

    pub fn import(
        &self,
        graph: &SourceGraph,
        file: u32,
        name: &ImportedName,
        local: &str,
    ) -> eyre::Result<Code> {
        let value = self.value(graph, file, name)?;
        Ok(
            Code::request(value.path.clone(), RequestKind::Import).surrounded(
                format!("import {{{} as {local}}} from ", quote(&value.exported)),
                ";\n",
            ),
        )
    }

    pub fn reexport(
        &self,
        graph: &SourceGraph,
        file: u32,
        name: &ImportedName,
        public: &str,
    ) -> eyre::Result<Code> {
        let value = self.value(graph, file, name)?;
        let mut code = Code::request(value.path.clone(), RequestKind::ReExport).surrounded(
            format!(
                "export {{{} as {}}} from ",
                quote(&value.exported),
                quote(public)
            ),
            ";\n",
        );
        code.default_export = public == "default";
        Ok(code)
    }

    pub fn evaluate(&self, graph: &SourceGraph, consumer: u32, target: u32) -> eyre::Result<Code> {
        let mut output = self.initialize(graph, target)?;
        let mut nodes: Vec<_> = self
            .demands
            .get(&(consumer, target))
            .into_iter()
            .flatten()
            .collect();
        nodes.sort_by_key(|node| match node {
            NodeId::CommonJsProxy { name, .. } => (false, name.as_deref()),
            NodeId::CommonJsNamespace(_) => (true, None),
            _ => unreachable!(),
        });
        for node in nodes {
            output.push(Code::evaluation(self.values[node].path.clone()));
        }
        Ok(output)
    }

    /// Dependencies and bindings retain their original source-local identities.
    pub fn static_dependency(
        &self,
        graph: &SourceGraph,
        file: u32,
        target: u32,
        dependency: &Dependency,
        facts: &ModuleFacts,
        symbols: &Symbols,
    ) -> eyre::Result<Code> {
        let mut requested = BTreeMap::new();
        let mut declarations = Code::default();
        if dependency.kind == DependencyKind::Import {
            for import in facts
                .imports
                .iter()
                .filter(|i| i.dependency == dependency.id)
            {
                let binding = facts
                    .bindings
                    .iter()
                    .find(|b| b.id == import.binding)
                    .unwrap();
                if !symbols.imports.contains_key(&(file, binding.id))
                    || !self.demands.get(&(file, target)).is_some_and(|nodes| {
                        nodes.contains(&Self::node(graph, target, &import.imported))
                    })
                {
                    continue;
                }
                let value = self.value(graph, target, &import.imported)?;
                declarations.push(self.import(graph, target, &import.imported, &binding.name)?);
                requested.insert(import_key(&import.imported), &value.path);
            }
        } else {
            for export in facts.exports.iter().filter(|e| {
                matches!(e.value, ExportValue::ReExport { dependency: d, .. } if d == dependency.id)
            }) {
                if export.name.is_none() && graph.has_external_star_exports(file) {
                    let proxy = &self.proxies[graph.file_path(target).unwrap()].0;
                    declarations.push(Code::request(proxy.clone(), RequestKind::ReExport).surrounded("export * from ", ";\n"));
                    continue;
                }
                let ExportValue::ReExport { imported, .. } = &export.value else {
                    unreachable!()
                };
                let names = match &export.name {
                    Some(name) => vec![(name.clone(), imported.clone())],
                    None => symbols.exports[&file]
                        .iter()
                        .filter_map(|(public, linked)| match &linked.identity {
                            ExportIdentity::CommonJs(f, name) if *f == target && public != "default" => {
                                Some((public.clone(), ImportedName::Name(name.clone())))
                            }
                            _ => None,
                        })
                        .collect(),
                };
                for (public, imported) in names {
                    let value = self.value(graph, target, &imported)?;
                    declarations.push(self.reexport(graph, target, &imported, &public)?);
                    requested.insert(import_key(&imported), &value.path);
                }
            }
        }
        // Establish sorted property snapshot order independently of source aliases.
        let mut output = self.initialize(graph, target)?;
        for path in requested.values() {
            output.push(Code::evaluation((*path).clone()));
        }
        output.push(declarations);
        Ok(output)
    }

    pub fn emit(
        &mut self,
        graph: &SourceGraph,
        modules: &BTreeMap<u32, Module<'_>>,
        namespaces: &BTreeMap<u32, PathBuf>,
        entries: &BTreeSet<u32>,
    ) -> eyre::Result<()> {
        // Export initializers before pure cyclic availability imports. Only CJS
        // requires, not ESM interop seams, may observe a still-running factory.
        let mut factories: Vec<_> = self.factories.iter().collect();
        factories.sort_by_key(|(_, value)| &value.name);
        let runtime = &self.runtime;
        for (_, value) in &factories {
            self.files.entry(value.path.clone()).or_default().push_str(&format!(
                "let {0}_record;\nexports.{0} = {0};\nfunction {0}(interop = false) {{\nif ({0}_record) {{\nif (interop && !{0}_record.loaded) throw {runtime}.assign(new {runtime}.Error('Cannot import CommonJS module in a cycle'), {{code: 'ERR_REQUIRE_CYCLE_MODULE'}});\nreturn {0}_record.exports;\n}}\nconst module = {0}_record = {{exports: {{}}, loaded: false}};\ntry {{ {runtime}.apply({0}_factory, module.exports, [module.exports, module, __filename, __dirname]); module.loaded = true; return module.exports; }} catch (error) {{ {0}_record = undefined; throw error; }}\n}}\n",
                value.name
            ));
        }
        let mut availability_imports = BTreeSet::new();
        for &(&file, value) in &factories {
            let module = &modules[&file];
            let mut renderer = crate::emit::CachedRenderer::new(
                module.source,
                &module.parsed.facts,
                &module.parsed.facts.emission.segments[0],
                &[],
            );
            let mut imports = BTreeMap::new();
            for dependency in module
                .parsed
                .facts
                .dependencies
                .iter()
                .filter(|d| !d.type_only)
            {
                if let Some(DependencyTarget::External(specifier)) =
                    graph.dependency_target(file, dependency.id)
                {
                    let (kind, prefix) = match dependency.kind {
                        DependencyKind::Require => (RequestKind::Require, "require("),
                        DependencyKind::DynamicImport => (RequestKind::DynamicImport, "import("),
                        _ => eyre::bail!("static ESM external in opaque CommonJS body"),
                    };
                    renderer.dependency(
                        dependency,
                        Code::external(specifier.clone(), kind).surrounded(prefix, ")"),
                        true,
                    );
                    continue;
                }
                if dependency.pattern.is_some() && dependency.kind == DependencyKind::DynamicImport
                {
                    renderer.context(
                        dependency,
                        self.context_dynamic(graph, file, dependency, namespaces)?
                            .ok_or_eyre("missing dynamic context")?,
                    );
                    continue;
                }
                let targets = match graph.dependency_target(file, dependency.id) {
                    Some(DependencyTarget::Context(candidates)) => candidates
                        .iter()
                        .map(|(request, path)| {
                            Ok((
                                request.clone(),
                                graph
                                    .file_id(path)
                                    .ok_or_eyre("missing require context target")?,
                            ))
                        })
                        .collect::<eyre::Result<BTreeMap<_, _>>>()?,
                    _ => BTreeMap::from([(
                        dependency.specifier.clone(),
                        target_file(graph, file, dependency)?,
                    )]),
                };
                let mut requests = BTreeMap::new();
                for (request, target) in targets {
                    let replacement = match dependency.kind {
                        DependencyKind::Require => {
                            if let Some(other) = self.factories.get(&target) {
                                if other.path != value.path
                                    && availability_imports
                                        .insert((value.path.clone(), other.name.clone()))
                                {
                                    imports.insert(
                                        other.name.clone(),
                                        Code::request(other.path.clone(), RequestKind::Require)
                                            .surrounded(
                                                format!("const {{{}}} = require(", other.name),
                                                ");\n",
                                            ),
                                    );
                                }
                                Code::from(format!("{}()", other.name))
                            } else {
                                // Native require is intentionally inside the original call site.
                                Code::request(namespaces[&target].clone(), RequestKind::Require)
                                    .surrounded("require(", ")")
                            }
                        }
                        DependencyKind::DynamicImport if self.contains(target) => {
                            self.dynamic(graph, target)?
                        }
                        DependencyKind::DynamicImport => {
                            Code::request(namespaces[&target].clone(), RequestKind::DynamicImport)
                                .surrounded("import(", ")")
                        }
                        _ => eyre::bail!("static ESM dependency in opaque CommonJS body"),
                    };
                    requests.insert(request, replacement);
                }
                if dependency.pattern.is_some() {
                    renderer.context(
                        dependency,
                        crate::context::Dispatch {
                            requests,
                            dynamic: false,
                            runtime: self.runtime.clone(),
                        },
                    );
                } else {
                    renderer.dependency(dependency, requests.into_values().next().unwrap(), true);
                }
            }
            let source = renderer.materialize()?;
            let output = self.files.get_mut(&value.path).unwrap();
            for import in imports.into_values() {
                output.push(import);
            }
            output.push(source.surrounded(
                format!(
                    "function {}_factory(exports, module, __filename, __dirname) {{\n",
                    value.name
                ),
                "\n}\n",
            ));
        }
        // NodeId order is initialization, sorted names, then namespace per source.
        let mut ordered: Vec<_> = self.values.iter().collect();
        ordered.sort_by_key(|(node, _)| match node {
            NodeId::CommonJsProxy { module, name } => (module, false, name.as_deref()),
            NodeId::CommonJsNamespace(module) => (module, true, None),
            _ => unreachable!(),
        });
        let mut imported = BTreeSet::new();
        for (node, value) in ordered {
            let module = match node {
                NodeId::CommonJsProxy { module, .. } | NodeId::CommonJsNamespace(module) => module,
                _ => unreachable!(),
            };
            let file = graph.file_id(module).unwrap();
            let mut imports = Code::default();
            let expression = match node {
                NodeId::CommonJsProxy { name: None, .. } => {
                    let factory = &self.factories[&file];
                    imports.push(
                        Code::request(factory.path.clone(), RequestKind::Import)
                            .surrounded(format!("import {}_factory from ", value.name), ";\n"),
                    );
                    format!("{}_factory.{}(true)", value.name, factory.name)
                }
                _ => {
                    let default = self.value(graph, file, &ImportedName::Name("default".into()))?;
                    if default.path != value.path
                        && imported.insert((value.path.clone(), default.path.clone()))
                    {
                        imports.push(
                            Code::request(default.path.clone(), RequestKind::Import).surrounded(
                                format!("import {{default as {}}} from ", default.name),
                                ";\n",
                            ),
                        );
                    }
                    if let NodeId::CommonJsProxy {
                        name: Some(name), ..
                    } = node
                    {
                        format!(
                            "{0} == null ? undefined : {0}[{1}]",
                            default.name,
                            quote(name)
                        )
                    } else {
                        let helper = format!("{}_namespace", value.name);
                        imports.push_str(
                            &include_str!("commonjs_namespace.mjs")
                                .replace("commonjsNamespace", &helper)
                                .replace("intrinsics", runtime),
                        );
                        format!("{helper}({})", default.name)
                    }
                }
            };
            imports.push_str(&format!(
                "\nconst {} = {expression};\nexport {{{} as {}}};\n",
                value.name,
                value.name,
                quote(&value.exported)
            ));
            imports.default_export = value.exported == "default";
            self.files
                .entry(value.path.clone())
                .or_default()
                .push(imports);
        }
        for (&file, module) in modules.iter().filter(|(_, m)| m.parsed.facts.commonjs) {
            let original = graph.file_path(file).unwrap().to_owned();
            if let Ok(default) = self.value(graph, file, &ImportedName::Name("default".into())) {
                let path = &self.proxies[&original].0;
                let mut output = Code::default();
                for name in &module.surface {
                    output.push(self.reexport(
                        graph,
                        file,
                        &ImportedName::Name(name.clone()),
                        name,
                    )?);
                }
                ensure!(
                    self.files.contains_key(&default.path),
                    "missing CommonJS default job"
                );
                if *path != default.path {
                    self.files.insert(path.clone(), output);
                }
            }
            if entries.contains(&file) {
                let factory = &self.factories[&file];
                let path = PathBuf::from(format!(
                    "native/entry-{}.cjs",
                    digest(original.to_str().unwrap())
                ));
                self.files.insert(
                    path.clone(),
                    Code::request(factory.path.clone(), RequestKind::Require)
                        .surrounded(format!("const {{{}}} = require(", factory.name),
                            format!(");\n{runtime}.defineProperty(module, 'exports', {{get: {0}}});\n{0}();\n", factory.name)),
                );
                self.entries.insert(file, path);
            }
        }
        Ok(())
    }
}

fn interop_path(module: &std::path::Path, part: impl std::fmt::Display) -> PathBuf {
    PathBuf::from(format!(
        "native/interop-{}-{part}.mjs",
        digest(module.to_str().unwrap())
    ))
}

fn import_key(name: &ImportedName) -> (bool, String) {
    match name {
        ImportedName::Name(name) => (false, name.clone()),
        ImportedName::Namespace => (true, String::new()),
    }
}

fn direct_request(graph: &SourceGraph, from: GraphNode, target: u32) -> bool {
    let (file, span) = match from {
        GraphNode::Module(file) => (file, None),
        GraphNode::Segment(key) => (
            key.file_id,
            Some(graph.file_segments(key.file_id).unwrap()[key.segment_idx as usize].span),
        ),
        GraphNode::Unlinked { .. } => return false,
    };
    graph
        .file_facts(file)
        .unwrap()
        .dependencies
        .iter()
        .any(|d| {
            !d.type_only
                && matches!(d.kind, DependencyKind::Import | DependencyKind::ReExport)
                && span.is_none_or(|span| contains(span, d.span))
                && target_file(graph, file, d).is_ok_and(|f| f == target)
        })
}
