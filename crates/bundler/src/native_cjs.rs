//! Opaque availability is pure; native interop jobs own demanded snapshots.
use super::*;
use ast_segmenter::owned::{ImportedName, ModuleFacts};

struct Value {
    path: PathBuf,
    name: String,
    exported: String,
}

pub(super) struct CommonJs {
    values: BTreeMap<NodeId, Value>,
    factories: BTreeMap<u32, Value>,
    demands: BTreeMap<(u32, u32), BTreeSet<NodeId>>,
    pub members: BTreeMap<NodeId, PathBuf>,
    pub files: BTreeMap<PathBuf, String>,
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
        Ok(result)
    }

    pub fn contains(&self, file: u32) -> bool {
        self.factories.contains_key(&file)
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
            .ok_or_eyre("missing demanded CommonJS facade part")
    }

    pub fn initialize(&self, graph: &SourceGraph, file: u32) -> eyre::Result<String> {
        Ok(format!(
            "import {};\n",
            specifier(
                &self
                    .value(graph, file, &ImportedName::Name("default".into()))?
                    .path
            )
        ))
    }

    pub fn dynamic(&self, graph: &SourceGraph, file: u32) -> eyre::Result<String> {
        let value = self.value(graph, file, &ImportedName::Namespace)?;
        Ok(format!(
            "import({}).then(ns => ns.default)",
            specifier(&value.path),
        ))
    }

    pub fn evaluate(
        &self,
        graph: &SourceGraph,
        consumer: u32,
        target: u32,
    ) -> eyre::Result<String> {
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
            output.push_str(&format!("import {};\n", specifier(&self.values[node].path)));
        }
        Ok(output)
    }

    /// Retained-source offsets are local to this parse, never original spans.
    pub fn static_dependency(
        &self,
        graph: &SourceGraph,
        file: u32,
        target: u32,
        dependency: &Dependency,
        facts: &ModuleFacts,
        symbols: &Symbols,
    ) -> eyre::Result<String> {
        let mut requested = BTreeMap::new();
        let mut declarations = String::new();
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
                let original = graph
                    .file_facts(file)
                    .ok_or_eyre("missing original CommonJS consumer facts")?
                    .bindings
                    .iter()
                    .find(|b| b.import && b.name == binding.name)
                    .ok_or_eyre("missing original CommonJS import")?;
                if !symbols.imports.contains_key(&(file, original.id))
                    || !self.demands.get(&(file, target)).is_some_and(|nodes| {
                        nodes.contains(&Self::node(graph, target, &import.imported))
                    })
                {
                    continue;
                }
                let value = self.value(graph, target, &import.imported)?;
                declarations.push_str(&format!(
                    "import {{{} as {}}} from {};\n",
                    quote(&value.exported),
                    binding.name,
                    specifier(&value.path)
                ));
                requested.insert(import_key(&import.imported), &value.path);
            }
        } else {
            for export in facts.exports.iter().filter(|e| {
                matches!(e.value, ExportValue::ReExport { dependency: d, .. } if d == dependency.id)
            }) {
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
                    declarations.push_str(&format!(
                        "export {{{} as {}}} from {};\n",
                        quote(&value.exported), quote(&public), specifier(&value.path)
                    ));
                    requested.insert(import_key(&imported), &value.path);
                }
            }
        }
        // Establish sorted property snapshot order independently of source aliases.
        let mut output = self.initialize(graph, target)?;
        for path in requested.values() {
            output.push_str(&format!("import {};\n", specifier(path)));
        }
        output.push_str(&declarations);
        Ok(output)
    }

    pub fn emit(
        &mut self,
        graph: &SourceGraph,
        modules: &BTreeMap<u32, Module<'_>>,
        owners: &BTreeMap<u32, u32>,
        entries: &BTreeSet<u32>,
    ) -> eyre::Result<()> {
        // Export initializers before pure cyclic availability imports. Only CJS
        // requires, not ESM interop seams, may observe a still-running factory.
        let mut factories: Vec<_> = self.factories.iter().collect();
        factories.sort_by_key(|(_, value)| &value.name);
        for (_, value) in &factories {
            self.files.entry(value.path.clone()).or_default().push_str(&format!(
                "let {0}_record;\nexports.{0} = {0};\nfunction {0}(interop = false) {{\nif ({0}_record) {{\nif (interop && !{0}_record.loaded) throw Object.assign(new Error('Cannot import CommonJS module in a cycle'), {{code: 'ERR_REQUIRE_CYCLE_MODULE'}});\nreturn {0}_record.exports;\n}}\nconst module = {0}_record = {{exports: {{}}, loaded: false}};\ntry {{ {0}_factory.call(module.exports, module.exports, module); module.loaded = true; return module.exports; }} catch (error) {{ {0}_record = undefined; throw error; }}\n}}\n",
                value.name
            ));
        }
        let mut availability_imports = BTreeSet::new();
        for &(&file, value) in &factories {
            let module = &modules[&file];
            let mut edits = Vec::new();
            let mut imports = BTreeSet::new();
            if let Some(hashbang) = &module.parsed.facts.hashbang {
                edits.push(Edit {
                    span: hashbang.span,
                    replacement: String::new(),
                });
            }
            for dependency in module
                .parsed
                .facts
                .dependencies
                .iter()
                .filter(|d| !d.type_only)
            {
                let target = target_file(graph, file, dependency)?;
                let replacement = match dependency.kind {
                    DependencyKind::Require => {
                        if let Some(other) = self.factories.get(&target) {
                            if other.path != value.path
                                && availability_imports
                                    .insert((value.path.clone(), other.name.clone()))
                            {
                                imports.insert(format!(
                                    "const {{{}}} = require({});\n",
                                    other.name,
                                    specifier(&other.path)
                                ));
                            }
                            format!("{}()", other.name)
                        } else {
                            // Native require is intentionally inside the original call site.
                            format!("require({})", request(graph, owners[&target]))
                        }
                    }
                    DependencyKind::DynamicImport if self.contains(target) => {
                        self.dynamic(graph, target)?
                    }
                    DependencyKind::DynamicImport => {
                        format!("import({})", request(graph, owners[&target]))
                    }
                    _ => eyre::bail!("static ESM dependency in opaque CommonJS body"),
                };
                edits.push(Edit {
                    span: dependency.span,
                    replacement,
                });
            }
            let source = rewrite::apply(module.source, &edits)?;
            let output = self.files.get_mut(&value.path).unwrap();
            for import in imports {
                output.push_str(&import);
            }
            output.push_str(&format!(
                "function {}_factory(exports, module) {{\n{source}\n}}\n",
                value.name
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
            let mut imports = String::new();
            let expression = match node {
                NodeId::CommonJsProxy { name: None, .. } => {
                    let factory = &self.factories[&file];
                    imports.push_str(&format!(
                        "import {}_factory from {};\n",
                        value.name,
                        specifier(&factory.path)
                    ));
                    format!("{}_factory.{}(true)", value.name, factory.name)
                }
                _ => {
                    let default = self.value(graph, file, &ImportedName::Name("default".into()))?;
                    if default.path != value.path
                        && imported.insert((value.path.clone(), default.path.clone()))
                    {
                        imports.push_str(&format!(
                            "import {{default as {}}} from {};\n",
                            default.name,
                            specifier(&default.path)
                        ));
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
                                .replace("commonjsNamespace", &helper),
                        );
                        format!("{helper}({})", default.name)
                    }
                }
            };
            self.files
                .entry(value.path.clone())
                .or_default()
                .push_str(&format!(
                    "{imports}\nconst {} = {expression};\nexport {{{} as {}}};\n",
                    value.name,
                    value.name,
                    quote(&value.exported)
                ));
        }
        for (&file, module) in modules.iter().filter(|(_, m)| m.parsed.facts.commonjs) {
            let original = graph.file_path(file).unwrap().to_owned();
            if let Ok(default) = self.value(graph, file, &ImportedName::Name("default".into())) {
                let mut path = default.path.clone();
                let mut output = String::new();
                let mut separate = false;
                for name in &module.surface {
                    let value = self.value(graph, file, &ImportedName::Name(name.clone()))?;
                    separate |= value.path != default.path;
                    output.push_str(&format!(
                        "export {{{} as {}}} from {};\n",
                        quote(&value.exported),
                        quote(name),
                        specifier(&value.path)
                    ));
                }
                ensure!(
                    self.files.contains_key(&default.path),
                    "missing CommonJS default job"
                );
                if separate {
                    path = interop_path(&original, "proxy");
                    self.files.insert(path.clone(), output);
                }
                self.proxies
                    .insert(original.clone(), (path, module.surface.clone()));
            }
            if let Ok(value) = self.value(graph, file, &ImportedName::Namespace) {
                self.namespaces.insert(original.clone(), value.path.clone());
            }
            if entries.contains(&file) {
                let factory = &self.factories[&file];
                let path = PathBuf::from(format!(
                    "native/entry-{}.cjs",
                    digest(original.to_str().unwrap())
                ));
                self.files.insert(
                    path.clone(),
                    format!(
                        "const {{{1}}} = require({0});\nObject.defineProperty(module, 'exports', {{get: {1}}});\n{1}();\n",
                        specifier(&factory.path),
                        factory.name
                    ),
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
