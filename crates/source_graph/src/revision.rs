use crate::{DependencyTarget, GraphNode, OwnedSourceFileInput, SourceGraph};
use ast_segmenter::owned::DependencyKind;
use import_resolver::structural::{DependencyResolver, ResolutionMode, ResolvedDependency};
use std::path::Path;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GraphRevision {
    pub source: u64,
    pub resolution: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionedNode {
    pub node: GraphNode,
    pub revision: GraphRevision,
}

impl SourceGraph {
    pub fn revision(&self) -> GraphRevision {
        self.revision
    }

    pub fn versioned_node(&self, node: GraphNode) -> VersionedNode {
        VersionedNode {
            node,
            revision: self.revision,
        }
    }

    pub fn is_current(&self, node: VersionedNode) -> bool {
        node.revision == self.revision
    }

    /// Re-resolve all occurrences, including prior misses. Call after resolver invalidation.
    pub fn refresh_resolution(&mut self, resolver: &impl DependencyResolver) -> eyre::Result<()> {
        for file in &mut self.files {
            let Some(input) = &mut file.owned else {
                continue;
            };
            input.resolutions.clear();
            for dependency in &input.facts.dependencies {
                let mode = if dependency.kind == DependencyKind::Require {
                    ResolutionMode::Require
                } else {
                    ResolutionMode::Import
                };
                let target =
                    match resolver.resolve_dependency(&input.path, &dependency.specifier, mode) {
                        Ok(ResolvedDependency::File(path)) => DependencyTarget::File(path),
                        Ok(ResolvedDependency::External(name)) => DependencyTarget::External(name),
                        Err(error) => DependencyTarget::Missing(error.to_string()),
                    };
                input.resolutions.insert(dependency.id, target);
            }
        }
        self.revision.resolution += 1;
        self.clear_reexport_cache();
        self.rebuild_edges();
        self.validate_links()
    }

    pub fn dependency_target(
        &self,
        file_id: u32,
        dependency: ast_segmenter::owned::DependencyId,
    ) -> Option<&DependencyTarget> {
        self.files
            .get(file_id as usize)?
            .owned
            .as_ref()?
            .resolutions
            .get(&dependency)
    }

    pub fn patch_owned(&mut self, path: &Path, input: OwnedSourceFileInput) -> eyre::Result<()> {
        eyre::ensure!(
            path == input.path,
            "patch path mismatch; rename requires remove plus add"
        );
        let mut replacement = Self::from_owned([input]);
        let file = replacement.files.pop().expect("one input");
        if let Some(id) = self.file_id(path) {
            self.files[id as usize] = file;
        } else {
            self.path_to_id
                .insert(path.to_owned(), self.files.len() as u32);
            self.files.push(file);
        }
        self.revision.source += 1;
        self.revision.resolution += 1;
        self.clear_reexport_cache();
        self.rebuild_edges();
        Ok(())
    }

    /// Tombstone IDs; recreation never aliases a removed module's node.
    pub fn remove_file(&mut self, path: &Path) -> bool {
        let Some(id) = self.path_to_id.remove(path) else {
            return false;
        };
        let file = &mut self.files[id as usize];
        file.segments.clear();
        file.symbol_to_segment.clear();
        file.name_to_declaring_segments.clear();
        file.resolved_import_paths.clear();
        file.owned = None;
        self.revision.source += 1;
        self.revision.resolution += 1;
        self.clear_reexport_cache();
        self.rebuild_edges();
        true
    }
}
