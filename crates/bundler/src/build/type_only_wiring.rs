//! Attempt-local type-only connector admission over checked original evidence.
use super::*;
use ast_segmenter::views::{ExportValue, ImportedName, NamespaceAccess};
use cache_flatbuffers::type_only_wire::{OccurrenceKind, Presence};
use source_graph::{ExportIdentity, Namespace, TypeOnlyWiring};

pub(super) struct Wiring {
    pub graph: Option<SourceGraph>,
    observations: BTreeMap<PathBuf, type_only_facts::Observation>,
    pub identity: Vec<(PathBuf, Option<[u8; 16]>)>,
    pub warnings: Vec<String>,
}

impl Wiring {
    pub fn recheck(&self) -> eyre::Result<()> {
        for observation in self.observations.values() {
            observation.validate().wrap_err_with(|| {
                format!(
                    "rechecking type-only metadata {}",
                    observation.input().path().display()
                )
            })?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Candidates {
    types: BTreeSet<(PathBuf, String)>,
    values: BTreeSet<(PathBuf, String)>,
    unknown: bool,
}

impl Candidates {
    fn only_type(&self) -> bool {
        self.values.is_empty() && !self.unknown && self.types.len() == 1
    }

    fn merge(&mut self, other: Self) {
        self.types.extend(other.types);
        self.values.extend(other.values);
        self.unknown |= other.unknown;
    }

    fn type_projection(mut self, explicit: bool) -> Self {
        if explicit {
            self.types.append(&mut self.values);
        }
        self
    }

    fn namespace(path: PathBuf) -> Self {
        Self {
            values: BTreeSet::from([(path, "*".into())]),
            ..Default::default()
        }
    }
}

struct Evidence<'a> {
    graph: &'a SourceGraph,
    sources: &'a BTreeMap<PathBuf, Source>,
    cache: &'a mut type_only_facts::TypeOnlyCache,
    inventory: &'a Inventory,
    resolver: &'a (dyn DependencyResolver + Sync),
    root: &'a Path,
    options: &'a BuildOptions,
    records: BTreeMap<PathBuf, type_only_facts::Observation>,
    routes: BTreeMap<(PathBuf, String), Option<PathBuf>>,
    classified: BTreeMap<(PathBuf, String), bool>,
    demanded: BTreeMap<PathBuf, BTreeSet<String>>,
    pending: BTreeSet<PathBuf>,
}

impl Evidence<'_> {
    fn metadata(&mut self, path: &Path) -> eyre::Result<Option<type_only_facts::Observation>> {
        if let Some(record) = self.records.get(path) {
            return Ok(Some(record.clone()));
        }
        if let Some(original) = self
            .sources
            .get(path)
            .and_then(|s| s.transpiled.as_ref())
            .and_then(|s| s.original_facts.as_ref())
        {
            self.inventory
                .merge_observations(vec![original.input().clone()])?;
            self.records.insert(path.to_owned(), original.clone());
            return Ok(Some(original.clone()));
        }
        let declaration = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                [".d.ts", ".d.mts", ".d.cts"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            });
        let original = if declaration {
            let (original, warning) = self.cache.load_declaration(
                self.inventory,
                self.root,
                path,
                self.inventory.package_format(path)?,
                self.options.cache,
            )?;
            if let Some(warning) = warning {
                tracing::warn!(target: "bundler::diagnostic", "{warning}");
            }
            Some(original)
        } else {
            let selected = self.options.loaders.select(path)?;
            if selected.script_syntax().is_some_and(|s| {
                matches!(
                    s,
                    crate::loaders::ScriptSyntax::Ts | crate::loaders::ScriptSyntax::Tsx
                )
            }) {
                // Prepared owners were reused above; only erased routes reach here.
                Some(self.cache.load_original(
                    self.inventory,
                    self.root,
                    path,
                    &crate::transpile::parser_path(path, selected.script_syntax().unwrap()),
                    self.inventory.package_format(path)?,
                    self.options.cache,
                )?)
            } else {
                None
            }
        };
        if let Some(original) = &original {
            self.records.insert(path.to_owned(), original.clone());
        }
        Ok(original)
    }

    fn route(&mut self, path: &Path, request: &str) -> eyre::Result<Option<PathBuf>> {
        let key = (path.to_owned(), request.to_owned());
        if let Some(route) = self.routes.get(&key) {
            return Ok(route.clone());
        }
        // Failed erased/unused requests are not a new TypeScript checking pass.
        // The resolver still owns their positive/negative proof for final checks.
        let resolved = self.options.execution.resolve(|| {
            self.resolver
                .resolve_dependency(path, request, ResolutionMode::Import)
        });
        let route = match resolved {
            Ok(ResolvedDependency::File(path)) => Some(path),
            _ => None,
        };
        self.routes.insert(key, route.clone());
        Ok(route)
    }

    fn resolve(
        &mut self,
        path: &Path,
        name: &str,
        active: &mut BTreeSet<(PathBuf, String)>,
    ) -> eyre::Result<Candidates> {
        if self
            .demanded
            .entry(path.to_owned())
            .or_default()
            .insert(name.to_owned())
            && !self.sources.contains_key(path)
        {
            self.pending.insert(path.to_owned());
        }
        let key = (path.to_owned(), name.to_owned());
        if !active.insert(key.clone()) {
            return Ok(Candidates::default());
        }
        let result = self.resolve_inner(path, name, active);
        active.remove(&key);
        result
    }

    fn resolve_inner(
        &mut self,
        path: &Path,
        name: &str,
        active: &mut BTreeSet<(PathBuf, String)>,
    ) -> eyre::Result<Candidates> {
        if let Some(file) = self.graph.file_id(path) {
            match self
                .graph
                .resolve_optional_export(file, name, Namespace::Value)
            {
                Ok(Some(linked)) => {
                    let origin = match linked.identity {
                        ExportIdentity::Binding(owner, binding) => (
                            self.graph.file_path(owner).unwrap().to_owned(),
                            self.graph
                                .file_facts(owner)
                                .unwrap()
                                .bindings()
                                .iter()
                                .find(|b| b.id() == binding)
                                .unwrap()
                                .name()
                                .to_owned(),
                        ),
                        ExportIdentity::Namespace(owner) => {
                            (self.graph.file_path(owner).unwrap().to_owned(), "*".into())
                        }
                        ExportIdentity::Expression(key) => (
                            self.graph.file_path(key.file_id).unwrap().to_owned(),
                            "default".into(),
                        ),
                        ExportIdentity::CommonJs(owner, name) => {
                            (self.graph.file_path(owner).unwrap().to_owned(), name)
                        }
                        _ => (path.to_owned(), name.to_owned()),
                    };
                    return Ok(Candidates {
                        values: BTreeSet::from([origin]),
                        ..Default::default()
                    });
                }
                Err(_) => {
                    return Ok(Candidates {
                        unknown: true,
                        ..Default::default()
                    })
                }
                Ok(None) => {}
            }
        }
        let original = self.metadata(path)?;
        let original = if original.is_none()
            && self.graph.file_id(path).is_some()
            && crate::resource::kind(path, &self.options.loaders)
                == crate::resource::ResourceKind::Script
            && type_only_facts::companion(path).is_some()
        {
            let observed = self
                .cache
                .load_companion(
                    self.inventory,
                    self.root,
                    path,
                    self.inventory.package_format(path)?,
                    self.options.cache,
                )?
                .map(|(record, _)| record);
            if let Some(record) = &observed {
                self.records.insert(path.to_owned(), record.clone());
            }
            observed
        } else {
            original
        };
        if let Some(record) = original.and_then(|o| o.record().cloned()) {
            let facts = record.facts();
            let explicit = facts.exports().iter().any(|e| {
                !e.star() && e.name().value() == name
                    || e.star() && e.name().value() == name && e.name().value() != "*"
            });
            let mut candidates = Candidates::default();
            for export in facts.exports().iter().filter(|e| {
                if explicit {
                    e.name().value() == name
                } else {
                    e.star() && e.name().value() == "*" && name != "default"
                }
            }) {
                if let Some(request) = export.request() {
                    if let Some(target) = self.route(path, request.value())? {
                        let imported = export.imported().map_or(name, |n| n.value());
                        let routed = if imported == "*" && export.name().value() != "*" {
                            Candidates::namespace(target)
                        } else {
                            self.resolve(
                                &target,
                                if imported == "*" { name } else { imported },
                                active,
                            )?
                        };
                        candidates.merge(routed.type_projection(export.explicit_type()));
                    } else {
                        candidates.unknown = true;
                    }
                } else if let Some(local) = export.local() {
                    let import = facts.occurrences().iter().find(|o| {
                        matches!(
                            o.kind(),
                            OccurrenceKind::Import | OccurrenceKind::ImportEquals
                        ) && o.local().is_some_and(|n| n.value() == local.value())
                    });
                    if let Some(import) = import {
                        if let (Some(request), Some(imported)) =
                            (import.request(), import.imported())
                        {
                            if let Some(target) = self.route(path, request.value())? {
                                let routed = if imported.value() == "*" {
                                    Candidates::namespace(target)
                                } else {
                                    self.resolve(&target, imported.value(), active)?
                                };
                                candidates.merge(routed.type_projection(
                                    import.explicit_type() || export.explicit_type(),
                                ));
                            } else {
                                candidates.unknown = true;
                            }
                        }
                    } else if export.presence() == Presence::OnlyType
                        && facts.locals().iter().any(|declaration| {
                            declaration.name().value() == local.value()
                                && matches!(
                                    declaration.presence(),
                                    Presence::OnlyType
                                        | Presence::RuntimeValue
                                        | Presence::ConstEnum
                                )
                        })
                    {
                        candidates
                            .types
                            .insert((path.to_owned(), local.value().to_owned()));
                    } else if matches!(
                        export.presence(),
                        Presence::RuntimeValue | Presence::ConstEnum
                    ) {
                        candidates
                            .values
                            .insert((path.to_owned(), local.value().to_owned()));
                    } else {
                        candidates.unknown = true;
                    }
                } else if export.presence() == Presence::OnlyType {
                    candidates.types.insert((path.to_owned(), name.to_owned()));
                } else if matches!(
                    export.presence(),
                    Presence::RuntimeValue | Presence::ConstEnum
                ) {
                    candidates.values.insert((path.to_owned(), name.to_owned()));
                }
            }
            return Ok(candidates);
        }
        // JS carries no generic TypeOnlyFacts. Follow its already checked routes.
        let mut candidates = Candidates::default();
        if let Some(file) = self.graph.file_id(path) {
            for (leaf, name) in self.graph.missing_export_leaves(file, name) {
                let leaf = self.graph.file_path(leaf).unwrap();
                if leaf != path {
                    candidates.merge(self.resolve(leaf, &name, active)?);
                }
            }
        }
        Ok(candidates)
    }

    fn only_type(&mut self, path: &Path, name: &str) -> eyre::Result<bool> {
        let key = (path.to_owned(), name.to_owned());
        if let Some(&only_type) = self.classified.get(&key) {
            return Ok(only_type);
        }
        // Partial SCC walks depend on the active path; cache complete roots only.
        let only_type = self.resolve(path, name, &mut BTreeSet::new())?.only_type();
        self.classified.insert(key, only_type);
        Ok(only_type)
    }
}

fn location(path: &Path, text: &str, offset: u32) -> String {
    let (mut line, mut column, mut cr) = (1, 1, false);
    for ch in text[..offset as usize].chars() {
        match ch {
            '\n' if cr => {}
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {
                line += 1;
                column = 1;
            }
            _ => column += ch.len_utf16(),
        }
        cr = ch == '\r';
    }
    format!("{}:{line}:{column}", path.display())
}

fn original_bytes(inventory: &Inventory, path: &Path) -> eyre::Result<std::sync::Arc<[u8]>> {
    inventory
        .snapshot()
        .read(path)
        .wrap_err_with(|| {
            format!(
                "reading original type-only warning source {}",
                path.display()
            )
        })?
        .1
        .ok_or_else(|| {
            eyre::eyre!(
                "missing original type-only warning source {}",
                path.display()
            )
        })
}

impl BuildSession {
    pub(super) fn type_only_wiring(
        &mut self,
        inventory: &Inventory,
        resolver: &(dyn DependencyResolver + Sync),
        options: &BuildOptions,
    ) -> eyre::Result<Wiring> {
        let reusable = self.last.as_ref().map(|last| &last.graph).filter(|graph| {
            graph.module_ids().count() == self.sources.len() + usize::from(self.env.is_some())
                && self.sources.iter().all(|(path, source)| {
                    graph.file_id(path).is_some_and(|file| {
                        graph.file_provenance(file) == Some(source.record.provenance())
                            && source
                                .record
                                .module()
                                .facts()
                                .dependencies()
                                .iter()
                                .all(|d| {
                                    graph.dependency_target(file, d.id())
                                        == self.resolutions[path].get(&d.id())
                                })
                    })
                })
                && self.env.as_ref().is_none_or(|env| {
                    graph.file_id(&env::path(&self.root)).is_some_and(|file| {
                        graph.file_provenance(file) == Some(env.record.provenance())
                    })
                })
        });
        let mut owned_graph = if reusable.is_some() {
            None
        } else {
            Some(self.prepared_graph(options)?)
        };
        let graph = reusable.unwrap_or_else(|| owned_graph.as_ref().unwrap());
        let mut evidence = Evidence {
            graph,
            sources: &self.sources,
            cache: &mut self.type_only_cache,
            inventory,
            resolver,
            root: &self.root,
            options,
            records: BTreeMap::new(),
            routes: BTreeMap::new(),
            classified: BTreeMap::new(),
            demanded: BTreeMap::new(),
            pending: BTreeSet::new(),
        };
        let mut warnings = BTreeSet::new();
        let mut masks = BTreeMap::<PathBuf, TypeOnlyWiring>::new();
        // Include locally erased unmarked imports, not just graph diagnostics.
        let mut pending = self.sources.keys().cloned().collect::<BTreeSet<_>>();
        let mut visited = BTreeSet::new();
        while let Some(path) = pending.pop_first() {
            let prepared = self.sources.contains_key(&path);
            let selected = if prepared {
                BTreeSet::new()
            } else {
                evidence.demanded.get(&path).cloned().unwrap_or_default()
            };
            if !visited.insert((path.clone(), selected.clone())) {
                continue;
            }
            let Some(original) = evidence.metadata(&path)? else {
                continue;
            };
            let Some(record) = original.record() else {
                continue;
            };
            if original.input().path() != path {
                continue;
            }
            let facts = record.facts();
            let locals: BTreeSet<_> = facts
                .exports()
                .iter()
                .filter(|e| selected.contains(e.name().value()))
                .filter_map(|e| e.local().map(|n| n.value()))
                .collect();
            for occurrence in facts.occurrences() {
                if !prepared
                    && !match occurrence.kind() {
                        OccurrenceKind::Import => occurrence
                            .local()
                            .is_some_and(|n| locals.contains(n.value())),
                        OccurrenceKind::Reexport => occurrence
                            .exported()
                            .is_some_and(|n| selected.contains(n.value())),
                        _ => false,
                    }
                {
                    continue;
                }
                if !matches!(
                    occurrence.kind(),
                    OccurrenceKind::Import | OccurrenceKind::Reexport
                ) {
                    continue;
                }
                let executable = occurrence.uses().iter().any(|u| {
                    u.value_use()
                        && !u.value_as_type()
                        && !facts.occurrences().iter().any(|o| {
                            o.kind() == OccurrenceKind::LocalExport
                                && o.local()
                                    .zip(o.exported())
                                    .is_some_and(|(local, exported)| {
                                        local.span().start() <= exported.span().start()
                                            && occurrence.local().is_some_and(|imported| {
                                                imported.value() == local.value()
                                            })
                                    })
                                && o.span().start() <= u.span().start()
                                && u.span().end() <= o.span().end()
                        })
                });
                if occurrence.explicit_type() {
                    if executable {
                        let bytes = original_bytes(inventory, &path)?;
                        let text = std::str::from_utf8(&bytes).wrap_err_with(|| {
                            format!("decoding original type import {}", path.display())
                        })?;
                        let local = occurrence
                            .local()
                            .or_else(|| occurrence.imported())
                            .unwrap();
                        eyre::bail!(
                            "{}: explicit type-only import {:?} is used as an executable value",
                            location(&path, text, local.span().start()),
                            local.value()
                        );
                    }
                    continue;
                }
                let (Some(request), Some(imported)) = (occurrence.request(), occurrence.imported())
                else {
                    continue;
                };
                if imported.value() == "*" {
                    continue;
                }
                let Some(target) = evidence.route(&path, request.value())? else {
                    continue;
                };
                if !evidence.only_type(&target, imported.value())? {
                    continue;
                }
                let bytes = original_bytes(inventory, &path)?;
                let text = std::str::from_utf8(&bytes).wrap_err_with(|| {
                    format!(
                        "decoding original type-only warning source {}",
                        path.display()
                    )
                })?;
                let at = location(&path, text, imported.span().start());
                let local = occurrence.local().map_or(imported.value(), |n| n.value());
                ensure!(!executable, "{at}: type-only export {:?} from {:?} (local {local:?}) is used as an executable value", imported.value(), request.value());
                let action = if occurrence.kind() == OccurrenceKind::Import {
                    "import type"
                } else {
                    "export type"
                };
                warnings.insert(format!(
                    "{at}: {:?} from {:?} (local {local:?}) exists only as a type; use `{action}`",
                    imported.value(),
                    request.value()
                ));
                // Original names link the evidence to generated coordinates, not spans.
                if let Some(file) = graph.file_id(&path) {
                    let runtime = graph.file_facts(file).unwrap();
                    let mask = masks.entry(path.clone()).or_default();
                    if occurrence.kind() == OccurrenceKind::Import {
                        for import in runtime.imports().iter().filter(|i| {
                            runtime
                                .bindings()
                                .iter()
                                .any(|b| b.id() == i.binding() && b.name() == local)
                        }) {
                            mask.imports.insert(import.binding());
                            for export in runtime
                                .exports()
                                .iter()
                                .filter(|e| e.value() == ExportValue::Binding(import.binding()))
                            {
                                if let Some(name) = export.name() {
                                    mask.exports.insert(name.to_owned());
                                }
                            }
                        }
                    } else if let Some(exported) = occurrence.exported() {
                        mask.exports.insert(exported.value().to_owned());
                    }
                }
            }
            for occurrence in facts
                .occurrences()
                .iter()
                .filter(|o| !o.explicit_type() && o.kind() == OccurrenceKind::LocalExport)
            {
                let (Some(local), Some(exported)) = (occurrence.local(), occurrence.exported())
                else {
                    continue;
                };
                if !prepared && !selected.contains(exported.value()) {
                    continue;
                }
                // A default interface declaration is already explicitly a type,
                // unlike a separate unmarked export clause.
                if facts.locals().iter().any(|l| {
                    l.name().value() == local.value()
                        && l.span().start() == occurrence.span().start()
                }) {
                    continue;
                }
                if evidence.only_type(&path, exported.value())? {
                    let bytes = original_bytes(inventory, &path)?;
                    let text = std::str::from_utf8(&bytes).wrap_err_with(|| {
                        format!(
                            "decoding original type-only warning source {}",
                            path.display()
                        )
                    })?;
                    let upstream = facts.occurrences().iter().find(|o| {
                        o.kind() == OccurrenceKind::Import
                            && o.local().is_some_and(|n| n.value() == local.value())
                    });
                    let request = upstream
                        .and_then(|o| o.request())
                        .map_or("<local>", |n| n.value());
                    warnings.insert(format!("{}: {:?} from {request:?} (local {:?}) exists only as a type; use `export type`",
                        location(&path, text, local.span().start()), exported.value(), local.value()));
                    if graph.file_id(&path).is_some() {
                        masks
                            .entry(path.clone())
                            .or_default()
                            .exports
                            .insert(exported.value().to_owned());
                    }
                }
            }
            pending.append(&mut evidence.pending);
        }
        // Generated named reexports can be present in plain JS importers too.
        for file in graph.module_ids() {
            let path = graph.file_path(file).unwrap();
            let facts = graph.file_facts(file).unwrap();
            for import in facts.imports().iter().filter(|i| !i.type_only()) {
                let ImportedName::Name(name) = import.imported() else {
                    continue;
                };
                let Some(DependencyTarget::File(target)) =
                    graph.dependency_target(file, import.dependency())
                else {
                    continue;
                };
                if !evidence.only_type(target, name)? {
                    continue;
                }
                let mask = masks.entry(path.to_owned()).or_default();
                if mask.imports.contains(&import.binding()) {
                    continue;
                }
                ensure!(!facts.references().iter().any(|r| r.binding() == Some(import.binding()) && !r.type_only()
                    && !facts.exports().iter().any(|e| matches!(e.value(), ExportValue::Binding(binding) if binding == import.binding())
                        && e.span().start <= r.span().start && r.span().end <= e.span().end)),
                    "{}:{}..{}: type-only import {name:?} is used as an executable value", path.display(), import.span().start, import.span().end);
                mask.imports.insert(import.binding());
                for export in facts
                    .exports()
                    .iter()
                    .filter(|e| e.value() == ExportValue::Binding(import.binding()))
                {
                    if let Some(name) = export.name() {
                        mask.exports.insert(name.to_owned());
                    }
                }
                if self.sources[path].transpiled.is_none() {
                    let text = self.sources[path].text.text(path)?;
                    let request = facts
                        .dependencies()
                        .iter()
                        .find(|d| d.id() == import.dependency())
                        .unwrap();
                    let local = facts
                        .bindings()
                        .iter()
                        .find(|b| b.id() == import.binding())
                        .unwrap();
                    warnings.insert(format!("{}: {name:?} from {:?} (local {:?}) exists only as a type; use `import type`",
                        location(path, &text, import.span().start), request.specifier(), local.name()));
                }
            }
            for export in facts
                .exports()
                .iter()
                .filter(|e| !e.type_only() && e.name().is_some())
            {
                if masks
                    .get(path)
                    .is_some_and(|m| export.name().is_some_and(|n| m.exports.contains(n)))
                {
                    continue;
                }
                let ExportValue::ReExport {
                    dependency,
                    imported: ImportedName::Name(name),
                } = export.value()
                else {
                    continue;
                };
                let Some(DependencyTarget::File(target)) =
                    graph.dependency_target(file, dependency)
                else {
                    continue;
                };
                if evidence.only_type(target, name)? {
                    masks
                        .entry(path.to_owned())
                        .or_default()
                        .exports
                        .insert(export.name().unwrap().to_owned());
                    if self.sources[path].transpiled.is_none() {
                        let text = self.sources[path].text.text(path)?;
                        let request = facts
                            .dependencies()
                            .iter()
                            .find(|d| d.id() == dependency)
                            .unwrap();
                        warnings.insert(format!(
                            "{}: {name:?} from {:?} exists only as a type; use `export type`",
                            location(path, &text, export.span().start),
                            request.specifier()
                        ));
                    }
                }
            }
            // Static namespace reads must not turn a proven type into undefined.
            for recipe in facts.emission().segments() {
                for usage in recipe.namespaces() {
                    let NamespaceAccess::DirectRead { property, member } = usage.access() else {
                        continue;
                    };
                    let reference = facts.references().get(usage.reference() as usize).unwrap();
                    let Some(import) = facts
                        .imports()
                        .iter()
                        .find(|i| Some(i.binding()) == reference.binding())
                    else {
                        continue;
                    };
                    let Some(DependencyTarget::File(target)) =
                        graph.dependency_target(file, import.dependency())
                    else {
                        continue;
                    };
                    let namespace = match import.imported() {
                        ImportedName::Namespace => Some(target.clone()),
                        ImportedName::Name(name) => graph
                            .file_id(target)
                            .and_then(|target| {
                                graph
                                    .resolve_optional_export(target, name, Namespace::Value)
                                    .ok()
                                    .flatten()
                            })
                            .and_then(|e| match e.identity {
                                ExportIdentity::Namespace(file) => {
                                    graph.file_path(file).map(Path::to_owned)
                                }
                                _ => None,
                            }),
                    };
                    if let Some(namespace) = namespace {
                        ensure!(!evidence.only_type(&namespace, property)?,
                            "{}:{}..{}: type-only namespace member {property:?} is used as an executable value",
                            path.display(), member.start, member.end);
                    }
                }
            }
        }
        for (path, mask) in &mut masks {
            let facts = graph.file_facts(graph.file_id(path).unwrap()).unwrap();
            for dependency in facts
                .dependencies()
                .iter()
                .filter(|d| matches!(d.kind(), DependencyKind::Import | DependencyKind::ReExport))
            {
                let imports: Vec<_> = facts
                    .imports()
                    .iter()
                    .filter(|i| i.dependency() == dependency.id())
                    .collect();
                let exports: Vec<_> = facts.exports().iter().filter(|e| matches!(e.value(), ExportValue::ReExport { dependency: id, .. } if id == dependency.id())).collect();
                if (!imports.is_empty() || !exports.is_empty())
                    && imports.iter().all(|i| mask.imports.contains(&i.binding()))
                    && exports
                        .iter()
                        .all(|e| e.name().is_some_and(|n| mask.exports.contains(n)))
                {
                    mask.dependencies.insert(dependency.id());
                }
            }
        }
        self.type_wiring = masks;
        let observations = evidence.records;
        let identity = observations
            .iter()
            .map(|(p, o)| {
                (
                    p.clone(),
                    o.record().map(|r| cache_flatbuffers::hash128(r.backing())),
                )
            })
            .collect();
        let result = Wiring {
            graph: owned_graph.take(),
            observations,
            identity,
            warnings: warnings.into_iter().collect(),
        };
        result.recheck()?;
        Ok(result)
    }
}
