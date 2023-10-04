use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use super::{
    node_visitor::{ExportKind, ImportedItem},
    unused_finder_visitor_runner::ImportExportInfo,
};

#[derive(Debug, Clone, Default)]
pub struct GraphFile {
    pub is_used: bool,
    // pub package_name: String,
    pub file_path: String,
    pub unused_exports: HashSet<ExportKind>,
    pub import_export_info: ImportExportInfo,
}

impl GraphFile {
    pub fn mark_item_as_used(&mut self, item: &ImportedItem) -> bool {
        self.is_used = true;
        let item = ExportKind::from(item);
        self.unused_exports.remove(&item)
    }
}

pub type Edge = (String, String, ImportedItem);

#[derive(Default, Debug)]
pub struct Graph {
    pub files: HashMap<String, Arc<GraphFile>>,
}

impl<'a> Graph {
    pub fn bfs_step(&mut self, entries: Vec<String>) -> Vec<String> {
        let edges = self.get_edges(entries);
        let new_entries = edges
            .iter()
            .filter_map(|(entry_path, imported, item)| {
                if self.files.contains_key(entry_path) {
                    return self.resolve_edge(imported, item);
                }
                None
            })
            .collect();
        new_entries
    }

    fn resolve_edge(&mut self, imported: &String, item: &ImportedItem) -> Option<String> {
        let mut any_used = false;
        if let Some(imported_file_rc) = self.files.get_mut(imported) {
            if let Some(imported_file) = Arc::get_mut(imported_file_rc) {
                // If `file` was already marked as used
                any_used = any_used || !imported_file.is_used;
                // if `item` was already marked as used
                any_used = imported_file.mark_item_as_used(item) || any_used;
                if any_used {
                    return Some(imported.clone());
                }
            }
        }
        None
    }

    fn get_edges(&mut self, entries: Vec<String>) -> HashSet<(String, String, ImportedItem)> {
        let edges: HashSet<Edge> = entries
            .par_iter()
            .filter_map(|entry| {
                if let Some(e) = self.files.get(entry) {
                    let mut edges: HashSet<Edge> = e
                        .import_export_info
                        .imported_paths
                        .iter()
                        .map(|imported| (entry.clone(), imported.clone(), ImportedItem::Namespace))
                        .chain(e.import_export_info.executed_paths.iter().map(|imported| {
                            (entry.clone(), imported.clone(), ImportedItem::ExecutionOnly)
                        }))
                        .chain(e.import_export_info.require_paths.iter().map(|imported| {
                            (entry.clone(), imported.clone(), ImportedItem::Namespace)
                        }))
                        .collect();
                    e.import_export_info
                        .imported_path_ids
                        .iter()
                        .for_each(|(path, items)| {
                            for item in items {
                                edges.insert((e.file_path.clone(), path.clone(), item.clone()));
                            }
                        });
                    e.import_export_info
                        .export_from_ids
                        .iter()
                        .for_each(|(path, items)| {
                            for item in items {
                                edges.insert((e.file_path.clone(), path.clone(), item.clone()));
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
