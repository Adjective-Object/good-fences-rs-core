use core::option::Option::None;
use std::{
    collections::HashSet,
    fmt::Display,
    path::{Path, PathBuf},
};

use ahashmap::{AHashMap, AHashSet};
use anyhow::Result;
use rayon::prelude::*;

use crate::{
    parse::{ExportedSymbol, ResolvedImportExportInfo},
    tag::UsedTag,
    walked_file::ResolvedSourceFile,
};
use logger::{debug_logf, Logger};

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
                file.import_export_info.num_exported_symbols(),
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
                let old_tag = self.symbol_tags.entry(symbol.clone()).or_default();
                *old_tag = old_tag.union(tag);
            }
            ExportedSymbol::Namespace => {
                // namespace imports will use _all_ named symbols from the imported file
                for (reexported_from, symbol) in self.import_export_info.iter_exported_symbols() {
                    match (reexported_from, symbol) {
                        (_, ExportedSymbol::Default | ExportedSymbol::Named(_)) => {
                            let old_tag = self.symbol_tags.entry(symbol.clone()).or_default();
                            *old_tag = old_tag.union(tag);
                        }
                        _ => {
                            // TODO: somehow handle re-exports of namespaces?
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

// A 1-way representation of an edge in the import graph
#[derive(Eq, PartialEq, Hash, Debug, Clone, PartialOrd, Ord)]
pub struct Edge {
    // The path of the file that is imported
    pub file_id: usize,
    // The symbol that is imported
    pub symbol: ExportedSymbol,
}

impl Display for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{:?}", self.file_id, self.symbol)
    }
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
}

impl Graph {
    pub fn get_file_by_path(&self, path: &Path) -> Option<&GraphFile> {
        self.path_to_id.get(path).map(|id| &self.files[*id])
    }

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

        Graph { path_to_id, files }
    }

    pub fn mark_symbol(&mut self, path: &Path, symbol: &ExportedSymbol, tag: UsedTag) {
        let file_id = match self.path_to_id.get(path) {
            Some(id) => *id,
            None => {
                return;
            }
        };

        let file = &mut self.files[file_id];
        file.tag_symbol(symbol, tag);
    }

    pub fn mark_file(&mut self, path: &Path, tag: UsedTag) {
        let file_id = match self.path_to_id.get(path) {
            Some(id) => *id,
            None => {
                return;
            }
        };

        let file = &mut self.files[file_id];
        file.file_tags.insert(tag);
    }

    pub fn get_file_tags(&self, path: &Path) -> Option<UsedTag> {
        let file_id = self.path_to_id.get(path)?;
        Some(self.files[*file_id].file_tags)
    }

    pub fn get_symbol_tags(&self, path: &Path, symbol: &ExportedSymbol) -> Option<&UsedTag> {
        let file_id = self.path_to_id.get(path)?;
        let file = &self.files[*file_id];
        file.symbol_tags.get(symbol)
    }

    pub fn traverse_bfs(
        &mut self,
        logger: impl Logger,
        initial_frontier_files: Vec<&Path>,
        initial_frontier_symbols: Vec<(&Path, Vec<ExportedSymbol>)>,
        tag: UsedTag,
    ) -> Result<()> {
        logger.debug(format!(
            "initial_frontier_files ({}:{}):\n  {}",
            tag,
            initial_frontier_files.len(),
            initial_frontier_files
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n  "),
        ));

        logger.debug(format!(
            "initial_frontier_symbols ({}:{}):\n  {}",
            tag,
            initial_frontier_symbols.len(),
            initial_frontier_symbols
                .iter()
                .map(|(path, symbols)| (format!(
                    "{}:{}",
                    path.display(),
                    symbols
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )))
                .collect::<Vec<_>>()
                .join("\n  ")
        ));

        let initial_file_edges = initial_frontier_files
            .into_iter()
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
            .map(|file_id| Edge::new(file_id, ExportedSymbol::Namespace));

        let initial_symbol_edges = initial_frontier_symbols
            .into_iter()
            .filter_map(
                |(path, symbols): (&Path, Vec<ExportedSymbol>)| -> Option<Vec<Edge>> {
                    match self.path_to_id.get(path).cloned() {
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

        const SYMBOLS_PER_FILE_HINT: usize = 4;
        let mut visited = AHashSet::with_capacity_and_hasher(
            self.files.len() * SYMBOLS_PER_FILE_HINT,
            Default::default(),
        );

        let mut frontier = initial_file_edges
            .chain(initial_symbol_edges)
            .collect::<Vec<_>>();

        debug_logf!(
            logger,
            "initial frontier ({}:{}): {}",
            tag,
            frontier.len(),
            frontier
                .iter()
                .map(|p| format!("{}:{}", self.files[p.file_id].file_path.display(), p.symbol))
                .collect::<Vec<_>>()
                .join("\n  ")
        );

        // Traverse the graph until we exhaust the frontier
        const MAX_ITERATIONS: usize = 1_000_000;
        for _ in 0..MAX_ITERATIONS {
            let next_frontier: Vec<Edge> = self.bfs_step(&mut visited, &frontier, tag);
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
    fn bfs_step(
        &mut self,
        visited: &mut AHashSet<Edge>,
        frontier: &[Edge],
        tag: UsedTag,
    ) -> Vec<Edge> {
        // get list of unique files that are being visited in this pass
        let mut from_files = frontier
            .iter()
            .map(|Edge { file_id, .. }| *file_id)
            .collect::<Vec<_>>();
        from_files.sort();
        from_files.dedup();

        // mark all symbols we visited in this pass as visited
        for edge in frontier.iter() {
            self.files[edge.file_id].tag_symbol(&edge.symbol, tag);
            visited.insert(edge.clone());
        }
        // mark all files we visited in this pass as visited
        for file in from_files.iter() {
            self.files[*file].file_tags |= tag;
        }

        // generate the next frontier in a parallel pass over the files
        let next_frontier_symbols = from_files
            .par_iter()
            .map(|file_id| {
                let file = &self.files[*file_id];
                // if the file was not visited before, add all its imports
                // to the frontier
                //
                // TODO: become more granular here for re-exported symbols
                let outgoing_edges = file
                    .import_export_info
                    .iter_imported_symbols_meta()
                    .filter_map(|(path, symbol, meta)| {
                        // don't traverse type-only re-exports of symbols when marking items.
                        //
                        // This is so that we don't mark a symbol as used if it is only used as a type.
                        // TODO: should this be a TraversalMode that the graph is parameterized on? e.g.
                        // track USED_ENTRY and USED_ENTRY_AS_TYPE as separate tags?
                        if let Some(meta) = meta {
                            if meta.is_type_only {
                                return None;
                            }
                        }

                        let edge = match self.path_to_id.get(path) {
                            Some(id) => Edge::new(*id, symbol.clone()),
                            None => {
                                return None;
                            }
                        };

                        // don't re-traverse edges we have already visited
                        if visited.contains(&edge) {
                            None
                        } else {
                            Some(edge)
                        }
                    })
                    .par_bridge();

                outgoing_edges
            })
            .flatten()
            .collect::<HashSet<_>>();

        next_frontier_symbols.into_iter().collect()
    }
}

#[cfg(test)]
mod test {
    use test_tmpdir::amap2;

    use super::*;
    #[test]
    fn test_tag_union() {
        let path = PathBuf::from("/test/a.ts");
        let a_sym = ExportedSymbol::Named("a".to_string());

        let src_files = vec![ResolvedSourceFile {
            owning_package: None,
            source_file_path: path.clone(),
            import_export_info: ResolvedImportExportInfo {
                exported_ids: amap2![
                    a_sym.clone() => Default::default()
                ],
                ..Default::default()
            },
        }];

        let mut graph = Graph::from_source_files(src_files.iter());
        graph.mark_symbol(&path, &a_sym, UsedTag::FROM_ENTRY);
        graph.mark_symbol(&path, &a_sym, UsedTag::FROM_IGNORED);

        // check the value is the union
        let tags = graph.get_symbol_tags(&path, &a_sym);
        assert_eq!(
            tags.unwrap().clone(),
            UsedTag::empty()
                .union(UsedTag::FROM_ENTRY)
                .union(UsedTag::FROM_IGNORED)
        )
    }
}
