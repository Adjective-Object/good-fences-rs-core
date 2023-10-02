use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

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
        let item = ExportKind::from(item);
        self.unused_exports.remove(&item)
    }
}

pub type Edge = (String, String, ImportedItem);

#[derive(Default, Debug)]
pub struct Graph {
    pub files: HashMap<String, Rc<GraphFile>>,
    pub edges: HashSet<Edge>,
}

impl<'a> Graph {
    pub fn bfs(&mut self, entries: Vec<String>) -> Vec<String> {
        let edges = self.get_edges(entries);
        let mut new_used_stuff = false;
        let new_entries = edges
            .iter()
            .filter_map(|(_, imported, item)| {
                if let Some(file) = self.files.get_mut(imported) {
                    if let Some(file) = Rc::get_mut(file) {
                        if !file.is_used {
                            file.is_used = true;
                            new_used_stuff = true;
                        }
                        new_used_stuff = new_used_stuff || file.mark_item_as_used(item);
                        if new_used_stuff {
                            new_used_stuff = false;
                            return Some(imported.clone());
                        }
                    }
                }
                None
            })
            .collect();
        self.edges.extend(edges);
        new_entries
    }

    fn get_edges(&mut self, entries: Vec<String>) -> HashSet<(String, String, ImportedItem)> {
        let edges: HashSet<Edge> = entries
            .iter()
            .filter_map(|entry| {
                if let Some(e) = self.files.get_mut(entry) {
                    Rc::get_mut(e).unwrap().is_used = true;
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
