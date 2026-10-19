//! Logical availability/identity manifest, not physical files or an evaluation schedule.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use eyre::{ensure, OptionExt};
use source_graph::{GraphRevision, SourceGraph};

use crate::chunks::{
    async_seed, enqueue_binding, node_id, AsyncSiteId, ChunkId, ChunkPlan, NodeId, RootId, RootSeed,
};
use crate::{policy::EffectPolicy, RetentionEdgeKind};

/// Facades are logical routes, not additional members of logical chunks.
/// Module identities occupy the same chunk as the corresponding module node.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SyntheticNode {
    EntryFacade(String),
    AsyncFacade(AsyncSiteId),
    ModuleIdentity {
        module: PathBuf,
        chunk: ChunkId,
    },
    CommonJsProxy {
        module: PathBuf,
        name: Option<String>,
        chunk: ChunkId,
    },
    CommonJsNamespace {
        module: PathBuf,
        chunk: ChunkId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacadeDemand {
    /// Initial requested module identities only, not a transitive runtime order.
    pub evaluation_modules: BTreeSet<PathBuf>,
    pub binding_nodes: BTreeSet<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facade {
    pub synthetic: SyntheticNode,
    pub demand: FacadeDemand,
    /// Logical code availability, not a demand-minimal physical file load set.
    pub load_chunks: BTreeSet<ChunkId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsyncFacade {
    pub consumer: NodeId,
    pub target: NodeId,
    pub facade: Facade,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChunkDependency {
    /// Consumer → synchronous code dependency; not a module execution edge.
    pub from: ChunkId,
    pub to: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticManifest {
    pub revision: GraphRevision,
    pub policy_fingerprint: [u8; 32],
    pub chunks: BTreeMap<ChunkId, Vec<NodeId>>,
    pub entries: BTreeMap<String, Facade>,
    pub async_facades: BTreeMap<AsyncSiteId, AsyncFacade>,
    pub chunk_dependencies: BTreeSet<ChunkDependency>,
    pub synthetic_nodes: BTreeSet<SyntheticNode>,
}

impl DiagnosticManifest {
    /// Like retention, manifests are local to the same graph instance.
    pub fn is_current(&self, graph: &SourceGraph, policy: &EffectPolicy) -> bool {
        self.revision == graph.revision() && self.policy_fingerprint == policy.fingerprint()
    }
}

pub fn diagnostic_manifest(
    graph: &SourceGraph,
    plan: &ChunkPlan,
    policy: &EffectPolicy,
) -> eyre::Result<DiagnosticManifest> {
    ensure!(
        plan.is_current(graph, policy),
        "stale partition revision or effect policy"
    );
    let mut manifest = DiagnosticManifest {
        revision: plan.revision,
        policy_fingerprint: plan.policy_fingerprint,
        chunks: plan.chunks.clone(),
        entries: BTreeMap::new(),
        async_facades: BTreeMap::new(),
        chunk_dependencies: BTreeSet::new(),
        synthetic_nodes: BTreeSet::new(),
    };
    for (root, seed) in &plan.roots {
        let synthetic = match root {
            RootId::Entry(id) => SyntheticNode::EntryFacade(id.clone()),
            RootId::Async(site) => SyntheticNode::AsyncFacade(site.clone()),
        };
        let facade = Facade {
            synthetic: synthetic.clone(),
            demand: facade_demand(graph, seed)?,
            load_chunks: plan.reachability[root]
                .iter()
                .filter_map(|n| plan.chunk_for(n))
                .collect(),
        };
        manifest.synthetic_nodes.insert(synthetic);
        match root {
            RootId::Entry(id) => {
                manifest.entries.insert(id.clone(), facade);
            }
            RootId::Async(site) => {
                let mut occurrence = None;
                for &index in &plan.retention.async_boundaries {
                    if async_seed(graph, index)?.id == *root {
                        occurrence = Some(&graph.edges()[index]);
                        break;
                    }
                }
                let edge = occurrence.ok_or_eyre("active async root has no retained occurrence")?;
                manifest.async_facades.insert(
                    site.clone(),
                    AsyncFacade {
                        consumer: node_id(graph, edge.from)?,
                        target: node_id(graph, edge.to)?,
                        facade,
                    },
                );
            }
        }
    }
    for (node, roots) in &plan.node_roots {
        if let NodeId::CommonJsNamespace(module) = node {
            manifest
                .synthetic_nodes
                .insert(SyntheticNode::CommonJsNamespace {
                    module: module.clone(),
                    chunk: ChunkId(roots.clone()),
                });
        }
        if let NodeId::CommonJsProxy { module, name } = node {
            manifest
                .synthetic_nodes
                .insert(SyntheticNode::CommonJsProxy {
                    module: module.clone(),
                    name: name.clone(),
                    chunk: ChunkId(roots.clone()),
                });
        }
        if let NodeId::Module(module) = node {
            if !roots.is_empty() {
                manifest
                    .synthetic_nodes
                    .insert(SyntheticNode::ModuleIdentity {
                        module: module.clone(),
                        chunk: ChunkId(roots.clone()),
                    });
            }
        }
    }
    for edge in &plan.retention.edges {
        if matches!(
            edge.kind,
            RetentionEdgeKind::EffectOrder | RetentionEdgeKind::Namespace
        ) {
            continue;
        }
        // Use exactly the same synchronous/namespace expansion as the oracle.
        if let Some(index) = edge.source_edge {
            let source = &graph.edges()[index];
            if source.activation == source_graph::Activation::Asynchronous || source.type_only {
                continue;
            }
        }
        let Some(from) = plan.chunk_for(&node_id(graph, edge.from)?) else {
            continue;
        };
        let mut targets = VecDeque::new();
        if matches!(
            edge.kind,
            RetentionEdgeKind::Binding | RetentionEdgeKind::Require
        ) {
            enqueue_binding(&plan.retention, edge.to, &mut targets);
        } else {
            targets.push_back(edge.to);
        }
        for target in targets {
            let to = plan
                .chunk_for(&node_id(graph, target)?)
                .ok_or_eyre("live chunk dependency has no chunk")?;
            if from != to {
                manifest.chunk_dependencies.insert(ChunkDependency {
                    from: from.clone(),
                    to,
                });
            }
        }
    }
    for file in plan
        .retention
        .modules
        .iter()
        .copied()
        .filter(|file| graph.file_facts(*file).is_some_and(|facts| facts.commonjs))
    {
        let module = graph.file_path(file).unwrap().to_owned();
        let initialize = NodeId::CommonJsProxy {
            module: module.clone(),
            name: None,
        };
        let namespace = NodeId::CommonJsNamespace(module.clone());
        let mut dependencies = vec![
            (initialize.clone(), NodeId::Module(module.clone())),
            (namespace.clone(), initialize.clone()),
        ];
        let names = crate::cjs::names(graph, &plan.retention, file);
        for name in names.iter().filter(|name| name.as_str() != "default") {
            dependencies.push((
                NodeId::CommonJsProxy {
                    module: module.clone(),
                    name: Some(name.clone()),
                },
                initialize.clone(),
            ));
        }
        for (from, request) in crate::cjs::requests(graph, &plan.retention, file) {
            let from = node_id(graph, from)?;
            dependencies.push((from.clone(), initialize.clone()));
            let requested = match request {
                ast_segmenter::owned::ImportedName::Name(name) => BTreeSet::from([name]),
                ast_segmenter::owned::ImportedName::Namespace => {
                    dependencies.push((from.clone(), namespace.clone()));
                    BTreeSet::new()
                }
            };
            for name in requested.into_iter().filter(|name| name != "default") {
                dependencies.push((
                    from.clone(),
                    NodeId::CommonJsProxy {
                        module: module.clone(),
                        name: Some(name),
                    },
                ));
            }
        }
        for (from, to) in dependencies {
            if let (Some(from), Some(to)) = (plan.chunk_for(&from), plan.chunk_for(&to)) {
                if from != to {
                    manifest
                        .chunk_dependencies
                        .insert(ChunkDependency { from, to });
                }
            }
        }
    }
    Ok(manifest)
}

fn facade_demand(graph: &SourceGraph, seed: &RootSeed) -> eyre::Result<FacadeDemand> {
    let mut evaluation_modules = BTreeSet::new();
    for &file in &seed.evaluation_roots {
        evaluation_modules.insert(
            graph
                .file_path(file)
                .ok_or_eyre("missing facade module")?
                .to_owned(),
        );
    }
    let mut binding_nodes = BTreeSet::new();
    for &node in &seed.binding_roots {
        let canonical = node_id(graph, node)?;
        match &canonical {
            NodeId::Module(module) | NodeId::Segment { module, .. } => {
                evaluation_modules.insert(module.clone());
            }
            NodeId::External { .. }
            | NodeId::CommonJsProxy { .. }
            | NodeId::CommonJsNamespace(_) => {}
        }
        binding_nodes.insert(canonical);
    }
    Ok(FacadeDemand {
        evaluation_modules,
        binding_nodes,
    })
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
