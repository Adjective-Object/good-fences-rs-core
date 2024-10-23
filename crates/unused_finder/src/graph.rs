use core::fmt;
use std::{fmt::Display, path::PathBuf};

use ahashmap::{AHashMap, AHashSet};
use anyhow::Result;
use rayon::prelude::*;

use crate::{
    logger::Logger,
    parse::{ExportedSymbol, ResolvedImportExportInfo},
    walked_file::ResolvedSourceFile,
};

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct UsedTag: u8 {
        /// True if this file or symbol was used recursively by an
        /// "entry package" (a package that was passed as an entry point).
        const FROM_ENTRY = 0x01;
        /// True if this file or symbol was used recursively by a test file.
        const FROM_TEST = 0x02;
        /// True if this file or symbol was used recursively by an
        /// ignored symbol or file.
        const FROM_IGNORED = 0x04;
    }
}

impl Display for UsedTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut tags = Vec::new();
        if self.contains(Self::FROM_ENTRY) {
            tags.push("entry");
        }
        if self.contains(Self::FROM_IGNORED) {
            tags.push("ignored");
        }
        write!(f, "{}", tags.join("+"))
    }
}

// graph node used to represent a file during the "used file" walk
#[derive(Debug, Clone, Default)]
pub struct GraphFile {
    /// The tags on this file
    pub file_tags: UsedTag,
    /// The tags on this file's symbols
    pub symbol_tags: AHashMap<ExportedSymbol, UsedTag>,
    // The path of this file within the graph
    pub file_path: PathBuf,
    // Map of re-exported items to the file that they came from
    // Resolved import/export information w
    pub import_export_info: ResolvedImportExportInfo,
}

impl GraphFile {
    pub fn new_from_source_file(file: &ResolvedSourceFile) -> Self {
        Self {
            file_tags: UsedTag::default(),
            symbol_tags: AHashMap::with_capacity_and_hasher(
                file.import_export_info.exported_ids.len(),
                Default::default(),
            ),
            file_path: file.source_file_path.clone(),
            import_export_info: file.import_export_info.clone(),
        }
    }

    /// Marks an item within this graph file as used
    ///
    /// If the item is a re-export of an item from another file, the origin file is returned
    fn tag_symbol(&mut self, symbol: &ExportedSymbol, tag: UsedTag) {
        // let item = ExportKind::from(item);
        match symbol {
            ExportedSymbol::Default | ExportedSymbol::Named(_) => {
                tag_named_or_default_symbol(&mut self.symbol_tags, symbol, tag);
            }
            ExportedSymbol::Namespace => {
                // namespace imports will use _all_ named symbols from the imported file
                for (reexported_from, symbol) in self.import_export_info.iter_exported_symbols() {
                    match (reexported_from, symbol) {
                        (_, ExportedSymbol::Default | ExportedSymbol::Named(_)) => {
                            // mark as used
                            tag_named_or_default_symbol(&mut self.symbol_tags, symbol, tag);
                        }
                        _ => {
                            // TODO: somehow handle re-exports of namespaces
                        }
                    }
                }
            }
            ExportedSymbol::ExecutionOnly => {
                // noop, don't mark any names as used
            }
        }
    }
}

fn tag_named_or_default_symbol(
    symbol_tags: &mut AHashMap<ExportedSymbol, UsedTag>,
    symbol: &ExportedSymbol,
    tag: UsedTag,
) {
    let tags = symbol_tags.get(symbol).copied().unwrap_or_default();
    if !tags.contains(tag) {
        symbol_tags.insert(symbol.clone(), tag);
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
        initial_frontier_files: Vec<PathBuf>,
        initial_frontier_symbols: Vec<(PathBuf, Vec<ExportedSymbol>)>,
        tag: UsedTag,
    ) -> Result<()> {
        let initial_file_edges = initial_frontier_files
            .into_iter()
            .filter_map(|path| match self.path_to_id.get(&path) {
                Some(file_id) => Some(*file_id),
                None => {
                    logger.log(format!(
                        "Frontier file not found in graph: {}",
                        path.to_string_lossy()
                    ));
                    None
                }
            })
            .map(|file_id| Edge::new(file_id, ExportedSymbol::Namespace));

        let initial_symbol_edges = initial_frontier_symbols
            .into_iter()
            .filter_map(
                |(path, symbols): (PathBuf, Vec<ExportedSymbol>)| -> Option<Vec<Edge>> {
                    match self.path_to_id.get(&path).cloned() {
                        Some(file_id) => Some(
                            symbols
                                .into_iter()
                                .map(|symbol| Edge::new(file_id, symbol))
                                .collect(),
                        ),
                        None => {
                            logger.log(format!(
                                "Frontier symbol's file not found in graph: {}",
                                path.to_string_lossy()
                            ));
                            None
                        }
                    }
                },
            )
            .flatten();

        let mut frontier = initial_file_edges
            .chain(initial_symbol_edges)
            .collect::<Vec<_>>();

        // Traverse the graph until we exhaust the frontier
        const MAX_ITERATIONS: usize = 1_000_000;
        for _ in 0..MAX_ITERATIONS {
            let next_frontier: Vec<Edge> = self.bfs_step(&frontier, tag);
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
    pub fn bfs_step(&mut self, frontier: &[Edge], tag: UsedTag) -> Vec<Edge> {
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
                            Some(id) => Edge::new(*id, symbol),
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
            self.files[edge.file_id].tag_symbol(&edge.symbol, tag);
            self.visited.insert(edge.clone());
        }
        // mark all files we visited in this pass as visited
        for file in from_files {
            self.files[file].file_tags |= tag;
        }

        next_frontier_symbols.sort();
        next_frontier_symbols.dedup();
        next_frontier_symbols
    }
}
