//! Exact synchronous reachability over retention constraints, not execution order.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eyre::{ensure, OptionExt};
use source_graph::{Activation, GraphNode, GraphRevision, SourceGraph};

use crate::{policy::EffectPolicy, RetentionEdgeKind, RetentionPlan};

#[path = "root_mask.rs"]
mod root_mask;
pub use root_mask::{RootMask, RootMaskIter};

/// Source occurrence identity, independent of file/edge insertion order.
/// Offsets identify sites only within the stamped source graph revision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AsyncSiteId {
    pub module: PathBuf,
    pub offset: u32,
    /// A context candidate, still scoped by its real source occurrence.
    pub candidate: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RootId {
    Entry(String),
    Async(AsyncSiteId),
    /// One requested export, not the union of an occurrence's selected names.
    AsyncExport {
        site: AsyncSiteId,
        name: String,
    },
}

impl RootId {
    pub fn async_site(&self) -> Option<&AsyncSiteId> {
        match self {
            Self::Entry(_) => None,
            Self::Async(site) | Self::AsyncExport { site, .. } => Some(site),
        }
    }
}

/// Canonical diagnostic identity; graph-local numeric IDs never name chunks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeId {
    Module(PathBuf),
    Segment {
        module: PathBuf,
        index: u32,
    },
    External {
        module: PathBuf,
        offset: u32,
    },
    /// None initializes the proxy/default; Some is one requested named snapshot.
    CommonJsProxy {
        module: PathBuf,
        name: Option<String>,
    },
    CommonJsNamespace(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootSeed {
    pub id: RootId,
    pub evaluation_roots: Vec<u32>,
    /// A module binding root requests its namespace, unlike evaluation alone.
    pub binding_roots: Vec<GraphNode>,
}

/// One root's canonical node order, sharing path-bearing identities across roots.
#[derive(Clone)]
pub struct ReachedNodes {
    nodes: Arc<[NodeId]>,
    indices: Box<[usize]>,
}

impl PartialEq for ReachedNodes {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.nodes, &other.nodes) {
            self.indices == other.indices
        } else {
            self.iter().eq(other.iter())
        }
    }
}

impl Eq for ReachedNodes {}

impl ReachedNodes {
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &NodeId> + DoubleEndedIterator {
        self.indices.iter().map(|&index| &self.nodes[index])
    }

    pub fn contains(&self, node: &NodeId) -> bool {
        self.nodes
            .binary_search(node)
            .is_ok_and(|index| self.indices.binary_search(&index).is_ok())
    }

    pub fn len(&self) -> usize {
        self.indices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

impl std::fmt::Debug for ReachedNodes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_set().entries(self.iter()).finish()
    }
}

impl<'a> IntoIterator for &'a ReachedNodes {
    type Item = &'a NodeId;
    type IntoIter = ReachedNodeIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        ReachedNodeIter {
            nodes: &self.nodes,
            indices: self.indices.iter(),
        }
    }
}

pub struct ReachedNodeIter<'a> {
    nodes: &'a [NodeId],
    indices: std::slice::Iter<'a, usize>,
}

impl<'a> Iterator for ReachedNodeIter<'a> {
    type Item = &'a NodeId;

    fn next(&mut self) -> Option<Self::Item> {
        self.indices.next().map(|&index| &self.nodes[index])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.indices.size_hint()
    }
}

impl DoubleEndedIterator for ReachedNodeIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.indices.next_back().map(|&index| &self.nodes[index])
    }
}

impl ExactSizeIterator for ReachedNodeIter<'_> {}

pub(crate) fn add_reachability_roots(
    reachability: &BTreeMap<RootId, ReachedNodes>,
    node_roots: &mut BTreeMap<NodeId, RootMask>,
) {
    let stage = crate::planning::Stage::new("root_mask_inversion");
    let Some(first) = reachability.values().next() else {
        stage.finish();
        return;
    };
    let roots: Arc<[RootId]> = reachability.keys().cloned().collect();
    let mut counts = vec![0; first.nodes.len()];
    for reached in reachability.values() {
        for &index in &reached.indices {
            counts[index] += 1;
        }
    }
    let mut masks: Vec<_> = counts.into_iter().map(Vec::with_capacity).collect();
    // Canonical root order makes every inverted mask sorted without tree nodes.
    for (root, reached) in reachability.values().enumerate() {
        for &index in &reached.indices {
            masks[index].push(root);
        }
    }
    let mut shared = HashSet::<Arc<[usize]>>::new();
    for (node, mask) in first.nodes.iter().zip(masks) {
        let indices = if let Some(indices) = shared.get(mask.as_slice()) {
            indices.clone()
        } else {
            let indices: Arc<[usize]> = mask.into();
            shared.insert(indices.clone());
            indices
        };
        node_roots.insert(node.clone(), RootMask::new(roots.clone(), indices));
    }
    tracing::info!(
        target: "bundler::progress",
        roots = reachability.len(),
        nodes = first.nodes.len(),
        memberships = reachability.values().map(ReachedNodes::len).sum::<usize>(),
        "Root mask counts"
    );
    stage.finish();
}

#[derive(PartialEq, Eq, Hash)]
enum ReachabilityKey {
    Source(GraphNode),
    CommonJsProxy(u32, Option<String>),
    CommonJsNamespace(u32),
}

#[derive(Default)]
struct ReachabilityNodes {
    indices: HashMap<ReachabilityKey, usize>,
    nodes: Vec<NodeId>,
}

impl ReachabilityNodes {
    fn intern(&mut self, graph: &SourceGraph, key: ReachabilityKey) -> eyre::Result<usize> {
        if let Some(&index) = self.indices.get(&key) {
            return Ok(index);
        }
        let node = match &key {
            ReachabilityKey::Source(node) => node_id(graph, *node)?,
            ReachabilityKey::CommonJsProxy(file, name) => NodeId::CommonJsProxy {
                module: graph
                    .file_path(*file)
                    .ok_or_eyre("missing CJS module")?
                    .to_owned(),
                name: name.clone(),
            },
            ReachabilityKey::CommonJsNamespace(file) => NodeId::CommonJsNamespace(
                graph
                    .file_path(*file)
                    .ok_or_eyre("missing CJS module")?
                    .to_owned(),
            ),
        };
        let index = self.nodes.len();
        self.nodes.push(node);
        self.indices.insert(key, index);
        Ok(index)
    }

    fn finish(self, roots: BTreeMap<RootId, Vec<usize>>) -> BTreeMap<RootId, ReachedNodes> {
        drop(self.indices);
        // Discovery uses graph-local handles; only canonical NodeId order escapes.
        let mut ordered: Vec<_> = self.nodes.into_iter().enumerate().collect();
        ordered.sort_unstable_by(|(_, left), (_, right)| left.cmp(right));
        let mut canonical = vec![0; ordered.len()];
        let nodes: Arc<[NodeId]> = ordered
            .into_iter()
            .enumerate()
            .map(|(index, (original, node))| {
                canonical[original] = index;
                node
            })
            .collect();
        roots
            .into_iter()
            .map(|(root, mut indices)| {
                for index in &mut indices {
                    *index = canonical[*index];
                }
                indices.sort_unstable();
                (
                    root,
                    ReachedNodes {
                        nodes: Arc::clone(&nodes),
                        indices: indices.into_boxed_slice(),
                    },
                )
            })
            .collect()
    }
}

pub fn node_id(graph: &SourceGraph, node: GraphNode) -> eyre::Result<NodeId> {
    let file = match node {
        GraphNode::Module(file) => file,
        GraphNode::Segment(key) => key.file_id,
        GraphNode::Unlinked { file_id, .. } => file_id,
    };
    let module = graph
        .file_path(file)
        .ok_or_eyre("missing chunk node module")?
        .to_owned();
    Ok(match node {
        GraphNode::Module(_) => NodeId::Module(module),
        GraphNode::Segment(key) => {
            ensure!(
                graph
                    .file_segments(file)
                    .is_some_and(|s| (key.segment_idx as usize) < s.len()),
                "missing chunk segment"
            );
            NodeId::Segment {
                module,
                index: key.segment_idx,
            }
        }
        GraphNode::Unlinked { dependency, .. } => {
            ensure!(
                matches!(
                    graph.dependency_target(file, dependency),
                    Some(source_graph::DependencyTarget::External(_))
                ),
                "chunk node is not a resolved external site"
            );
            NodeId::External {
                module,
                offset: dependency.0,
            }
        }
    })
}

pub fn async_seed(graph: &SourceGraph, edge_index: usize) -> eyre::Result<RootSeed> {
    let edge = graph
        .edges()
        .get(edge_index)
        .ok_or_eyre("missing async occurrence")?;
    ensure!(
        edge.activation == Activation::Asynchronous && !edge.type_only,
        "not a runtime async occurrence"
    );
    let dependency = edge
        .dependency
        .ok_or_eyre("async occurrence has no site identity")?;
    let module = match node_id(graph, edge.from)? {
        NodeId::Module(path) | NodeId::Segment { module: path, .. } => path,
        NodeId::External { .. } | NodeId::CommonJsProxy { .. } | NodeId::CommonJsNamespace(_) => {
            eyre::bail!("async consumer must be a source node")
        }
    };
    let file = graph
        .file_id(&module)
        .ok_or_eyre("missing async consumer")?;
    let candidate = if matches!(
        graph.dependency_target(file, dependency),
        Some(source_graph::DependencyTarget::Context(_))
    ) {
        let NodeId::Module(path) = node_id(graph, edge.to)? else {
            eyre::bail!("context candidate must be a source module");
        };
        Some(path)
    } else {
        None
    };
    Ok(RootSeed {
        id: RootId::Async(AsyncSiteId {
            module,
            offset: dependency.0,
            candidate,
        }),
        evaluation_roots: vec![],
        binding_roots: vec![edge.to],
    })
}

/// Independent export roots sharing one activation occurrence. Every selected
/// route carries the original import target's evaluation, including sibling effects.
/// Unsafe/unknown namespace observations keep the conservative namespace root.
pub fn async_demand_seeds(graph: &SourceGraph, edge_index: usize) -> eyre::Result<Vec<RootSeed>> {
    let original = async_seed(graph, edge_index)?;
    let Some(demand) = graph.dynamic_import_demand(edge_index) else {
        return Ok(vec![original]);
    };
    let Some(bindings) = demand.bindings else {
        return Ok(vec![original]);
    };
    let GraphNode::Module(target) = demand.evaluation.to else {
        eyre::bail!("precise dynamic evaluation must target a local module");
    };
    let mut routes = BTreeMap::<String, Vec<GraphNode>>::new();
    for binding in bindings {
        let Some(ast_segmenter::owned::ImportedName::Name(name)) = binding.imported else {
            eyre::bail!("precise dynamic route requires a requested export name");
        };
        let route = routes.entry(name).or_default();
        if !route.contains(&binding.to) {
            route.push(binding.to);
        }
    }
    if routes.is_empty() {
        return Ok(vec![RootSeed {
            evaluation_roots: vec![target],
            binding_roots: vec![],
            ..original
        }]);
    }
    let RootId::Async(site) = original.id else {
        unreachable!("async_seed always supplies an occurrence identity")
    };
    Ok(routes
        .into_iter()
        .map(|(name, binding_roots)| RootSeed {
            id: RootId::AsyncExport {
                site: site.clone(),
                name,
            },
            evaluation_roots: vec![target],
            binding_roots,
        })
        .collect())
}

/// Deliberately simple reference oracle: one independent flood fill per root.
/// Deferred synchronous dependencies supply code, not permission to run it.
pub fn reference_reachability(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    policy: &EffectPolicy,
    roots: &[RootSeed],
) -> eyre::Result<BTreeMap<RootId, BTreeSet<NodeId>>> {
    Ok(compact_reachability(graph, retention, policy, roots)?
        .into_iter()
        .map(|(root, nodes)| (root, nodes.iter().cloned().collect()))
        .collect())
}

pub(crate) fn compact_reachability(
    graph: &SourceGraph,
    retention: &RetentionPlan,
    policy: &EffectPolicy,
    roots: &[RootSeed],
) -> eyre::Result<BTreeMap<RootId, ReachedNodes>> {
    let stage = crate::planning::Stage::new("compact_reachability");
    ensure!(
        retention.is_current(graph, policy),
        "stale retention revision or effect policy"
    );
    let mut outgoing = HashMap::<_, Vec<_>>::new();
    let namespaces = NamespaceEdges::new(retention);
    for edge in &retention.edges {
        if !matches!(
            edge.kind,
            RetentionEdgeKind::EffectOrder | RetentionEdgeKind::Namespace
        ) {
            outgoing.entry(edge.from).or_default().push(edge);
        }
    }
    let retained_segments: HashSet<_> = retention.segments.iter().map(|s| s.key).collect();
    let retained_modules: HashSet<_> = retention.modules.iter().copied().collect();
    let export_names =
        crate::link::namespace_export_names(graph, |_| BTreeSet::from(["default".into()]));
    let cjs_files: Vec<_> = retention
        .modules
        .iter()
        .copied()
        .filter(|file| {
            graph
                .file_facts(*file)
                .is_some_and(|facts| facts.commonjs())
        })
        .collect();
    let cjs_requests: HashMap<_, _> = if cjs_files.is_empty() || roots.is_empty() {
        HashMap::new()
    } else {
        let requests = crate::cjs::Requests::new(graph, retention, None);
        cjs_files
            .iter()
            .map(|&file| (file, requests.requests(file)))
            .collect()
    };
    let mut result = BTreeMap::new();
    let mut identities = ReachabilityNodes::default();
    let mut memberships = 0usize;
    let mut progress = Instant::now();
    for root in roots {
        ensure!(!result.contains_key(&root.id), "duplicate root identity");
        let mut export_requests = BTreeSet::new();
        let mut namespace_modules: Vec<_> = root
            .binding_roots
            .iter()
            .filter_map(|node| match node {
                GraphNode::Module(file) => Some(*file),
                _ => None,
            })
            .collect();
        let mut queue: VecDeque<_> = root
            .evaluation_roots
            .iter()
            .copied()
            .map(GraphNode::Module)
            .collect();
        for &node in &root.binding_roots {
            enqueue_binding(&namespaces, node, &mut queue);
        }
        let mut visited = HashSet::new();
        let mut reached = Vec::new();
        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }
            reached.push(identities.intern(graph, ReachabilityKey::Source(node))?);
            ensure!(
                match node {
                    GraphNode::Module(file) => retained_modules.contains(&file),
                    GraphNode::Segment(key) => retained_segments.contains(&key),
                    GraphNode::Unlinked { .. } => true,
                },
                "root or dependency is absent from retention"
            );
            for edge in outgoing.get(&node).into_iter().flatten() {
                if let Some(index) = edge.source_edge {
                    let source = graph
                        .edges()
                        .get(index)
                        .ok_or_eyre("missing retained occurrence")?;
                    if source.activation == Activation::Asynchronous || source.type_only {
                        continue;
                    }
                }
                if matches!(
                    edge.kind,
                    RetentionEdgeKind::Binding | RetentionEdgeKind::Require
                ) {
                    enqueue_binding(&namespaces, edge.to, &mut queue);
                    if let GraphNode::Module(file) = edge.to {
                        namespace_modules.push(file);
                    }
                } else {
                    queue.push_back(edge.to);
                }
            }
        }
        // Checked namespace routes can bypass import declarations. Preserve their
        // named CJS snapshots without traversing a connector's sibling bindings.
        // Track binding demands, not merely reached module-evaluation nodes.
        let mut seen_namespaces = HashSet::new();
        while let Some(file) = namespace_modules.pop() {
            if !seen_namespaces.insert(file) || graph.file_facts(file).unwrap().commonjs() {
                continue;
            }
            for name in &export_names[&file] {
                commonjs_export_requests(graph, file, name, &mut export_requests);
            }
            namespace_modules.extend(namespaces.targets(file).filter_map(|target| match target {
                GraphNode::Module(target) => Some(target),
                _ => None,
            }));
        }
        for &file in &cjs_files {
            let requests = &cjs_requests[&file];
            let mut demanded: BTreeSet<_> = export_requests
                .iter()
                .filter(|(target, _)| *target == file)
                .map(|(_, name)| name.clone())
                .collect();
            let direct_entry = root.evaluation_roots.contains(&file)
                || root.binding_roots.iter().any(|node| match node {
                    GraphNode::Module(target) => *target == file,
                    GraphNode::Segment(key) => key.file_id == file,
                    GraphNode::Unlinked { .. } => false,
                });
            let mut initialize = direct_entry || !demanded.is_empty();
            let mut namespace = direct_entry;
            if root.binding_roots.contains(&GraphNode::Module(file)) {
                demanded.insert("default".into());
                demanded.extend(requests.iter().filter_map(|(_, request)| match request {
                    ast_segmenter::owned::ImportedName::Name(name) => Some(name.clone()),
                    ast_segmenter::owned::ImportedName::Namespace => None,
                }));
            }
            for (from, request) in requests {
                if !visited.contains(from) {
                    continue;
                }
                initialize = true;
                match request {
                    ast_segmenter::owned::ImportedName::Name(name) => {
                        demanded.insert(name.clone());
                    }
                    ast_segmenter::owned::ImportedName::Namespace => namespace = true,
                }
            }
            if initialize {
                reached.push(identities.intern(graph, ReachabilityKey::CommonJsProxy(file, None))?);
                for name in demanded.into_iter().filter(|name| name != "default") {
                    reached.push(
                        identities
                            .intern(graph, ReachabilityKey::CommonJsProxy(file, Some(name)))?,
                    );
                }
                if namespace {
                    reached
                        .push(identities.intern(graph, ReachabilityKey::CommonJsNamespace(file))?);
                }
            }
        }
        memberships += reached.len();
        result.insert(root.id.clone(), reached);
        if progress.elapsed() >= Duration::from_secs(1) {
            tracing::info!(
                target: "bundler::progress",
                roots = result.len(),
                total_roots = roots.len(),
                nodes = identities.nodes.len(),
                memberships,
                "Reachability progress"
            );
            progress = Instant::now();
        }
    }
    tracing::info!(
        target: "bundler::progress",
        roots = result.len(),
        nodes = identities.nodes.len(),
        memberships,
        "Reachability counts"
    );
    let canonical = crate::planning::Stage::new("reachability_canonicalization");
    let result = identities.finish(result);
    canonical.finish();
    stage.finish();
    Ok(result)
}

fn commonjs_export_requests(
    graph: &SourceGraph,
    file: u32,
    name: &str,
    requests: &mut BTreeSet<(u32, String)>,
) {
    // Missing/ambiguous implicit star exports are excluded from namespace
    // surfaces by retention too; explicit invalid exports already fail linking.
    if let Ok(alternatives) =
        graph.resolve_export_alternatives(file, name, source_graph::Namespace::Value)
    {
        for linked in alternatives {
            if let source_graph::ExportIdentity::CommonJs(target, name) = linked.identity {
                requests.insert((target, name));
            }
        }
    }
}

pub(crate) struct NamespaceEdges<'a> {
    outgoing: HashMap<u32, Vec<&'a crate::RetentionEdge>>,
    #[cfg(test)]
    edge_visits: std::cell::Cell<usize>,
}

impl<'a> NamespaceEdges<'a> {
    pub(crate) fn new(retention: &'a RetentionPlan) -> Self {
        let mut outgoing = HashMap::<_, Vec<_>>::new();
        #[cfg(test)]
        let mut edge_visits = 0;
        for edge in &retention.edges {
            #[cfg(test)]
            {
                edge_visits += 1;
            }
            if let GraphNode::Module(file) = edge.from {
                if edge.kind == RetentionEdgeKind::Namespace {
                    outgoing.entry(file).or_default().push(edge);
                }
            }
        }
        Self {
            outgoing,
            #[cfg(test)]
            edge_visits: std::cell::Cell::new(edge_visits),
        }
    }

    fn targets(&self, file: u32) -> impl Iterator<Item = GraphNode> + '_ {
        self.outgoing.get(&file).into_iter().flatten().map(|edge| {
            #[cfg(test)]
            self.edge_visits.set(self.edge_visits.get() + 1);
            edge.to
        })
    }
}

pub(crate) fn enqueue_binding(
    namespaces: &NamespaceEdges<'_>,
    node: GraphNode,
    queue: &mut VecDeque<GraphNode>,
) {
    queue.push_back(node);
    let mut pending = vec![node];
    let mut seen = HashSet::new();
    while let Some(module @ GraphNode::Module(file)) = pending.pop() {
        if !seen.insert(module) {
            continue;
        }
        for target in namespaces.targets(file) {
            queue.push_back(target);
            if matches!(target, GraphNode::Module(_)) {
                pending.push(target);
            }
        }
    }
}

/// The exact nonempty root set is the logical identity, not a filename/ordinal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkId(pub RootMask);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    pub revision: GraphRevision,
    pub policy_fingerprint: [u8; 32],
    pub retention: RetentionPlan,
    /// Graph-local seeds, retained for facade construction at this revision.
    pub roots: BTreeMap<RootId, RootSeed>,
    pub reachability: BTreeMap<RootId, ReachedNodes>,
    /// Includes dead source modules/segments with empty sets.
    pub node_roots: BTreeMap<NodeId, RootMask>,
    /// Exactly one chunk per nonempty set, without merging or hoisting.
    pub chunks: BTreeMap<ChunkId, Vec<NodeId>>,
    /// Derived reference occurrences, never source dependency IDs or cached facts.
    pub generated_boundaries: BTreeMap<AsyncSiteId, GeneratedBoundary>,
    /// Select checked per-export demands rather than complete async namespaces.
    pub precise_dynamic_imports: bool,
    /// One original import activation projects onto several independent roots.
    pub async_activations: BTreeMap<AsyncSiteId, AsyncActivation>,
    /// Checked pre-retention obligation overlays; native reconstruction is gated.
    pub lazy_modules: crate::lazy_modules::Certification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncActivation {
    /// Original source import for diagnostics, not necessarily a retained edge.
    pub source_edge: usize,
    pub origin: AsyncOrigin,
    pub roots: BTreeSet<RootId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncOrigin {
    Import,
    /// Real extraction identities, never manufactured DependencyIds.
    Lazy {
        declaration: source_graph::SegmentKey,
        module_candidate: u32,
        selector: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedBoundary {
    pub source_edge: usize,
    /// Index into the consumer module's checked reference occurrences.
    pub reference: u32,
    pub target: source_graph::SegmentKey,
}

impl ChunkPlan {
    pub fn is_current(&self, graph: &SourceGraph, policy: &EffectPolicy) -> bool {
        self.revision == graph.revision()
            && self.policy_fingerprint == policy.fingerprint()
            && self.retention.is_current(graph, policy)
            && self.lazy_modules.has_provenance(graph, policy)
    }

    pub fn is_current_with_options(
        &self,
        graph: &SourceGraph,
        policy: &EffectPolicy,
        options: &crate::build::OutputOptions,
    ) -> bool {
        self.is_current(graph, policy)
            && self
                .lazy_modules
                .is_current(graph, policy, &options.lazy_runtimes)
    }

    pub fn chunk_for(&self, node: &NodeId) -> Option<ChunkId> {
        self.node_roots
            .get(node)
            .filter(|roots| !roots.is_empty())
            .cloned()
            .map(ChunkId)
    }
}

/// Activate only reachable async occurrences, recomputing P4 retention as new
/// checked export demands appear. No parent root crosses an asynchronous edge.
pub fn partition(
    graph: &SourceGraph,
    entries: &[RootSeed],
    policy: &EffectPolicy,
) -> eyre::Result<ChunkPlan> {
    partition_precise(graph, entries, policy)
}

/// Conservative namespace reference for callers comparing demand policies.
pub fn partition_conservative(
    graph: &SourceGraph,
    entries: &[RootSeed],
    policy: &EffectPolicy,
) -> eyre::Result<ChunkPlan> {
    partition_impl(graph, entries, policy, false, &[])
}

/// Plan independent requested-export roots with shared occurrence activation.
pub fn partition_precise(
    graph: &SourceGraph,
    entries: &[RootSeed],
    policy: &EffectPolicy,
) -> eyre::Result<ChunkPlan> {
    partition_impl(graph, entries, policy, true, &[])
}

fn partition_impl(
    graph: &SourceGraph,
    entries: &[RootSeed],
    policy: &EffectPolicy,
    precise_dynamic_imports: bool,
    lazy_runtimes: &[crate::lazy_modules::LazyRuntime],
) -> eyre::Result<ChunkPlan> {
    let stage = crate::planning::Stage::new("partition");
    graph.validate_links()?;
    let lazy_modules = crate::lazy_modules::certify(graph, policy, lazy_runtimes)?;
    let ecco_effects = crate::effect_analysis::analyze(graph, policy)?;
    let mut roots = BTreeMap::new();
    let mut async_activations = BTreeMap::<AsyncSiteId, AsyncActivation>::new();
    for entry in entries {
        ensure!(
            matches!(entry.id, RootId::Entry(_)),
            "initial roots must be entries"
        );
        ensure!(
            !entry.evaluation_roots.is_empty() || !entry.binding_roots.is_empty(),
            "entry requires an evaluation or binding root"
        );
        ensure!(
            !entry
                .binding_roots
                .iter()
                .any(|n| matches!(n, GraphNode::Unlinked { .. })),
            "entry cannot bind an external site"
        );
        ensure!(
            roots.insert(entry.id.clone(), entry.clone()).is_none(),
            "duplicate entry identity"
        );
    }
    let mut round = 0usize;
    let (retention, reachability) = loop {
        round += 1;
        tracing::info!(
            target: "bundler::progress",
            round,
            roots = roots.len(),
            "Partition round begin"
        );
        let evaluation: Vec<_> = roots
            .values()
            .flat_map(|r| r.evaluation_roots.iter().copied())
            .collect();
        let bindings: Vec<_> = roots
            .values()
            .flat_map(|r| r.binding_roots.iter().copied())
            .filter(|n| !matches!(n, GraphNode::Unlinked { .. }))
            .collect();
        let retention = crate::retain_with_effects(
            graph,
            &evaluation,
            &bindings,
            policy,
            &ecco_effects,
            precise_dynamic_imports,
            Some(&lazy_modules),
        )?;
        let seeds: Vec<_> = roots.values().cloned().collect();
        let reached = compact_reachability(graph, &retention, policy, &seeds)?;
        let mut added = false;
        for &index in &retention.async_boundaries {
            let consumer = node_id(graph, graph.edges()[index].from)?;
            if !reached.values().any(|nodes| nodes.contains(&consumer)) {
                continue;
            }
            let original = async_seed(graph, index)?;
            let RootId::Async(site) = original.id.clone() else {
                unreachable!()
            };
            let seeds = if precise_dynamic_imports {
                async_demand_seeds(graph, index)?
            } else {
                vec![original]
            };
            let activation = AsyncActivation {
                source_edge: index,
                origin: AsyncOrigin::Import,
                roots: seeds.iter().map(|seed| seed.id.clone()).collect(),
            };
            if let Some(existing) = async_activations.get(&site) {
                ensure!(*existing == activation, "conflicting async activation");
            } else {
                async_activations.insert(site, activation);
            }
            for seed in seeds {
                if let Some(existing) = roots.get(&seed.id) {
                    ensure!(*existing == seed, "conflicting async site identity");
                } else {
                    roots.insert(seed.id.clone(), seed);
                    added = true;
                }
            }
        }
        for module in lazy_modules.modules() {
            for selector in &module.selectors {
                let consumer = node_id(graph, GraphNode::Segment(selector.owner))?;
                // Residual argument effects do not demand the wrapper's value.
                let value = retention
                    .segments
                    .iter()
                    .find(|segment| segment.key == selector.owner)
                    .is_some_and(|segment| {
                        selector.value_demanded(
                            segment.reasons.iter().any(|reason| reason.demands_value()),
                        )
                    });
                if !value || !reached.values().any(|nodes| nodes.contains(&consumer)) {
                    continue;
                }
                let site = AsyncSiteId {
                    module: graph
                        .file_path(selector.owner.file_id)
                        .ok_or_eyre("missing lazy selector module")?
                        .to_owned(),
                    offset: selector.construction.start,
                    candidate: None,
                };
                let seed = RootSeed {
                    id: RootId::AsyncExport {
                        site: site.clone(),
                        name: selector.property.clone(),
                    },
                    evaluation_roots: vec![module.target],
                    binding_roots: selector.selected.route.clone(),
                };
                let activation = AsyncActivation {
                    source_edge: module.source_edge,
                    origin: AsyncOrigin::Lazy {
                        declaration: module.owner,
                        module_candidate: module.candidate,
                        selector: selector.candidate,
                    },
                    roots: BTreeSet::from([seed.id.clone()]),
                };
                if let Some(existing) = async_activations.get(&site) {
                    ensure!(*existing == activation, "conflicting lazy activation");
                } else {
                    async_activations.insert(site, activation);
                }
                if let Some(existing) = roots.get(&seed.id) {
                    ensure!(*existing == seed, "conflicting lazy demand");
                } else {
                    roots.insert(seed.id.clone(), seed);
                    added = true;
                }
            }
        }
        if !added {
            break (retention, reached);
        }
    };
    let initialization = crate::planning::Stage::new("node_root_initialization");
    let mut node_roots: BTreeMap<_, RootMask> = BTreeMap::new();
    let empty = RootMask::default();
    for file in graph.module_ids() {
        if let Some(segments) = graph.file_segments(file) {
            node_roots
                .entry(node_id(graph, GraphNode::Module(file))?)
                .or_insert_with(|| empty.clone());
            for index in 0..segments.len() {
                node_roots
                    .entry(node_id(
                        graph,
                        GraphNode::Segment(source_graph::SegmentKey::new(file, index as u32)),
                    )?)
                    .or_insert_with(|| empty.clone());
            }
        }
    }
    initialization.finish();
    add_reachability_roots(&reachability, &mut node_roots);
    let grouping = crate::planning::Stage::new("chunk_grouping");
    let mut chunks: BTreeMap<ChunkId, Vec<NodeId>> = BTreeMap::new();
    for (node, roots) in &node_roots {
        if !roots.is_empty() {
            chunks
                .entry(ChunkId(roots.clone()))
                .or_default()
                .push(node.clone());
        }
    }
    tracing::info!(
        target: "bundler::progress",
        rounds = round,
        roots = roots.len(),
        nodes = node_roots.len(),
        distinct_masks = chunks.len(),
        "Partition counts"
    );
    grouping.finish();
    stage.finish();
    Ok(ChunkPlan {
        revision: graph.revision(),
        policy_fingerprint: policy.fingerprint(),
        retention,
        roots,
        reachability,
        node_roots,
        chunks,
        generated_boundaries: BTreeMap::new(),
        precise_dynamic_imports,
        async_activations,
        lazy_modules,
    })
}

/// Partition with explicit lazy certification and opt-in private-function suspension.
/// Source facts and the ordinary partition remain independent of this policy.
pub fn partition_with_options(
    graph: &SourceGraph,
    entries: &[RootSeed],
    policy: &EffectPolicy,
    options: &crate::build::OutputOptions,
) -> eyre::Result<ChunkPlan> {
    let plan = partition_impl(graph, entries, policy, true, &options.lazy_runtimes)?;
    if options.synthesize_async_boundaries {
        crate::async_boundaries::extract(graph, &plan, policy)
    } else {
        Ok(plan)
    }
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
