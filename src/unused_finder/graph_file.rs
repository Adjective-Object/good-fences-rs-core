use std::{
    cell::{RefCell, RefMut},
    collections::{HashMap, HashSet},
    rc::Rc, borrow::BorrowMut, ops::DerefMut,
};

use super::{
    node_visitor::ExportedItem, unused_finder_visitor_runner::ImportExportInfo, WalkFileMetaData,
};

#[derive(Debug, Clone, Default)]
pub struct GraphFile {
    pub is_used: bool,
    // pub package_name: String,
    pub file_path: String,
    pub unused_exports: HashSet<ExportedItem>,
    pub import_export_info: ImportExportInfo,
}

impl GraphFile {
    pub fn new(source_file: WalkFileMetaData) -> Self {
        Self {
            is_used: false,
            file_path: source_file.source_file_path,
            unused_exports: source_file.import_export_info.exported_ids.clone(),
            import_export_info: source_file.import_export_info,
            ..Default::default()
        }
    }

    pub fn import(&mut self, files: &mut HashMap<String, Rc<GraphFile>>) {
        // self.mark_used_items(files.clone());
        // self.add_imported_files(files);
    }

    fn mark_used_items<'a>(&mut self, files: &mut HashMap<String, Rc<GraphFile>>) -> Vec<String> {
        let mut imported_files: Vec<String> = Vec::new();
        for (path, values) in self.import_export_info.export_from_ids.iter() {
            match files.get_mut(path) {
                Some(f) => {
                    let f = Rc::get_mut(f).unwrap();
                    dbg!("marking as used", &f.file_path);
                    f.is_used = true;
                    imported_files.push(f.file_path.clone());
                    f.unused_exports.retain(|i| !values.contains(&i.into()));
                }
                None => {}
            }
        }
        for (path, values) in self.import_export_info.imported_path_ids.iter() {
            match files.get_mut(path) {
                Some(f) => {
                    let f = Rc::get_mut(f).unwrap();
                    dbg!("marking as used", &f.file_path);
                    f.is_used = true;
                    imported_files.push(f.file_path.clone());
                    f.unused_exports.retain(|i| !values.contains(&i.into()));
                }
                None => {}
            }
        }
        for path in self.import_export_info.imported_paths.iter() {
            match files.get_mut(path) {
                Some(f) => {
                    let f = Rc::get_mut(f).unwrap();
                    dbg!("marking as used", &f.file_path);
                    f.is_used = true;
                    imported_files.push(f.file_path.clone());
                    f.unused_exports.clear();
                }
                None => {}
            };
        }
        return imported_files;
    }
}

impl From<WalkFileMetaData> for GraphFile {
    fn from(source_file: WalkFileMetaData) -> Self {
        Self::new(source_file)
    }
}

#[derive(Default, Debug)]
pub struct Graph {
    pub files:HashMap<String, Rc<GraphFile>>,
    pub edges: Vec<(Rc<GraphFile>, Rc<GraphFile>)>,
}

impl<'a> Graph {
    pub fn bfs(&mut self, entries: &'a mut Vec<Rc<GraphFile>>) {
        // ... (unchanged)
        let mut m: Vec<(String, Vec<String>)> = entries
            .iter_mut()
            .filter_map(|e| Rc::get_mut(e))
            .map(|entry| (entry.file_path.clone(), entry.mark_used_items(&mut self.files)))
            .collect();
        for e in entries.iter() {
            let imported: Vec<Rc<GraphFile>> = self.get_imported(e.clone());
            self.add_edges(e.clone(), imported);
        }
    }

    fn add_edges(&mut self, importer: Rc<GraphFile>, imported: Vec<Rc<GraphFile>>) {
        for imported in imported {
            self.edges.push((importer.clone(), imported));
        }
    }

    fn get_imported(&self, imported: Rc<GraphFile>) -> Vec<Rc<GraphFile>> {
        let mut files = vec![];
        for imported in &imported.import_export_info.imported_paths {
            if let Some(i) = self.files.get(imported) {
                files.push(i.clone());
            }
        }
        files
    }

    pub fn bfs_iter(&mut self) {
        // Implementation for bfs_iter
    }
}

// impl<'a> Graph {
//     // pub fn new(mut entries: Vec<WalkFileMetaData>, mut files: Vec<WalkFileMetaData>) -> Self {
//     // }

//     pub fn bfs(&mut self, entries: Vec<WalkFileMetaData>) {
//         let mut entries: Vec<Rc<GraphFile>> = entries.iter().filter_map(|e| {
//             if let Some(e) = self.files.get_mut(&e.source_file_path) {
//                 if let Some(e) = Rc::get_mut(e) {
//                     dbg!("marked as used", &e.file_path);
//                     e.is_used = true;
//                 } else {
//                     dbg!(&e.file_path);
//                 }
//                 return Some(Rc::clone(&e))
//             }
//             None
//         }).collect();
//         entries.iter_mut().for_each(|e| {
            
//             Rc::get_mut(e).unwrap().mark_used_items(self.files.clone());
//         });
//         // let mut entries: Vec<Rc<GraphFile>> = entries.map(|e| e.clone()).collect();
//         // dbg!(entries.len());
//         // entries.iter_mut().for_each(|e| {
//         //     // e.is_used = true;
//         //     if let Some(e) = Rc::get_mut(&mut e.clone()) {
//         //         dbg!("marked as used", &e.file_path);
//         //         e.is_used = true;
//         //     } else {
//         //         dbg!(&e.file_path);
//         //     }
//         // });
//         let mut last_used_count = 0 ;//entries.count();
//         dbg!(&last_used_count);
//         loop {
//             entries = self.bfs_iter(entries);
//             let used_count = self.files.values().filter(|f| f.is_used).count();
//             dbg!(last_used_count, used_count);
//             if used_count == last_used_count {
//                 break;
//             } else {
//                 last_used_count = used_count;
//             }
//         }
//     }

//     pub fn bfs_iter(&'a mut self, mut entries: Vec<Rc<GraphFile>>) -> Vec<Rc<GraphFile>> {
//         let mut m: HashMap<String, Vec<String>> = entries
//             .iter_mut()
//             .filter_map(|e| Rc::get_mut(e))
//             .map(|entry| {
//                 // Rc::
//                 // entry.mark_used_items(&mut self.files);
//                 // if let Some(entry) = Rc::get_mut(entry) {
//                     dbg!(&entry.file_path);
//                     dbg!(entry.mark_used_items(self.files.clone()));
//                     return (entry.file_path.clone(), entry.mark_used_items(self.files.clone()))
//                 // } 
//                 // None
//             })
//             .collect();
//         let mut new_entries = Vec::new();
//         for (importer, imported) in m.clone() {
//             let imp: Vec<Rc<GraphFile>> = imported.iter().filter_map(|i| self.files.get(i).cloned()).collect();
//             self.add_edges(self.files.get(&importer).unwrap().clone(), imp);
//         }
//         let v: Vec<String> = m.drain().flat_map(|e| e.1).collect();
//         new_entries = self.get_imported(v);
//         // for i in v {
//         //     if let Some(a) = self.files.get(&i) { 
//         //         new_entries.push(Rc::clone(a));
//         //     }
//         // }
//         new_entries
//     }

//     fn add_edges(&'a mut self, importer: Rc<GraphFile>, imported: Vec<Rc<GraphFile>>) {
//         for imported in imported {
//             self.edges.push((importer.clone(), imported.clone()));
//         }
//     }

//     fn get_imported(&'a self, imported: Vec<String>) -> Vec<Rc<GraphFile>> {
//         let mut files: Vec<Rc<GraphFile>> = vec![];
//         for imported in imported {
//             if let Some(i) = self.files.get(&imported) {
//                 files.push(i.clone());
//             }
//         }
//         files
//     }
// }
