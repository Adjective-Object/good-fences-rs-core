use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use super::{
    node_visitor::{ExportKind, ImportedItem},
    unused_finder_visitor_runner::ImportExportInfo,
};

pub enum MarkItemResult {
    MarkedAsUsed,
    AlreadyMarked,
    ResolveExportFrom(String),
}

#[derive(Debug, Clone, Default)]
pub struct GraphFile {
    pub is_used: bool,
    pub is_test_file: bool,
    // pub package_name: String,
    pub file_path: String,
    pub unused_exports: HashSet<ExportKind>,
    //
    pub export_from: HashMap<ExportKind, String>,
    pub import_export_info: ImportExportInfo,
    pub only_used_in_test: HashSet<ExportKind>,
}

impl GraphFile {
    pub fn new(
        file_path: String,
        unused_exports: HashSet<ExportKind>,
        import_export_info: ImportExportInfo,
        is_used: bool,
        is_test_file: bool,
    ) -> Self {
        let mut export_from = HashMap::new();
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
            is_test_file,
            ..Default::default()
        }
    }

    pub fn resolve_export_from_items(&mut self, files: &mut HashMap<String, Arc<GraphFile>>) {
        self.export_from.iter().for_each(|(item, path)| {
            if let Some(origin) = files.get_mut(path) {
                Arc::get_mut(origin).unwrap().mark_item_as_used(item);
                // origin.mark_item_as_used(item);
            }
        });
    }

    pub fn mark_item_as_used(&mut self, item: &ExportKind) -> MarkItemResult {
        self.is_used = true;
        // let item = ExportKind::from(item);
        if self.unused_exports.remove(&item) {
            return MarkItemResult::MarkedAsUsed;
        }
        if let Some(from) = self.export_from.get(item) {
            return MarkItemResult::ResolveExportFrom(from.clone());
        }
        MarkItemResult::AlreadyMarked
    }

    pub fn mark_item_as_used_in_test(&mut self, item: &ExportKind) {
        if let Some(taken) = self.unused_exports.take(item) {
            self.only_used_in_test.insert(taken);
        }
    }
}

pub type Edge = (String, String, ImportedItem);

#[derive(Default, Debug, Clone)]
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
            .flatten()
            .collect();
        new_entries
    }

    fn resolve_edge(&mut self, imported: &String, item: &ImportedItem) -> Option<Vec<String>> {
        let mut any_used = false;
        let mut pending_paths: Vec<(String, &ImportedItem)> = Vec::new();
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
            if let Some(resolutions) = self.resolve_edge(&imported, item) {
                any_used = true;
                resolved.extend(resolutions);
            }
        }
        if any_used {
            resolved.push(imported.clone());
            return Some(resolved);
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
