use std::path::PathBuf;

use ahashmap::{AHashMap, AHashSet};
use anyhow::Result;
use rayon::prelude::*;

use crate::{
    logger::Logger,
    parse::{ExportedSymbol, ResolvedImportExportInfo},
    walked_file::ResolvedSourceFile,
};

// graph node used to represent a file during the "used file" walk
#[derive(Debug, Clone, Default)]
pub struct GraphFile {
    pub is_file_used: bool,
    // The path of this file within the graph
    pub file_path: PathBuf,
    // The unused exports within this file
    pub unused_named_exports: AHashSet<ExportedSymbol>,
    // Map of re-exported items to the file that they came from
    // Resolved import/export information w
    pub import_export_info: ResolvedImportExportInfo,
}

impl GraphFile {
    pub fn new_from_source_file(file: &ResolvedSourceFile) -> Self {
        let all_exported_symbols = file
            .import_export_info
            .exported_ids
            .keys()
            .cloned()
            .collect();
        println!(
            "new_from_source_file: {} {:?}",
            file.source_file_path.display(),
            all_exported_symbols
        );
        Self {
            is_file_used: false,
            file_path: file.source_file_path.clone(),
            unused_named_exports: all_exported_symbols,
            import_export_info: file.import_export_info.clone(),
        }
    }

    /// Marks an item within this graph file as used
    ///
    /// If the item is a re-export of an item from another file, the origin file is returned
    fn mark_symbol_used(&mut self, symbol: &ExportedSymbol) {
        self.is_file_used = true;
        println!(
            "    mark symbol used: {}:{:?}",
            self.file_path.display(),
            symbol
        );

        // let item = ExportKind::from(item);
        match symbol {
            ExportedSymbol::Default | ExportedSymbol::Named(_) => {
                self.unused_named_exports.remove(symbol);
            }
            ExportedSymbol::Namespace => {
                // namespace imports will use _all_ symbols from the imported file
                self.unused_named_exports.clear();
            }
            ExportedSymbol::ExecutionOnly => {
                // noop, don't mark any names as used
            }
        }
    }
}

// A 1-way representation of an edge in the import graph
#[derive(Eq, PartialEq, Hash, Debug, Clone, PartialOrd, Ord)]
pub struct Edge {
    // The path of the file that is imported
    pub file_id: usize,
    // The symbol that is imported
    pub symbol: ExportedSymbol,
}

impl Edge {
    pub fn new(file_id: usize, symbol: ExportedSymbol) -> Self {
        Self { file_id, symbol }
    }
}

#[derive(Default, Debug, Clone)]
pub struct Graph {
    pub path_to_id: AHashMap<PathBuf, usize>,
    pub files: Vec<GraphFile>,
    // Set of edges we have already traversed
    pub visited: AHashSet<Edge>,
}

impl Graph {
    /// Create new graph from a list of source files
    pub fn from_source_files<'a>(
        source_files: impl Iterator<Item = &'a ResolvedSourceFile>,
    ) -> Self {
        let mut path_to_id = AHashMap::default();
        let files: Vec<GraphFile> = source_files
            .map(|source_file| {
                let id = path_to_id.len();
                path_to_id.insert(source_file.source_file_path.clone(), id);
                GraphFile::new_from_source_file(source_file)
            })
            .collect();

        Graph {
            path_to_id,
            files,
            visited: AHashSet::default(),
        }
    }

    pub fn traverse_bfs(
        &mut self,
        logger: impl Logger,
        initial_frontier: Vec<PathBuf>,
    ) -> Result<()> {
        let initial_file_ids = initial_frontier
            .iter()
            .filter_map(|path| match self.path_to_id.get(path) {
                Some(file_id) => Some(*file_id),
                None => {
                    logger.log(format!(
                        "Frontier file not found in graph: {}",
                        path.to_string_lossy()
                    ));
                    None
                }
            })
            .collect::<Vec<_>>();

        // Create a set of edges to the initial frontier
        let mut frontier = initial_file_ids
            .into_iter()
            .map(|file_id| Edge::new(file_id, ExportedSymbol::Namespace))
            .collect::<Vec<_>>();

        // Traverse the graph until we exhaust the frontier
        const MAX_ITERATIONS: usize = 1_000_000;
        for _ in 0..MAX_ITERATIONS {
            let next_frontier: Vec<Edge> = self.bfs_step(&frontier);
            println!("bfs_step: {:?} -> {:?}", frontier, next_frontier);
            frontier = next_frontier;
            if frontier.is_empty() {
                return Ok(());
            }
        }

        Err(anyhow::anyhow!(
            "import graph traversal exceeded MAX_ITERATIONS ({})",
            MAX_ITERATIONS
        ))
    }

    /// Perform a single step of the BFS algorithm, returning the list of files that should be visited next
    pub fn bfs_step(&mut self, frontier: &[Edge]) -> Vec<Edge> {
        // get list of unique files that are being visited in this pass
        let mut from_files = frontier
            .iter()
            .map(|Edge { file_id, .. }| *file_id)
            .collect::<Vec<_>>();
        from_files.sort();
        from_files.dedup();

        // generate the next frontier in a parallel pass over the files
        let mut next_frontier_symbols = from_files
            .par_iter()
            .map(|file_id| {
                let file = &self.files[*file_id];
                // if the file was not visited before, add all its imports
                // to the frontier
                //
                // TODO: become more granular here for re-exported symbols
                let outgoing_edges = file
                    .import_export_info
                    .iter_imported_symbols()
                    .filter_map(|(path, symbol)| {
                        let edge = match self.path_to_id.get(path) {
                            Some(id) => Edge::new(*id, ExportedSymbol::from(symbol)),
                            None => {
                                return None;
                            }
                        };

                        // don't re-traverse edges we have already visited
                        if self.visited.contains(&edge) {
                            None
                        } else {
                            Some(edge)
                        }
                    })
                    .par_bridge();

                outgoing_edges
            })
            .flatten()
            .collect::<Vec<_>>();

        // mark all symbols we visited in this pass as visited
        for edge in frontier.iter() {
            self.files[edge.file_id].mark_symbol_used(&edge.symbol);
            self.visited.insert(edge.clone());
        }

        next_frontier_symbols.sort();
        next_frontier_symbols.dedup();
        next_frontier_symbols
    }
}
