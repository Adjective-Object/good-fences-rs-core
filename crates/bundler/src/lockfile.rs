//! Expected install identities only; never a substitute for Node/filesystem resolution.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use aube_lockfile::{LocalSource, LockedPackage, LockfileGraph, LockfileKind};
use eyre::{ensure, Context, OptionExt};
use ftree_cache::inputs::PathInput;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

type Hash = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageFingerprint {
    pub(crate) instance: Hash,
    pub(crate) dependencies: Hash,
    /// This closure includes workspace/local inputs requiring on-demand validation.
    pub(crate) mutable: bool,
    /// False for unresolved edges or adapters that cannot preserve the full graph.
    pub(crate) complete: bool,
}

pub(crate) struct ProjectLockfile {
    pub(crate) path: PathBuf,
    pub(crate) kind: LockfileKind,
    packages: BTreeMap<String, PackageFingerprint>,
    importers: BTreeMap<String, PackageFingerprint>,
    inputs: Vec<PathInput>,
}

impl ProjectLockfile {
    /// `project` is the chosen lockfile root, not an arbitrary importing directory.
    /// Upstream selects precedence/branch lockfiles. Format readers deliberately
    /// bypass its `fill_local_package_versions` eager workspace manifest reads.
    pub(crate) fn read(
        project: &Path,
        selected: Option<LockfileKind>,
    ) -> eyre::Result<Option<Self>> {
        let project = project.canonicalize()?;
        let (_, path) = aube_lockfile::active_lockfile_path_selecting(&project, selected);
        let Some(path) = path else { return Ok(None) };
        let mut input = PathInput::capture(&path)?;
        let raw = std::fs::read(&path)?;
        input.seal_content(&raw);
        let mut inputs = vec![input];
        let kind = aube_lockfile::detect_existing_lockfile_kind_selecting(&project, selected)
            .ok_or_eyre("selected lockfile has no recognized format")?;
        let mut manifest_bytes = None;
        let graph = match kind {
            LockfileKind::Aube | LockfileKind::Pnpm => aube_lockfile::pnpm::parse(&path)?,
            LockfileKind::Npm | LockfileKind::NpmShrinkwrap => aube_lockfile::npm::parse(&path)?,
            LockfileKind::Bun => aube_lockfile::bun::parse(&path)?,
            LockfileKind::Yarn | LockfileKind::YarnBerry => {
                // Classic Yarn needs only the root manifest to identify direct deps.
                let manifest = if kind == LockfileKind::Yarn {
                    let bytes = std::fs::read(project.join("package.json"))?;
                    let mut input = PathInput::capture(&project.join("package.json"))?;
                    input.seal_content(&bytes);
                    inputs.push(input);
                    let manifest = serde_json::from_slice(&bytes)?;
                    manifest_bytes = Some(bytes);
                    manifest
                } else {
                    Default::default()
                };
                aube_lockfile::yarn::parse(&path, &manifest)?
            }
        };
        let mut nodes = if kind == LockfileKind::YarnBerry {
            // aube 2.8.1 intentionally skips workspace/builtin-patch blocks and
            // merges same-version locators. Retain these identities and edges
            // from the already validated Berry document, not installed manifests.
            berry_nodes(std::str::from_utf8(&raw)?)?
        } else {
            normalized_nodes(&graph)?
        };
        if matches!(
            kind,
            LockfileKind::Npm | LockfileKind::NpmShrinkwrap | LockfileKind::Yarn
        ) {
            // aube drops unresolved npm/Classic edges and Classic optional edges,
            // and merges npm install paths. Until a lossless overlay exists,
            // no normalized record from these adapters is eligible as complete.
            // Retain even discarded declarations/instances in a coarse identity.
            let raw: Hash = Sha256::digest(&raw).into();
            let manifest: Option<Hash> = manifest_bytes.map(|bytes| Sha256::digest(bytes).into());
            for node in nodes.values_mut() {
                node.identity = json!(["lossy-adapter-v1", node.identity, raw, manifest]);
                node.complete = false;
            }
        }
        let records = fingerprint_nodes(&nodes)?;
        let mut packages = BTreeMap::new();
        let mut importers = BTreeMap::new();
        for (key, record) in records {
            match key {
                Key::Package(key) => {
                    packages.insert(key, record);
                }
                Key::Importer(key) => {
                    importers.insert(key, record);
                }
            }
        }
        // Format parsers may reopen the file. Seal the bytes surrounding that
        // parse, rather than attributing a later read to the parsed graph.
        for input in &inputs {
            input.validate()?;
        }
        Ok(Some(Self {
            path: path.canonicalize()?,
            kind,
            packages,
            importers,
            inputs,
        }))
    }

    pub(crate) fn recheck(&self) -> eyre::Result<()> {
        for input in &self.inputs {
            input.validate()?;
        }
        Ok(())
    }

    pub(crate) fn physical_package(
        &self,
        project: &Path,
        package: &Path,
        manifest: &Value,
    ) -> Option<&PackageFingerprint> {
        if let Ok(relative) = package.strip_prefix(project) {
            let relative = relative
                .components()
                .map(|p| p.as_os_str().to_str())
                .collect::<Option<Vec<_>>>()?
                .join("/");
            if let Some(record) = self.importer(if relative.is_empty() { "." } else { &relative }) {
                return Some(record);
            }
        }
        // Only explicit installed locators can select peer/patch instances.
        // Name/version similarity is not evidence of a selected peer provider.
        manifest
            .get("_locator")
            .or_else(|| manifest.get("_resolution"))
            .and_then(Value::as_str)
            .and_then(|locator| self.package(locator))
    }

    pub(crate) fn package(&self, locator: &str) -> Option<&PackageFingerprint> {
        self.packages.get(locator)
    }

    pub(crate) fn importer(&self, path: &str) -> Option<&PackageFingerprint> {
        self.importers.get(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
enum Key {
    Package(String),
    Importer(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct Edge {
    role: String,
    name: String,
    request: String,
    target: Option<Key>,
}

struct Node {
    identity: Value,
    metadata: Value,
    mutable: bool,
    complete: bool,
    edges: Vec<Edge>,
}

fn local_identity(source: &LocalSource) -> Value {
    match source {
        LocalSource::Directory(p) => json!(["directory", p]),
        LocalSource::Tarball(p) => json!(["tarball", p]),
        LocalSource::Link(p) => json!(["link", p]),
        LocalSource::Portal(p) => json!(["portal", p]),
        LocalSource::Exec(p) => json!(["exec", p]),
        LocalSource::Git(g) => json!([
            "git",
            g.url,
            g.committish,
            g.resolved,
            g.integrity,
            g.subpath
        ]),
        LocalSource::RemoteTarball(t) => {
            json!(["remote-tarball", t.url, t.integrity, t.git_hosted])
        }
    }
}

fn package_identity(key: &str, p: &LockedPackage, graph: &LockfileGraph) -> Value {
    json!({
        "locator": key, "dep_path": p.dep_path, "name": p.name, "version": p.version,
        "integrity": p.integrity, "checksum": p.yarn_checksum, "tarball": p.tarball_url,
        "source": p.local_source.as_ref().map(local_identity), "alias": p.alias_of,
        "registry_git_hosted": p.registry_git_hosted,
        "patch": p.lookup_patch(&graph.patched_dependencies),
    })
}

fn normalized_nodes(graph: &LockfileGraph) -> eyre::Result<BTreeMap<Key, Node>> {
    let mut nodes = BTreeMap::new();
    for (key, p) in &graph.packages {
        let mut edges = Vec::new();
        for (role, deps) in [
            ("dependency", &p.dependencies),
            ("optional", &p.optional_dependencies),
            ("peer", &p.peer_dependencies),
        ] {
            for (name, request) in deps {
                let resolved = if role == "peer" {
                    p.dependencies
                        .get(name)
                        .or_else(|| p.optional_dependencies.get(name))
                } else {
                    Some(request)
                };
                edges.push(Edge {
                    role: role.into(),
                    name: name.clone(),
                    request: request.clone(),
                    target: resolved
                        .and_then(|resolved| {
                            aube_lockfile::resolve_dep_edge(name, resolved, |k| {
                                graph.packages.contains_key(k)
                            })
                        })
                        .map(Key::Package),
                });
            }
        }
        let peers: BTreeMap<_, _> = p
            .peer_dependencies_meta
            .iter()
            .map(|(name, meta)| (name, meta.optional))
            .collect();
        let mutable = p
            .local_source
            .as_ref()
            .is_some_and(|s| !matches!(s, LocalSource::Git(_) | LocalSource::RemoteTarball(_)))
            || key.contains("@workspace:")
            || p.version.starts_with("workspace:")
            || p.lookup_patch(&graph.patched_dependencies).is_some();
        nodes.insert(
            Key::Package(key.clone()),
            Node {
                identity: package_identity(key, p, graph),
                metadata: json!({
                    "peers": p.peer_dependencies, "peer_meta": peers,
                    "transitive_peers": p.transitive_peer_dependencies,
                    "os": p.os, "cpu": p.cpu, "libc": p.libc,
                    "bundled": p.bundled_dependencies, "engines": p.engines,
                    "declared": p.declared_dependencies, "extra": p.extra_meta,
                    "optional": p.optional,
                }),
                mutable,
                complete: true,
                edges,
            },
        );
    }
    for (path, deps) in &graph.importers {
        let edges = deps
            .iter()
            .map(|d| Edge {
                role: aube_lockfile::dep_type_label(d.dep_type).into(),
                name: d.name.clone(),
                request: d.specifier.clone().unwrap_or_else(|| d.dep_path.clone()),
                target: graph
                    .packages
                    .contains_key(&d.dep_path)
                    .then(|| Key::Package(d.dep_path.clone())),
            })
            .collect();
        nodes.insert(
            Key::Importer(path.clone()),
            Node {
                identity: json!(["importer", path]),
                metadata: Value::Null,
                mutable: true,
                complete: true,
                edges,
            },
        );
    }
    Ok(nodes)
}

fn berry_nodes(content: &str) -> eyre::Result<BTreeMap<Key, Node>> {
    // JSON-compatible Berry mappings give canonical scalar metadata without
    // depending on the upstream parser's private preservation-marker fields.
    let doc: Value = yaml_serde::from_str(content).wrap_err("reading Berry identity overlay")?;
    let blocks = doc.as_object().ok_or_eyre("Berry root must be a mapping")?;
    let mut descriptors = BTreeMap::new();
    let mut locators = BTreeSet::new();
    for (header, block) in blocks.iter().filter(|(h, _)| !h.starts_with("__")) {
        let locator = block["resolution"]
            .as_str()
            .ok_or_eyre("Berry block missing resolution")?;
        ensure!(
            locators.insert(locator.to_owned()),
            "duplicate Berry locator {locator}"
        );
        for descriptor in header.split(", ") {
            if let Some(old) = descriptors.insert(descriptor.to_owned(), locator.to_owned()) {
                ensure!(old == locator, "conflicting Berry descriptor {descriptor}");
            }
        }
    }
    let mut nodes = BTreeMap::new();
    for (header, block) in blocks.iter().filter(|(h, _)| !h.starts_with("__")) {
        let locator = block["resolution"]
            .as_str()
            .ok_or_eyre("Berry block missing resolution")?;
        let (_, source) = locator
            .trim_start_matches('@')
            .split_once('@')
            .ok_or_eyre("Berry locator missing protocol")?;
        let mutable = ["workspace:", "file:", "link:", "portal:", "exec:", "patch:"]
            .iter()
            .any(|p| source.starts_with(p));
        let mut edges = Vec::new();
        for role in ["dependencies", "optionalDependencies", "peerDependencies"] {
            if let Some(deps) = block[role].as_object() {
                for (name, request) in deps {
                    // Yarn/YAML permits unquoted numeric version constraints.
                    let request = match request {
                        Value::String(value) => value.clone(),
                        Value::Number(value) => value.to_string(),
                        _ => eyre::bail!("Berry {role} dependency must be a string or number"),
                    };
                    // Peer ranges are constraints, not selected providers.
                    // Only an explicit dependency supplies a locked peer edge;
                    // otherwise require later installed-tree validation.
                    let resolved = if role == "peerDependencies" {
                        block["dependencies"][name]
                            .as_str()
                            .or_else(|| block["optionalDependencies"][name].as_str())
                    } else {
                        Some(request.as_str())
                    };
                    let target = resolved.and_then(|resolved| {
                        let exact = format!("{name}@{resolved}");
                        let npm = format!("{name}@npm:{resolved}");
                        descriptors.get(&exact).or_else(|| descriptors.get(&npm))
                    });
                    edges.push(Edge {
                        role: role.into(),
                        name: name.clone(),
                        request,
                        target: target.cloned().map(Key::Package),
                    });
                }
            }
        }
        let node = Node {
            identity: json!([
                "berry",
                locator,
                block["checksum"],
                block["version"],
                block["linkType"]
            ]),
            metadata: block.clone(),
            mutable,
            complete: true,
            edges: edges.clone(),
        };
        nodes.insert(Key::Package(locator.into()), node);
        // Workspace locators encode the importer path, including scoped names.
        if let Some(path) = source.strip_prefix("workspace:") {
            ensure!(
                !nodes.contains_key(&Key::Importer(path.into())),
                "duplicate Berry workspace {path}"
            );
            nodes.insert(
                Key::Importer(path.into()),
                Node {
                    identity: json!(["berry-importer", path, locator]),
                    metadata: json!(header),
                    mutable: true,
                    complete: true,
                    edges: vec![Edge {
                        role: "workspace".into(),
                        name: locator.into(),
                        request: locator.into(),
                        target: Some(Key::Package(locator.into())),
                    }],
                },
            );
        }
    }
    Ok(nodes)
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.iter().map(|(k, v)| (k, canonical(v))).collect();
            serde_json::to_value(sorted).expect("JSON values are serializable")
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        value => value.clone(),
    }
}

fn hash(value: Value) -> eyre::Result<Hash> {
    Ok(Sha256::digest(serde_json::to_vec(&canonical(&value))?).into())
}

/// Hash the SCC condensation DAG, not a DFS tree with cycle back-edges cut.
/// Every member sees the complete component and all outgoing named edges.
fn fingerprint_nodes(
    nodes: &BTreeMap<Key, Node>,
) -> eyre::Result<BTreeMap<Key, PackageFingerprint>> {
    let keys: Vec<_> = nodes.keys().cloned().collect();
    let index: BTreeMap<_, _> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| (k.clone(), i))
        .collect();
    let mut edges = Vec::new();
    let mut reverse = vec![Vec::new(); keys.len()];
    for (i, key) in keys.iter().enumerate() {
        let targets: Vec<_> = nodes[key]
            .edges
            .iter()
            .filter_map(|e| e.target.as_ref())
            .filter_map(|k| index.get(k).copied())
            .collect();
        for &j in &targets {
            reverse[j].push(i);
        }
        edges.push(targets);
    }
    let mut visited = vec![false; keys.len()];
    let mut finished = Vec::new();
    for start in 0..keys.len() {
        let mut stack = vec![(start, false)];
        while let Some((i, exit)) = stack.pop() {
            if exit {
                finished.push(i);
                continue;
            }
            if std::mem::replace(&mut visited[i], true) {
                continue;
            }
            stack.push((i, true));
            stack.extend(edges[i].iter().map(|&j| (j, false)));
        }
    }
    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut owner = vec![usize::MAX; keys.len()];
    for &start in finished.iter().rev() {
        if owner[start] != usize::MAX {
            continue;
        }
        let id = components.len();
        let mut members = Vec::new();
        let mut stack = vec![start];
        owner[start] = id;
        while let Some(i) = stack.pop() {
            members.push(i);
            for &j in &reverse[i] {
                if owner[j] == usize::MAX {
                    owner[j] = id;
                    stack.push(j);
                }
            }
        }
        members.sort_unstable();
        components.push(members);
    }
    // Kosaraju above produces source-first components: children are ready in reverse.
    let mut component_hashes = vec![[0; 32]; components.len()];
    let mut mutable = vec![false; components.len()];
    let mut complete = vec![true; components.len()];
    for id in (0..components.len()).rev() {
        let mut records = Vec::new();
        for &i in &components[id] {
            let node = &nodes[&keys[i]];
            mutable[id] |= node.mutable;
            complete[id] &= node.complete;
            let mut named = node.edges.clone();
            named.sort();
            let mut named_records = Vec::new();
            for edge in named {
                let child = edge.target.as_ref().and_then(|k| index.get(k)).copied();
                let outgoing = if let Some(child) = child {
                    let c = owner[child];
                    if c != id {
                        ensure!(c > id, "invalid component order");
                        mutable[id] |= mutable[c];
                        complete[id] &= complete[c];
                        Some(component_hashes[c])
                    } else {
                        None
                    }
                } else {
                    complete[id] = false;
                    None
                };
                named_records.push(json!([edge, outgoing]));
            }
            records.push(json!([
                keys[i],
                node.identity,
                node.metadata,
                named_records
            ]));
        }
        component_hashes[id] = hash(json!(["bundler-lock-scc-v1", records]))?;
    }
    keys.iter()
        .enumerate()
        .map(|(i, key)| {
            Ok((
                key.clone(),
                PackageFingerprint {
                    instance: hash(json!(["bundler-lock-instance-v1", nodes[key].identity]))?,
                    dependencies: hash(json!([
                        "bundler-lock-package-v1",
                        key,
                        component_hashes[owner[i]]
                    ]))?,
                    mutable: mutable[owner[i]],
                    complete: complete[owner[i]],
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aube_lockfile::{DepType, DirectDep};

    fn package(name: &str) -> LockedPackage {
        LockedPackage {
            name: name.into(),
            version: "1".into(),
            dep_path: format!("{name}@1"),
            integrity: Some(format!("sha512-{name}")),
            ..Default::default()
        }
    }

    fn fingerprints(graph: &LockfileGraph) -> eyre::Result<BTreeMap<Key, PackageFingerprint>> {
        fingerprint_nodes(&normalized_nodes(graph)?)
    }

    #[test]
    fn cycle_member_changes_reach_all_ancestors_but_not_disconnected_packages() -> eyre::Result<()>
    {
        let mut graph = LockfileGraph::default();
        for name in ["root", "a", "b", "unrelated"] {
            graph.packages.insert(format!("{name}@1"), package(name));
        }
        graph
            .packages
            .get_mut("root@1")
            .unwrap()
            .dependencies
            .insert("a".into(), "1".into());
        graph
            .packages
            .get_mut("a@1")
            .unwrap()
            .dependencies
            .insert("b".into(), "b@1".into());
        graph
            .packages
            .get_mut("b@1")
            .unwrap()
            .dependencies
            .insert("a".into(), "1".into());
        graph.importers.insert(
            ".".into(),
            vec![DirectDep {
                name: "root".into(),
                dep_path: "root@1".into(),
                dep_type: DepType::Production,
                specifier: Some("^1".into()),
            }],
        );
        let before = fingerprints(&graph)?;
        graph.packages.get_mut("b@1").unwrap().integrity = Some("changed".into());
        let after = fingerprints(&graph)?;
        for key in ["root@1", "a@1", "b@1"] {
            assert_ne!(
                before[&Key::Package(key.into())].dependencies,
                after[&Key::Package(key.into())].dependencies
            );
        }
        assert_ne!(
            before[&Key::Importer(".".into())],
            after[&Key::Importer(".".into())]
        );
        assert_eq!(
            before[&Key::Package("unrelated@1".into())],
            after[&Key::Package("unrelated@1".into())]
        );
        assert_eq!(
            before[&Key::Package("root@1".into())].instance,
            after[&Key::Package("root@1".into())].instance
        );
        assert!(after.values().all(|p| p.complete));
        Ok(())
    }

    #[test]
    fn named_edges_peer_patch_and_local_sources_are_not_interchangeable() -> eyre::Result<()> {
        let mut graph = LockfileGraph::default();
        let mut root = package("root");
        root.dependencies
            .insert("alias".into(), "child@1(peer@2)".into());
        graph.packages.insert("root@1".into(), root);
        graph
            .packages
            .insert("child@1(peer@2)".into(), package("child"));
        let before = fingerprints(&graph)?;
        graph
            .packages
            .insert("child@1(peer@3)".into(), package("child"));
        let peer_variants = fingerprints(&graph)?;
        assert_ne!(
            peer_variants[&Key::Package("child@1(peer@2)".into())].instance,
            peer_variants[&Key::Package("child@1(peer@3)".into())].instance,
        );
        assert_eq!(
            before[&Key::Package("root@1".into())],
            peer_variants[&Key::Package("root@1".into())]
        );
        let original = &before[&Key::Package("root@1".into())];
        let request = graph
            .packages
            .get_mut("root@1")
            .unwrap()
            .dependencies
            .remove("alias")
            .unwrap();
        graph
            .packages
            .get_mut("root@1")
            .unwrap()
            .dependencies
            .insert("renamed".into(), request);
        assert_ne!(
            original.dependencies,
            fingerprints(&graph)?[&Key::Package("root@1".into())].dependencies
        );
        let child = Key::Package("child@1(peer@2)".into());
        let old_instance = before[&child].instance;
        graph
            .patched_dependencies
            .insert("child@1".into(), "patches/child.patch".into());
        assert_ne!(old_instance, fingerprints(&graph)?[&child].instance);
        graph
            .packages
            .get_mut("child@1(peer@2)")
            .unwrap()
            .local_source = Some(LocalSource::Link("packages/child".into()));
        let linked = fingerprints(&graph)?;
        assert!(linked[&Key::Package("root@1".into())].mutable);
        assert_ne!(old_instance, linked[&child].instance);
        graph.packages.remove("child@1(peer@2)");
        assert!(!fingerprints(&graph)?[&Key::Package("root@1".into())].complete);
        Ok(())
    }

    const BERRY: &str = r#"
__metadata:
  version: 10
  cacheKey: 10c0
"root@workspace:.":
  version: 0.0.0-use.local
  resolution: "root@workspace:."
  dependencies:
    app: "workspace:*"
    patched: "patch:patched@npm%3A1#./one.patch"
"app@workspace:*, app@workspace:packages/app":
  version: 0.0.0-use.local
  resolution: "app@workspace:packages/app"
  dependencies:
    patched: "patch:patched@npm%3A1#./two.patch"
"patched@patch:patched@npm%3A1#./one.patch":
  version: "1"
  resolution: "patched@patch:patched@npm%3A1#./one.patch::version=1&hash=111"
  checksum: one
"patched@patch:patched@npm%3A1#./two.patch":
  version: "1"
  resolution: "patched@patch:patched@npm%3A1#./two.patch::version=1&hash=222"
  checksum: two
"unused@npm:^1":
  version: "1"
  resolution: "unused@npm:1"
  checksum: unused
"#;

    #[test]
    fn berry_preserves_workspaces_patch_instances_and_named_closures() -> eyre::Result<()> {
        let dir = tempfile::tempdir_in(std::env::current_dir()?)?;
        // No manifests exist: package/workspace identities come from the lockfile.
        std::fs::write(dir.path().join("yarn.lock"), BERRY)?;
        let before = ProjectLockfile::read(dir.path(), None)?.unwrap();
        assert_eq!(before.kind, LockfileKind::YarnBerry);
        assert_eq!(before.path, dir.path().join("yarn.lock").canonicalize()?);
        assert!(before.importer(".").unwrap().complete);
        assert!(before.importer("packages/app").unwrap().mutable);
        let one = "patched@patch:patched@npm%3A1#./one.patch::version=1&hash=111";
        let two = "patched@patch:patched@npm%3A1#./two.patch::version=1&hash=222";
        assert_ne!(
            before.package(one).unwrap().instance,
            before.package(two).unwrap().instance
        );
        std::fs::write(
            dir.path().join("yarn.lock"),
            BERRY.replace("checksum: two", "checksum: changed"),
        )?;
        let after = ProjectLockfile::read(dir.path(), Some(LockfileKind::Yarn))?.unwrap();
        assert_ne!(before.importer(".").unwrap(), after.importer(".").unwrap());
        assert_ne!(
            before.importer("packages/app").unwrap(),
            after.importer("packages/app").unwrap()
        );
        assert_eq!(before.package(one), after.package(one));
        assert_eq!(
            before.package("unused@npm:1"),
            after.package("unused@npm:1")
        );
        let mut nodes = berry_nodes(BERRY)?;
        let first = fingerprint_nodes(&nodes)?;
        for node in nodes.values_mut() {
            node.edges.reverse();
            // Object insertion order must not depend on serde_json's
            // feature-unified `preserve_order` setting.
            if let Some(object) = node.metadata.as_object_mut() {
                let entries: Vec<_> = object.clone().into_iter().rev().collect();
                object.clear();
                object.extend(entries);
            }
        }
        assert_eq!(first, fingerprint_nodes(&nodes)?);
        Ok(())
    }

    #[test]
    fn selection_no_lockfile_and_pnpm_do_not_read_local_manifests() -> eyre::Result<()> {
        let dir = tempfile::tempdir_in(std::env::current_dir()?)?;
        assert!(ProjectLockfile::read(dir.path(), None)?.is_none());
        std::fs::write(dir.path().join("yarn.lock"), BERRY)?;
        std::fs::write(
            dir.path().join("pnpm-lock.yaml"),
            r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      local:
        specifier: link:packages/local
        version: link:packages/local
"#,
        )?;
        std::fs::create_dir_all(dir.path().join("packages").join("local"))?;
        std::fs::write(
            dir.path()
                .join("packages")
                .join("local")
                .join("package.json"),
            "not JSON",
        )?;
        let pnpm = ProjectLockfile::read(dir.path(), Some(LockfileKind::Pnpm))?.unwrap();
        assert_eq!(pnpm.kind, LockfileKind::Pnpm);
        assert!(pnpm.importer(".").unwrap().mutable);
        assert_eq!(
            ProjectLockfile::read(dir.path(), Some(LockfileKind::Yarn))?
                .unwrap()
                .kind,
            LockfileKind::YarnBerry
        );
        std::fs::write(dir.path().join("yarn.lock"), "__metadata: [invalid")?;
        assert!(ProjectLockfile::read(dir.path(), Some(LockfileKind::Yarn)).is_err());
        Ok(())
    }

    #[test]
    fn unknown_catalog_and_peer_edges_are_retained_without_guessing_providers() -> eyre::Result<()>
    {
        let original = BERRY.replace(
            "  checksum: unused",
            "  checksum: unused\n  peerDependencies:\n    patched: 1\n  dependencies:\n    missing: 'catalog:shared'",
        );
        let before = fingerprint_nodes(&berry_nodes(&original)?)?;
        let key = Key::Package("unused@npm:1".into());
        assert!(!before[&key].complete);
        let changed = original.replace("catalog:shared", "catalog:other");
        let after = fingerprint_nodes(&berry_nodes(&changed)?)?;
        assert_ne!(before[&key].dependencies, after[&key].dependencies);
        assert_eq!(before[&key].instance, after[&key].instance);
        // An unrelated, same-named locked patch is not a selected peer provider.
        let nodes = berry_nodes(&original)?;
        assert!(nodes[&key].edges.iter().all(|e| e.target.is_none()));
        Ok(())
    }

    #[test]
    fn npm_dropped_required_and_importer_declarations_remain_ineligible_and_hashed(
    ) -> eyre::Result<()> {
        let dir = tempfile::tempdir_in(std::env::current_dir()?)?;
        let path = dir.path().join("package-lock.json");
        for owner in ["", "node_modules/app"] {
            let mut document = json!({
                "name": "root",
                "lockfileVersion": 3,
                "packages": {
                    "": {"name": "root", "dependencies": {"app": "1.0.0"}},
                    "node_modules/app": {"version": "1.0.0", "integrity": "sha512-app"}
                }
            });
            document["packages"][owner]["dependencies"]["missing"] = json!("^1");
            std::fs::write(&path, serde_json::to_vec(&document)?)?;
            let graph = aube_lockfile::npm::parse(&path)?;
            assert!(!graph.packages["app@1.0.0"]
                .dependencies
                .contains_key("missing"));
            assert!(graph
                .importers
                .values()
                .flatten()
                .all(|d| d.name != "missing"));
            let before = ProjectLockfile::read(dir.path(), Some(LockfileKind::Npm))?.unwrap();
            assert!(!before.package("app@1.0.0").unwrap().complete);
            assert!(!before.importer(".").unwrap().complete);
            document["packages"][owner]["dependencies"]["missing"] = json!("^2");
            std::fs::write(&path, serde_json::to_vec(&document)?)?;
            let after = ProjectLockfile::read(dir.path(), Some(LockfileKind::Npm))?.unwrap();
            assert_ne!(
                before.package("app@1.0.0").unwrap().instance,
                after.package("app@1.0.0").unwrap().instance
            );
            assert_ne!(before.importer("."), after.importer("."));
        }
        Ok(())
    }

    #[test]
    fn classic_dropped_required_importer_and_optional_edges_are_ineligible_and_hashed(
    ) -> eyre::Result<()> {
        let dir = tempfile::tempdir_in(std::env::current_dir()?)?;
        let path = dir.path().join("yarn.lock");
        let manifest_path = dir.path().join("package.json");
        let lock = r#"# yarn lockfile v1

app@1.0.0:
  version "1.0.0"
  integrity sha512-app
  dependencies:
    missing "^1"
  optionalDependencies:
    optional "^1"

optional@^1:
  version "1.0.0"
  integrity sha512-optional
"#;
        let manifest = json!({"dependencies": {"app": "1.0.0", "missing": "^1"}});
        std::fs::write(&path, lock)?;
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
        let graph = aube_lockfile::yarn::parse(&path, &serde_json::from_value(manifest.clone())?)?;
        assert!(graph.packages["app@1.0.0"].dependencies.is_empty());
        assert!(graph.packages["app@1.0.0"].optional_dependencies.is_empty());
        assert!(graph
            .importers
            .values()
            .flatten()
            .all(|d| d.name != "missing"));
        let before = ProjectLockfile::read(dir.path(), Some(LockfileKind::Yarn))?.unwrap();
        assert!(before.packages.values().all(|p| !p.complete));
        assert!(!before.importer(".").unwrap().complete);
        for changed in [
            lock.replace("missing \"^1\"", "missing \"^2\""),
            lock.replace("optional \"^1\"", "optional \"^2\""),
        ] {
            std::fs::write(&path, changed)?;
            let after = ProjectLockfile::read(dir.path(), Some(LockfileKind::Yarn))?.unwrap();
            assert_ne!(
                before.package("app@1.0.0").unwrap().instance,
                after.package("app@1.0.0").unwrap().instance
            );
        }
        std::fs::write(&path, lock)?;
        let mut changed = manifest;
        changed["dependencies"]["missing"] = json!("^2");
        std::fs::write(&manifest_path, serde_json::to_vec(&changed)?)?;
        let after = ProjectLockfile::read(dir.path(), Some(LockfileKind::Yarn))?.unwrap();
        assert_ne!(before.importer("."), after.importer("."));
        Ok(())
    }

    #[test]
    fn npm_collapsed_install_paths_never_claim_complete_or_hide_discarded_integrity(
    ) -> eyre::Result<()> {
        let dir = tempfile::tempdir_in(std::env::current_dir()?)?;
        let path = dir.path().join("package-lock.json");
        let mut document = json!({
            "name": "root",
            "lockfileVersion": 3,
            "packages": {
                "": {"name": "root", "dependencies": {"app": "1.0.0"}},
                "node_modules/app": {"version": "1.0.0", "integrity": "sha512-first"},
                "node_modules/parent/node_modules/app": {
                    "version": "1.0.0", "integrity": "sha512-second",
                    "dependencies": {"missing": "^1"}
                }
            }
        });
        std::fs::write(&path, serde_json::to_vec(&document)?)?;
        let graph = aube_lockfile::npm::parse(&path)?;
        assert_eq!(graph.packages.len(), 1);
        assert_eq!(
            graph.packages["app@1.0.0"].integrity.as_deref(),
            Some("sha512-first")
        );
        let before = ProjectLockfile::read(dir.path(), Some(LockfileKind::Npm))?.unwrap();
        assert!(!before.package("app@1.0.0").unwrap().complete);
        document["packages"]["node_modules/parent/node_modules/app"]["integrity"] =
            json!("sha512-changed");
        std::fs::write(&path, serde_json::to_vec(&document)?)?;
        let changed_graph = aube_lockfile::npm::parse(&path)?;
        assert_eq!(fingerprints(&graph)?, fingerprints(&changed_graph)?);
        let after = ProjectLockfile::read(dir.path(), Some(LockfileKind::Npm))?.unwrap();
        assert_ne!(
            before.package("app@1.0.0").unwrap().instance,
            after.package("app@1.0.0").unwrap().instance
        );
        assert_ne!(before.importer("."), after.importer("."));
        assert!(!after.package("app@1.0.0").unwrap().complete);
        Ok(())
    }

    #[test]
    fn normalized_exact_peer_requires_an_explicit_selected_provider() -> eyre::Result<()> {
        let dir = tempfile::tempdir_in(std::env::current_dir()?)?;
        let path = dir.path().join("pnpm-lock.yaml");
        let lock = r#"
lockfileVersion: '9.0'
importers:
  .:
    dependencies:
      app:
        specifier: 1.0.0
        version: 1.0.0
packages:
  app@1.0.0:
    peerDependencies:
      peer: 1.0.0
  peer@1.0.0:
    resolution: {integrity: sha512-peer}
snapshots:
  app@1.0.0: {}
  peer@1.0.0: {}
"#;
        std::fs::write(&path, lock)?;
        let graph = aube_lockfile::pnpm::parse(&path)?;
        let nodes = normalized_nodes(&graph)?;
        let key = Key::Package("app@1.0.0".into());
        assert_eq!(nodes[&key].edges.len(), 1);
        assert_eq!(nodes[&key].edges[0].role, "peer");
        assert!(nodes[&key].edges[0].target.is_none());
        let before = ProjectLockfile::read(dir.path(), Some(LockfileKind::Pnpm))?.unwrap();
        assert!(!before.package("app@1.0.0").unwrap().complete);
        assert!(!before.importer(".").unwrap().complete);
        std::fs::write(&path, lock.replace("sha512-peer", "sha512-changed"))?;
        let after = ProjectLockfile::read(dir.path(), Some(LockfileKind::Pnpm))?.unwrap();
        assert_eq!(before.package("app@1.0.0"), after.package("app@1.0.0"));
        assert_ne!(before.package("peer@1.0.0"), after.package("peer@1.0.0"));
        std::fs::write(
            &path,
            lock.replace(
                "app@1.0.0: {}",
                "app@1.0.0:\n    dependencies:\n      peer: 1.0.0",
            ),
        )?;
        let selected = ProjectLockfile::read(dir.path(), Some(LockfileKind::Pnpm))?.unwrap();
        assert!(selected.package("app@1.0.0").unwrap().complete);
        assert!(selected.importer(".").unwrap().complete);
        Ok(())
    }

    /// Opt-in read-only compatibility probe; never embeds project lockfile data.
    #[test]
    #[ignore = "set BUNDLER_LOCKFILE_PROJECT to a local Yarn Berry project"]
    fn actual_project_lockfile() -> eyre::Result<()> {
        let project = std::env::var_os("BUNDLER_LOCKFILE_PROJECT")
            .ok_or_eyre("set BUNDLER_LOCKFILE_PROJECT")?;
        let lock = ProjectLockfile::read(Path::new(&project), Some(LockfileKind::Yarn))?.unwrap();
        assert_eq!(lock.kind, LockfileKind::YarnBerry);
        assert!(lock.packages.len() > 100);
        assert!(lock.importers.len() > 1);
        assert!(lock.packages.keys().any(|key| key.contains("@patch:")));
        assert!(lock.packages.keys().any(|key| key.contains("@npm:")));
        assert!(lock.importer(".").is_some());
        eprintln!(
            "parsed {} package instances and {} workspace importers ({} incomplete closures)",
            lock.packages.len(),
            lock.importers.len(),
            lock.packages.values().filter(|p| !p.complete).count()
        );
        Ok(())
    }
}
