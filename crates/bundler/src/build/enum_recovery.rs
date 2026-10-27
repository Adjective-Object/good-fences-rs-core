//! Attempt-local compatibility overlays. Persist evidence, never linking decisions.
use super::*;
use ast_segmenter::views::{ImportedName, NamespaceAccess};
use cache_flatbuffers::type_only_wire::{ConstantKind, Presence};
use oxc_span::Span;
use source_graph::{ExportIdentity, Namespace};

pub(super) struct Recovery {
    pub graph: Option<SourceGraph>,
    pub warnings: Vec<String>,
    pub identity: Vec<(PathBuf, Option<[u8; 16]>)>,
    pub generated: BTreeMap<PathBuf, Span>,
    observations: BTreeMap<PathBuf, type_only_facts::Observation>,
}

#[derive(Clone)]
pub(super) struct Retained {
    input_key: Fingerprint,
    warnings: Vec<String>,
    identity: Vec<(PathBuf, Option<[u8; 16]>)>,
    generated: BTreeMap<PathBuf, Span>,
    observations: BTreeMap<PathBuf, type_only_facts::Observation>,
}

impl Recovery {
    pub fn recheck(&self) -> eyre::Result<()> {
        for observation in self.observations.values() {
            observation.validate().wrap_err_with(|| {
                format!(
                    "rechecking const enum declaration {}",
                    observation.input().path().display()
                )
            })?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Demand {
    importer: u32,
    start: u32,
    end: u32,
    target: u32,
    name: String,
}

#[derive(Default)]
struct Enum {
    members: Vec<(String, Value)>,
    exports: BTreeSet<String>,
}

enum Value {
    Number(f64),
    String(String),
}

fn quote(value: &str) -> String {
    serde_json::to_string(value)
        .expect("string JSON")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn warning_location(session: &BuildSession, importer: &Path, offset: u32) -> eyre::Result<String> {
    let source = &session.sources[importer];
    let text = source.text.text(importer)?;
    let prefix = &text[..offset as usize];
    let (mut line, mut column, mut cr) = (0, 0, false);
    for ch in prefix.chars() {
        match ch {
            '\n' if cr => {}
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {
                line += 1;
                column = 0;
            }
            _ => column += ch.len_utf16() as u32,
        }
        cr = ch == '\r';
    }
    let mut path = importer.to_owned();
    let mut original = source.transpiled.is_none();
    for map in source
        .transpiled
        .as_ref()
        .and_then(|t| t.source_map.as_ref())
        .into_iter()
        .chain(session.upstreams.get(importer).map(|u| u.map()))
    {
        let Some(token) = map.lookup(line, column).filter(|token| token.source() >= 0) else {
            original = false;
            break;
        };
        let mapped = map.root().sources().get(token.source() as usize);
        path = url::Url::parse(mapped.url())
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .unwrap_or_else(|| PathBuf::from(mapped.url()));
        line = token.original_line();
        column = token.original_column();
        original = true;
        // Upstream maps describe the compiler's input, not arbitrary imported sources.
        if path != importer {
            break;
        }
    }
    Ok(format!(
        "{}:{}:{}{}",
        path.display(),
        line + 1,
        column + 1,
        if original {
            ""
        } else {
            " (prepared JavaScript coordinates)"
        }
    ))
}

fn demands(graph: &SourceGraph) -> Vec<Demand> {
    let mut demands = BTreeSet::new();
    for diagnostic in graph.diagnostics() {
        if let Some(missing) = &diagnostic.missing_export {
            if missing.namespace == Namespace::Value {
                demands.insert(Demand {
                    importer: diagnostic.file_id,
                    start: diagnostic.site.start,
                    end: diagnostic.site.end,
                    target: missing.target,
                    name: missing.name.clone(),
                });
            }
        }
    }
    for file in graph.module_ids() {
        let Some(facts) = graph.file_facts(file) else {
            continue;
        };
        for import in facts.imports().iter().filter(|i| !i.type_only()) {
            let Some(DependencyTarget::File(path)) =
                graph.dependency_target(file, import.dependency())
            else {
                continue;
            };
            let Some(target) = graph.file_id(path) else {
                continue;
            };
            if let ImportedName::Name(name) = import.imported() {
                if name != "default"
                    && matches!(graph.resolve_optional_export(target, name, Namespace::Value), Ok(Some(export)) if matches!(export.identity, ExportIdentity::CommonJs(..)))
                {
                    demands.insert(Demand {
                        importer: file,
                        start: import.span().start,
                        end: import.span().end,
                        target,
                        name: name.to_owned(),
                    });
                }
            }
        }
        for recipe in facts.emission().segments() {
            for usage in recipe.namespaces() {
                let NamespaceAccess::DirectRead { member, property } = usage.access() else {
                    continue;
                };
                let reference = facts
                    .references()
                    .get(usage.reference() as usize)
                    .expect("checked reference");
                let Some(binding) = reference.binding() else {
                    continue;
                };
                let Some(import) = facts
                    .imports()
                    .iter()
                    .find(|i| i.binding() == binding && !i.type_only())
                else {
                    continue;
                };
                let Some(DependencyTarget::File(path)) =
                    graph.dependency_target(file, import.dependency())
                else {
                    continue;
                };
                let Some(target) = graph.file_id(path) else {
                    continue;
                };
                let namespace = match import.imported() {
                    ImportedName::Namespace => Some(target),
                    ImportedName::Name(name) => graph
                        .resolve_optional_export(target, name, Namespace::Value)
                        .ok()
                        .flatten()
                        .and_then(|e| match e.identity {
                            ExportIdentity::Namespace(file) => Some(file),
                            _ => None,
                        }),
                };
                let Some(target) = namespace else { continue };
                if matches!(
                    graph.resolve_optional_export(target, property, Namespace::Value),
                    Ok(None)
                ) || (property != "default"
                    && graph.file_facts(target).is_some_and(|f| f.commonjs()))
                {
                    demands.insert(Demand {
                        importer: file,
                        start: member.start,
                        end: member.end,
                        target,
                        name: property.to_owned(),
                    });
                }
            }
        }
    }
    demands.into_iter().collect()
}

fn opaque_cjs_surface(facts: ast_segmenter::views::ModuleFacts<'_>, name: &str) -> bool {
    let surface = facts.commonjs_surface();
    facts.commonjs()
        && (matches!(
            name,
            "__proto__"
                | "constructor"
                | "toString"
                | "toLocaleString"
                | "valueOf"
                | "hasOwnProperty"
                | "isPrototypeOf"
                | "propertyIsEnumerable"
                | "__defineGetter__"
                | "__defineSetter__"
                | "__lookupGetter__"
                | "__lookupSetter__"
        ) || !surface.unknown().is_empty()
            || surface.accesses().iter().any(|access| {
                access
                    .names()
                    .iter()
                    .any(|potential| potential.name() == name)
            }))
}

/// Read only checked borrowed metadata; unresolved values are never representatives.
fn enum_for(observation: &type_only_facts::Observation, name: &str) -> Option<(String, Enum)> {
    let facts = observation.record()?.facts();
    let export = facts
        .exports()
        .iter()
        .find(|e| e.name().value() == name && !e.explicit_type() && !e.star())?;
    if export.presence() != Presence::ConstEnum || export.request().is_some() {
        return None;
    }
    let local = export.local()?.value();
    let mut result = Enum::default();
    let mut found = false;
    for table in facts
        .const_enums()
        .iter()
        .filter(|e| e.name().value() == local)
    {
        found = true;
        if facts
            .diagnostics()
            .iter()
            .any(|d| d.span().start() < table.span().end() && d.span().end() > table.span().start())
        {
            return None;
        }
        for member in table.members() {
            let value = match member.kind() {
                ConstantKind::Number => Value::Number(member.number()?),
                ConstantKind::String => Value::String(member.string()?.to_owned()),
                _ => return None,
            };
            result
                .members
                .push((member.name().value().to_owned(), value));
        }
    }
    found.then(|| (local.to_owned(), result))
}

fn initializer(source: &Source, enums: &BTreeMap<String, Enum>) -> String {
    let facts = source.record.module().facts();
    let mut occupied: BTreeSet<_> = facts
        .bindings()
        .iter()
        .map(|b| b.name().to_owned())
        .chain(facts.references().iter().map(|r| r.name().to_owned()))
        .collect();
    let mut generated = String::from("\n;");
    for (index, table) in enums.values().enumerate() {
        let mut private = format!("__gf_const_enum_{index}");
        while !occupied.insert(private.clone()) {
            private.push('_');
        }
        let empty = if table.members.iter().any(|(name, _)| name == "__proto__") {
            "{[\"__proto__\"]: void 0}"
        } else {
            "{}"
        };
        generated.push_str(&format!("\nconst {private} = {empty};\n"));
        // Define forward data properties, including __proto__, without global helpers.
        // Repeated merged members retain ordinary enum assignment order.
        for (member, value) in &table.members {
            let numeric = matches!(value, Value::Number(_));
            let member = quote(member);
            let value = match value {
                Value::Number(value) if *value == 0.0 && value.is_sign_negative() => {
                    "-0".to_owned()
                }
                Value::Number(value) => value.to_string(),
                Value::String(value) => quote(value),
            };
            generated.push_str(&format!("{private}[{member}] = {value};\n"));
            if numeric {
                generated.push_str(&format!("{private}[{value}] = {member};\n"));
            }
        }
        if facts.commonjs() {
            for name in &table.exports {
                let name = quote(name);
                generated.push_str(&format!("if (module.exports !== null && (typeof module.exports === 'object' || typeof module.exports === 'function') && !({name} in module.exports)) module.exports[{name}] = {private};\n"));
            }
        } else {
            for name in &table.exports {
                generated.push_str(&format!("export {{ {private} as {} }};\n", quote(name)));
            }
        }
    }
    generated
}

impl BuildSession {
    pub(super) fn prepared_graph(&self, options: &BuildOptions) -> eyre::Result<SourceGraph> {
        let mut inputs = self
            .sources
            .iter()
            .map(|(path, source)| {
                let text = source.text.text(path)?;
                let parsed = source
                    .record
                    .to_parsed_reporting_policies(&text)
                    .wrap_err_with(|| {
                        format!("borrowing prepared graph facts for {}", path.display())
                    })?;
                let constructor = if crate::resource::kind(path, &options.loaders)
                    == crate::resource::ResourceKind::Script
                    && source.transpiled.is_none()
                    && !self.enum_bases.contains_key(path)
                    && options
                        .loaders
                        .select(path)?
                        .script_syntax()
                        .is_some_and(|syntax| crate::transpile::parser_path(path, syntax) == *path)
                {
                    OwnedSourceFileInput::from_parsed
                } else {
                    OwnedSourceFileInput::from_prepared_resource
                };
                Ok(constructor(
                    path.clone(),
                    parsed,
                    self.resolutions[path]
                        .iter()
                        .map(|(id, target)| (*id, target.clone()))
                        .collect(),
                ))
            })
            .collect::<eyre::Result<Vec<_>>>()?;
        if let Some(env) = &self.env {
            inputs.push(env.input()?);
        }
        Ok(SourceGraph::from_owned(inputs))
    }

    pub(super) fn recover_enums(
        &mut self,
        inventory: &Inventory,
        options: &BuildOptions,
        declarations: &PreparedDeclarations,
        previous: &BTreeMap<PathBuf, Source>,
        counters: &mut Counters,
        warnings: &mut Vec<String>,
    ) -> eyre::Result<Recovery> {
        let input_key = Fingerprint::of(
            format!(
                "{:?}:{:?}:{:?}:{:?}:{:?}",
                options.loaders,
                self.sources
                    .iter()
                    .map(|(path, source)| (
                        path,
                        source.text.input_identity(),
                        &source.css_identity,
                        source.record.source_len(),
                        source.record.source_hash(),
                        source.record.config(),
                        source.record.provenance().records_hash(),
                        source.transpiled.as_ref().map(|t| (
                            t.config,
                            t.source_map
                                .as_ref()
                                .map(|m| cache_flatbuffers::hash128(m.bytes())),
                            t.source_map_hint,
                        ))
                    ))
                    .collect::<Vec<_>>(),
                self.resolutions
                    .iter()
                    .map(|(path, targets)| {
                        let mut targets: Vec<_> = targets.iter().collect();
                        targets.sort_by_key(|(id, _)| id.0);
                        (path, targets)
                    })
                    .collect::<Vec<_>>(),
                self.upstreams
                    .iter()
                    .map(|(path, u)| (path, cache_flatbuffers::hash128(u.map().bytes())))
                    .collect::<Vec<_>>(),
                self.env.as_ref().map(|env| &*env.text),
            )
            .as_bytes(),
        );
        if let Some(retained) = self
            .enum_recovery
            .as_ref()
            .filter(|r| {
                r.input_key == input_key
                    && r.generated.keys().eq(previous.keys())
                    && r.observations.values().all(|o| o.validate().is_ok())
            })
            .cloned()
        {
            for path in retained.observations.keys() {
                // Re-admit metadata through this attempt's scoped epoch.
                self.type_only_cache.load_companion(
                    inventory,
                    &self.root,
                    path,
                    inventory.package_format(path)?,
                    options.cache,
                )?;
            }
            for (path, source) in previous {
                let original = self
                    .sources
                    .insert(path.clone(), source.clone())
                    .expect("original prepared source");
                self.enum_bases.insert(path.clone(), original);
            }
            let result = Recovery {
                graph: None,
                warnings: retained.warnings,
                identity: retained.identity,
                generated: retained.generated,
                observations: retained.observations,
            };
            result.recheck()?;
            for warning in &result.warnings {
                tracing::warn!(target: "bundler::diagnostic", "{warning}");
                warnings.push(warning.clone());
            }
            return Ok(result);
        }
        let mut result = Recovery {
            graph: None,
            warnings: Vec::new(),
            identity: Vec::new(),
            generated: BTreeMap::new(),
            observations: BTreeMap::new(),
        };
        let mut recovered: BTreeMap<PathBuf, BTreeMap<String, Enum>> = BTreeMap::new();
        let mut warned = BTreeSet::new();
        loop {
            let graph = self.prepared_graph(options)?;
            let mut changed = BTreeSet::new();
            for demand in demands(&graph) {
                let mut candidates = Vec::new();
                for (file, name) in graph.missing_export_leaves(demand.target, &demand.name) {
                    let path = graph.file_path(file).expect("candidate file");
                    if crate::resource::kind(path, &options.loaders)
                        != crate::resource::ResourceKind::Script
                        || type_only_facts::companion(path).is_none()
                        || graph
                            .file_facts(file)
                            .is_some_and(|facts| opaque_cjs_surface(facts, &name))
                    {
                        continue;
                    }
                    if !result.observations.contains_key(path) {
                        if let Some((observation, warning)) = self
                            .type_only_cache
                            .load_companion(
                                inventory,
                                &self.root,
                                path,
                                inventory.package_format(path)?,
                                options.cache,
                            )
                            .wrap_err_with(|| {
                                format!(
                                    "recovering missing export {:?} from {}",
                                    demand.name,
                                    path.display()
                                )
                            })?
                        {
                            if let Some(warning) = warning {
                                warnings.push(warning);
                            }
                            result.observations.insert(path.to_owned(), observation);
                        }
                    }
                    if let Some(observation) = result.observations.get(path) {
                        if let Some((local, table)) = enum_for(observation, &name) {
                            candidates.push((path.to_owned(), name, local, table));
                        }
                    }
                }
                if candidates.len() > 1 {
                    let importer = graph.file_path(demand.importer).expect("requesting source");
                    let location = warning_location(self, importer, demand.start)?;
                    let origins = candidates
                        .iter()
                        .map(|(path, name, local, _)| {
                            format!(
                                "{} (export {name:?}, const enum {local:?}, declaration {})",
                                path.display(),
                                result.observations[path].input().path().display()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    eyre::bail!(
                        "{location}: ambiguous missing export {:?}: competing declaration-backed origins: {origins}",
                        demand.name
                    );
                }
                if candidates.is_empty() {
                    continue;
                }
                let (path, name, local, table) = candidates.pop().unwrap();
                let observation = &result.observations[&path];
                if warned.insert(demand.clone()) {
                    let importer = graph.file_path(demand.importer).expect("requesting source");
                    let location = warning_location(self, importer, demand.start)?;
                    result.warnings.push(format!(
                        "{location}: recovered missing export {:?} (leaf export {name:?}, declared const enum {local:?}) as an ordinary runtime enum from {} using declaration {}; prefer a package-provided runtime enum",
                        demand.name, path.display(), observation.input().path().display()
                    ));
                }
                let tables = recovered.entry(path.clone()).or_default();
                if tables.entry(local).or_insert(table).exports.insert(name) {
                    changed.insert(path);
                }
            }
            if changed.is_empty() {
                result.graph = Some(graph);
                break;
            }
            for path in changed {
                let original = self
                    .enum_bases
                    .entry(path.clone())
                    .or_insert_with(|| self.sources[&path].clone());
                let original_text = original.text.text(&path)?;
                let insert = original
                    .record
                    .module()
                    .facts()
                    .emission()
                    .source_map()
                    .map_or(original_text.len(), |span| span.start as usize);
                let generated = initializer(original, &recovered[&path]);
                let mut text = String::with_capacity(original_text.len() + generated.len());
                text.push_str(&original_text[..insert]);
                text.push_str(&generated);
                text.push_str(&original_text[insert..]);
                let end = insert
                    .checked_add(generated.len())
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| eyre::eyre!("generated enum source exceeds byte span range"))?;
                let parser_path = path.with_extension(if original.record.config().commonjs() {
                    "cjs"
                } else {
                    "mjs"
                });
                let mut loaded = prepare::load(
                    &path,
                    original.text.read_input(&path)?,
                    text,
                    &parser_path,
                    inventory.package_format(&path)?,
                    previous.get(&path),
                    options.cache,
                    (&self.root, &options.policy_allowlist),
                    declarations,
                    &|_, _, _| {},
                )?;
                if let Some(error) = loaded.admission.error {
                    return Err(eyre::Report::new(error)).wrap_err_with(|| {
                        format!("admitting generated enum source {}", path.display())
                    });
                }
                if let Some(warning) = loaded.warning {
                    warnings.push(warning);
                }
                match loaded.status {
                    Some(CacheStatus::Hit) => counters.disk_hits += 1,
                    Some(CacheStatus::Miss(_)) => counters.source_parses += 1,
                    None => counters.memory_hits += 1,
                }
                let mut source = loaded
                    .source
                    .take()
                    .unwrap_or_else(|| previous[&path].clone());
                source.transpiled = original.transpiled.clone();
                // Insertion follows executable code: original dependency IDs stay exact.
                ensure!(
                    source
                        .record
                        .module()
                        .facts()
                        .dependencies()
                        .iter()
                        .map(|d| d.id())
                        .eq(original
                            .record
                            .module()
                            .facts()
                            .dependencies()
                            .iter()
                            .map(|d| d.id())),
                    "generated enum changed dependency occurrence identities: {}",
                    path.display()
                );
                self.sources.insert(path.clone(), source);
                result.generated.insert(path, Span::new(insert as u32, end));
            }
        }
        result.identity = result
            .observations
            .iter()
            .map(|(path, o)| {
                (
                    path.clone(),
                    o.record().map(|r| cache_flatbuffers::hash128(r.backing())),
                )
            })
            .collect();
        result.recheck()?;
        result.warnings.sort();
        self.enum_recovery = Some(Retained {
            input_key,
            warnings: result.warnings.clone(),
            identity: result.identity.clone(),
            generated: result.generated.clone(),
            observations: result.observations.clone(),
        });
        for warning in &result.warnings {
            tracing::warn!(target: "bundler::diagnostic", "{warning}");
            warnings.push(warning.clone());
        }
        Ok(result)
    }
}
