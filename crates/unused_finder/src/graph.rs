use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use ahashmap::{AHashMap, AHashSet};
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use crate::{
    parse::{ExportKind, ImportedItem, ResolvedImportExportInfo},
    walked_file::ResolvedSourceFile,
};

pub enum MarkItemResult {
    MarkedAsUsed,
    AlreadyMarked,
    ResolveExportFrom(PathBuf),
}

// graph node used to represent a file during the "used file" walk
#[derive(Debug, Clone, Default)]
pub struct GraphFile {
    // If this file is used or not
    pub is_used: bool,
    // The path of this file within the graph
    pub file_path: PathBuf,
    // The unused exports within this file
    pub unused_exports: AHashSet<ExportKind>,
    // Map of re-exports from this file
    pub export_from: AHashMap<ExportKind, PathBuf>,
    // Resolved import/export information w
    pub import_export_info: ResolvedImportExportInfo,
}

impl GraphFile {
    pub fn new_from_source_file(file: &ResolvedSourceFile) -> Self {
        let all_exported_symbols = file
            .import_export_info
            .exported_ids
            .iter()
            .map(|e| e.metadata.export_kind.clone())
            .collect();
        Self::new(
            file.source_file_path.clone(),
            all_exported_symbols,
            file.import_export_info.clone(),
            false,
        )
    }

    pub fn new(
        file_path: PathBuf,
        unused_exports: AHashSet<ExportKind>,
        import_export_info: ResolvedImportExportInfo,
        is_used: bool,
    ) -> Self {
        let mut export_from = AHashMap::default();
        import_export_info
            .export_from_ids
            .iter()
            .for_each(|(source_file, items)| {
                for item in items {
                    export_from.insert(item.into(), source_file.clone());
                }
            });
        Self {
            is_used,
            export_from,
            file_path,
            unused_exports,
            import_export_info,
        }
    }

    pub fn resolve_export_from_items(&mut self, files: &mut AHashMap<PathBuf, Arc<GraphFile>>) {
        self.export_from.iter().for_each(|(item, path)| {
            if let Some(origin) = files.get_mut(path) {
                Arc::get_mut(origin).unwrap().mark_item_as_used(item);
                // origin.mark_item_as_used(item);
            }
        });
    }

    /// Marks an item within this graph file as used
    ///
    /// If the item is a re-export of an item from another file, the origin file is returned
    pub fn mark_item_as_used(&mut self, item: &ExportKind) -> MarkItemResult {
        self.is_used = true;
        // let item = ExportKind::from(item);
        if self.unused_exports.remove(item) {
            return MarkItemResult::MarkedAsUsed;
        }
        if let Some(from) = self.export_from.get(item) {
            return MarkItemResult::ResolveExportFrom(from.to_owned());
        }
        MarkItemResult::AlreadyMarked
    }
}

// A 1-way representation of an edge in the import graph
#[derive(Eq, PartialEq, Hash, Debug, Clone)]
struct Edge {
    // The path of the file that is imported
    to_file: PathBuf,
    // The item that is imported
    item: ImportedItem,
}

impl Edge {
    pub fn new(to_file: PathBuf, item: ImportedItem) -> Self {
        Self { to_file, item }
    }
}

#[derive(Default, Debug, Clone)]
pub struct Graph {
    pub files: AHashMap<PathBuf, Arc<GraphFile>>,
}

impl Graph {
    /// Create new graph from a list of source files
    pub fn from_source_files<'a>(
        source_files: impl Iterator<Item = &'a ResolvedSourceFile>,
    ) -> Self {
        Graph {
            files: source_files
                .map(|source_file| {
                    (
                        source_file.source_file_path.to_path_buf(),
                        Arc::new(GraphFile::new_from_source_file(source_file)),
                    )
                })
                .collect(),
        }
    }

    /// Perform a single step of the BFS algorithm, returning the list of files that should be visited next
    pub fn bfs_step(&mut self, entries: Vec<PathBuf>) -> Vec<PathBuf> {
        let edges = self.get_edges(entries);
        let new_entries = edges
            .iter()
            .filter_map(|Edge { to_file, item }| {
                if self.files.contains_key(to_file) {
                    return self.mark_used(to_file, item);
                }
                None
            })
            .flatten()
            .collect();
        new_entries
    }

    /// Mark a file (and an item in that file) as used, and return the list of files that
    /// should be visited next
    ///
    /// This can happen when a file re-exports a symbol from another file
    fn mark_used<'a>(&'a mut self, imported: &Path, item: &ImportedItem) -> Option<Vec<PathBuf>> {
        let mut any_used = false;
        let mut pending_paths: Vec<(PathBuf, &ImportedItem)> = Vec::new();
        let mut resolved = vec![];
        if let Some(imported_file_rc) = self.files.get_mut(imported) {
            if let Some(imported_file) = Arc::get_mut(imported_file_rc) {
                // If `file` was already marked as used
                any_used = any_used || !imported_file.is_used;
                // if `item` was already marked as used
                match imported_file.mark_item_as_used(&item.into()) {
                    MarkItemResult::MarkedAsUsed => any_used = true,
                    MarkItemResult::AlreadyMarked => {}
                    MarkItemResult::ResolveExportFrom(origin) => {
                        pending_paths.push((origin, item));
                    }
                }
            }
        }
        for (imported, item) in pending_paths {
            if let Some(resolutions) = self.mark_used(&imported, item) {
                any_used = true;
                resolved.extend(resolutions);
            }
        }
        if any_used {
            resolved.push(imported.to_path_buf());
            return Some(resolved);
        }
        None
    }

    fn get_edges<'a>(&'a self, entries: Vec<PathBuf>) -> AHashSet<Edge> {
        let edges: AHashSet<Edge> = entries
            .par_iter()
            .filter_map(|entry| {
                if let Some(e) = self.files.get(entry) {
                    let mut edges: AHashSet<Edge> =
                        e.import_export_info
                            .imported_paths
                            .iter()
                            .map(|imported| Edge::new(imported.clone(), ImportedItem::Namespace))
                            .chain(e.import_export_info.executed_paths.iter().map(|imported| {
                                Edge::new(imported.clone(), ImportedItem::ExecutionOnly)
                            }))
                            .chain(e.import_export_info.require_paths.iter().map(|imported| {
                                Edge::new(imported.clone(), ImportedItem::Namespace)
                            }))
                            .collect();
                    e.import_export_info
                        .imported_path_ids
                        .iter()
                        .for_each(|(path, items)| {
                            for item in items {
                                edges.insert(Edge::new(path.clone(), item.clone()));
                            }
                        });
                    e.import_export_info
                        .export_from_ids
                        .iter()
                        .for_each(|(path, items)| {
                            for item in items {
                                edges.insert(Edge::new(path.clone(), item.clone()));
                            }
                        });
                    return Some(edges);
                }
                None
            })
            .flatten()
            .collect();
        edges
    }
}
