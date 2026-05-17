use ahashmap::{AHashMap, AHashSet};
use source_graph::{SegmentKey, SourceGraph};

use core::fmt::{self, Display};

bitflags::bitflags! {
    /// Tracks how a segment or file is transitively used.
    ///
    /// This is a duplicate of `unused_finder::tag::UsedTag`. Phase 7 will
    /// migrate `unused_finder` to import from here instead.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
    pub struct UsedTag: u8 {
        /// Used recursively by an entry package.
        const FROM_ENTRY = 0x01;
        /// Used recursively by a test file.
        const FROM_TEST = 0x02;
        /// Used recursively by an ignored symbol or file.
        const FROM_IGNORED = 0x04;
        /// This symbol is a type-only symbol.
        const TYPE_ONLY = 0x08;
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
        if self.contains(Self::FROM_TEST) {
            tags.push("test");
        }
        if self.contains(Self::TYPE_ONLY) {
            tags.push("type-only");
        }
        write!(f, "{}", tags.join("+"))
    }
}

/// Per-segment tag storage with file-level derivation.
///
/// `TagGraph` tracks usage tags per segment and derives file-level tags
/// by unioning all segment tags within a file. It borrows a [`SourceGraph`]
/// for structural queries but owns the tag state independently.
pub struct TagGraph {
    tags: AHashMap<SegmentKey, UsedTag>,
}

impl TagGraph {
    /// Create a new empty `TagGraph`.
    pub fn new() -> Self {
        TagGraph {
            tags: AHashMap::default(),
        }
    }

    /// Get the tag for a specific segment. Returns `UsedTag::empty()` if
    /// the segment has not been tagged.
    pub fn get_tag(&self, key: SegmentKey) -> UsedTag {
        self.tags
            .get(&key)
            .copied()
            .unwrap_or_else(UsedTag::empty)
    }

    /// Set (union) a tag on a specific segment.
    pub fn set_tag(&mut self, key: SegmentKey, tag: UsedTag) {
        *self.tags.entry(key).or_insert_with(UsedTag::empty) |= tag;
    }

    /// Derived file-level tag: union of all segment tags for segments
    /// belonging to `file_id`.
    ///
    /// Returns `UsedTag::empty()` if the file has no segments or no
    /// segments have been tagged.
    pub fn file_tag(&self, source: &SourceGraph, file_id: u32) -> UsedTag {
        let segments = match source.file_segments(file_id) {
            Some(segs) => segs,
            None => return UsedTag::empty(),
        };

        let mut combined = UsedTag::empty();
        for seg_idx in 0..segments.len() {
            let key = SegmentKey::new(file_id, seg_idx as u32);
            combined |= self.get_tag(key);
        }
        combined
    }
}

impl TagGraph {
    /// Downward BFS: tag all segments reachable from `roots` along import edges.
    ///
    /// For each segment in the frontier:
    /// 1. Tag the segment with `tag`
    /// 2. Find inter-file imports via `RawModuleDeps` (imports, dynamic_imports,
    ///    requires, executed_paths)
    /// 3. Resolve each import to target `SegmentKey`(s) via
    ///    `SourceGraph::resolve_import_across_files`
    /// 4. Skip edges where `is_type_only` is true unless `follow_type_only` is set
    /// 5. Find intra-file dependencies via escaped symbols →
    ///    `SourceGraph::resolve_symbol_in_file`
    /// 6. Add unvisited targets to the next frontier
    pub fn propagate_tags_to_used(
        &mut self,
        source: &SourceGraph,
        roots: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
    ) {
        use std::collections::VecDeque;

        let mut visited: AHashSet<SegmentKey> = AHashSet::default();
        let mut queue: VecDeque<SegmentKey> = VecDeque::new();

        for root in roots {
            if visited.insert(root) {
                queue.push_back(root);
            }
        }

        while let Some(key) = queue.pop_front() {
            self.set_tag(key, tag);

            let segments = match source.file_segments(key.file_id) {
                Some(segs) => segs,
                None => continue,
            };
            let segment = match segments.get(key.segment_idx as usize) {
                Some(seg) => seg,
                None => continue,
            };

            let resolved_import_paths = source
                .file_resolved_import_paths(key.file_id)
                .cloned()
                .unwrap_or_default();

            // Inter-file edges: static imports
            for (specifier, tagged_symbols) in &segment.module_deps.imports {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };

                for tagged_sym in tagged_symbols {
                    if !follow_type_only && tagged_sym.tags.is_type_only {
                        continue;
                    }
                    let targets =
                        Self::resolve_symbol_import(source, target_file_id, &tagged_sym.symbol);
                    for target_key in targets {
                        if visited.insert(target_key) {
                            queue.push_back(target_key);
                        }
                    }
                }
            }

            // Inter-file edges: dynamic imports
            for (specifier, symbols) in &segment.module_deps.dynamic_imports {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };

                for sym in symbols {
                    let targets = Self::resolve_symbol_import(source, target_file_id, sym);
                    for target_key in targets {
                        if visited.insert(target_key) {
                            queue.push_back(target_key);
                        }
                    }
                }
            }

            // Inter-file edges: requires (CommonJS — namespace-like, tag all exports)
            for specifier in &segment.module_deps.requires {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };
                Self::enqueue_all_segments(source, target_file_id, &mut visited, &mut queue);
            }

            // Inter-file edges: side-effect-only imports (`import './foo'`)
            for specifier in &segment.module_deps.executed_paths {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };
                Self::enqueue_all_segments(source, target_file_id, &mut visited, &mut queue);
            }

            // Intra-file edges: escaped symbols reference declarations in the same file
            for escaped_name in segment.variables.get_escaped_symbols() {
                if let Some(target_key) = source.resolve_symbol_in_file(
                    key.file_id,
                    escaped_name.as_ref(),
                    key.segment_idx,
                ) {
                    if target_key != key && visited.insert(target_key) {
                        queue.push_back(target_key);
                    }
                }
            }
        }
    }

    /// Convert a `Symbol` (Named/Default/Namespace) to resolution targets.
    fn resolve_symbol_import(
        source: &SourceGraph,
        target_file_id: u32,
        symbol: &ast_segmenter::raw_module_deps::Symbol,
    ) -> Vec<SegmentKey> {
        use ast_segmenter::raw_module_deps::Symbol;
        match symbol {
            Symbol::Named(name) => {
                let exported = ast_segmenter::ExportedSymbol::Named(name.clone());
                source.resolve_import_across_files(target_file_id, &exported)
            }
            Symbol::Default => {
                source.resolve_import_across_files(target_file_id, &ast_segmenter::ExportedSymbol::Default)
            }
            Symbol::Namespace => {
                // Namespace import: tag all segments in the target file
                let mut keys = Vec::new();
                if let Some(segs) = source.file_segments(target_file_id) {
                    for idx in 0..segs.len() {
                        keys.push(SegmentKey::new(target_file_id, idx as u32));
                    }
                }
                keys
            }
        }
    }

    /// Enqueue all segments of a file into the BFS.
    fn enqueue_all_segments(
        source: &SourceGraph,
        file_id: u32,
        visited: &mut AHashSet<SegmentKey>,
        queue: &mut std::collections::VecDeque<SegmentKey>,
    ) {
        if let Some(segs) = source.file_segments(file_id) {
            for idx in 0..segs.len() {
                let key = SegmentKey::new(file_id, idx as u32);
                if visited.insert(key) {
                    queue.push_back(key);
                }
            }
        }
    }

    /// Upward BFS: tag all segments that transitively import any of the `seeds`.
    ///
    /// Builds a reverse edge index (target → importers) from the `SourceGraph`,
    /// then BFS-es upward from `seeds` toward root segments. This finds all
    /// transitive "users" of the seeded segments.
    pub fn propagate_tags_to_users(
        &mut self,
        source: &SourceGraph,
        seeds: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
    ) {
        use std::collections::VecDeque;

        let reverse_edges = Self::build_reverse_edges(source, follow_type_only);

        let mut visited: AHashSet<SegmentKey> = AHashSet::default();
        let mut queue: VecDeque<SegmentKey> = VecDeque::new();

        for seed in seeds {
            if visited.insert(seed) {
                queue.push_back(seed);
            }
        }

        while let Some(key) = queue.pop_front() {
            self.set_tag(key, tag);

            if let Some(importers) = reverse_edges.get(&key) {
                for &importer_key in importers {
                    if visited.insert(importer_key) {
                        queue.push_back(importer_key);
                    }
                }
            }
        }
    }

    /// Build a reverse edge index: for each segment B that is imported by A,
    /// record A in B's reverse-edge set.
    ///
    /// Iterates all segments, resolves their forward edges (imports, dynamic
    /// imports, requires, executed paths, intra-file escaped symbols), and
    /// records the reverse mapping.
    fn build_reverse_edges(
        source: &SourceGraph,
        follow_type_only: bool,
    ) -> AHashMap<SegmentKey, Vec<SegmentKey>> {
        let mut reverse: AHashMap<SegmentKey, Vec<SegmentKey>> = AHashMap::default();

        for (key, segment) in source.iter_segments() {
            let resolved_import_paths = source
                .file_resolved_import_paths(key.file_id)
                .cloned()
                .unwrap_or_default();

            // Inter-file: static imports
            for (specifier, tagged_symbols) in &segment.module_deps.imports {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };

                for tagged_sym in tagged_symbols {
                    if !follow_type_only && tagged_sym.tags.is_type_only {
                        continue;
                    }
                    let targets =
                        Self::resolve_symbol_import(source, target_file_id, &tagged_sym.symbol);
                    for target_key in targets {
                        reverse.entry(target_key).or_default().push(key);
                    }
                }
            }

            // Inter-file: dynamic imports
            for (specifier, symbols) in &segment.module_deps.dynamic_imports {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };

                for sym in symbols {
                    let targets = Self::resolve_symbol_import(source, target_file_id, sym);
                    for target_key in targets {
                        reverse.entry(target_key).or_default().push(key);
                    }
                }
            }

            // Inter-file: requires
            for specifier in &segment.module_deps.requires {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };
                if let Some(segs) = source.file_segments(target_file_id) {
                    for idx in 0..segs.len() {
                        let target_key = SegmentKey::new(target_file_id, idx as u32);
                        reverse.entry(target_key).or_default().push(key);
                    }
                }
            }

            // Inter-file: side-effect-only imports
            for specifier in &segment.module_deps.executed_paths {
                let target_path = match resolved_import_paths.get(specifier) {
                    Some(p) => p,
                    None => continue,
                };
                let target_file_id = match source.file_id(target_path) {
                    Some(id) => id,
                    None => continue,
                };
                if let Some(segs) = source.file_segments(target_file_id) {
                    for idx in 0..segs.len() {
                        let target_key = SegmentKey::new(target_file_id, idx as u32);
                        reverse.entry(target_key).or_default().push(key);
                    }
                }
            }

            // Intra-file: escaped symbols
            for escaped_name in segment.variables.get_escaped_symbols() {
                if let Some(target_key) = source.resolve_symbol_in_file(
                    key.file_id,
                    escaped_name.as_ref(),
                    key.segment_idx,
                ) {
                    if target_key != key {
                        reverse.entry(target_key).or_default().push(key);
                    }
                }
            }
        }

        reverse
    }
}

impl Default for TagGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahashmap::AHashMap;
    use source_graph::{SourceFileInput, SourceGraph};

    use ast_segmenter::raw_module_deps::RawModuleDeps;
    use ast_segmenter::Segment;
    use ast_name_tracker::visitor::VariableScope;
    use swc_common::{BytePos, Span};

    fn make_span(lo: u32, hi: u32) -> Span {
        Span::new(BytePos(lo), BytePos(hi))
    }

    fn make_segment(lo: u32, hi: u32) -> Segment {
        Segment {
            span: make_span(lo, hi),
            module_deps: RawModuleDeps::default(),
            variables: VariableScope::new(),
        }
    }

    fn make_source_graph(file_segments: Vec<(&str, Vec<Segment>)>) -> SourceGraph {
        SourceGraph::new(file_segments.into_iter().map(|(path, segments)| {
            SourceFileInput {
                source_file_path: path.into(),
                segments,
                resolved_reexport_paths: AHashMap::default(),
                resolved_import_paths: AHashMap::default(),
            }
        }))
    }

    #[test]
    fn test_get_tag_untagged_returns_empty() {
        let tg = TagGraph::new();
        let key = SegmentKey::new(0, 0);
        assert_eq!(tg.get_tag(key), UsedTag::empty());
    }

    #[test]
    fn test_set_and_get_tag() {
        let mut tg = TagGraph::new();
        let key = SegmentKey::new(0, 0);
        tg.set_tag(key, UsedTag::FROM_ENTRY);
        assert_eq!(tg.get_tag(key), UsedTag::FROM_ENTRY);
    }

    #[test]
    fn test_set_tag_unions_flags() {
        let mut tg = TagGraph::new();
        let key = SegmentKey::new(0, 0);
        tg.set_tag(key, UsedTag::FROM_ENTRY);
        tg.set_tag(key, UsedTag::FROM_TEST);
        assert_eq!(
            tg.get_tag(key),
            UsedTag::FROM_ENTRY | UsedTag::FROM_TEST
        );
    }

    #[test]
    fn test_file_tag_unions_all_segments() {
        let sg = make_source_graph(vec![
            ("a.ts", vec![make_segment(0, 10), make_segment(10, 20), make_segment(20, 30)]),
        ]);
        let mut tg = TagGraph::new();

        // Tag first segment as entry, third as test
        tg.set_tag(SegmentKey::new(0, 0), UsedTag::FROM_ENTRY);
        tg.set_tag(SegmentKey::new(0, 2), UsedTag::FROM_TEST);

        let tag = tg.file_tag(&sg, 0);
        assert_eq!(tag, UsedTag::FROM_ENTRY | UsedTag::FROM_TEST);
    }

    #[test]
    fn test_file_tag_empty_when_no_tags() {
        let sg = make_source_graph(vec![
            ("a.ts", vec![make_segment(0, 10), make_segment(10, 20)]),
        ]);
        let tg = TagGraph::new();
        assert_eq!(tg.file_tag(&sg, 0), UsedTag::empty());
    }

    #[test]
    fn test_file_tag_unknown_file_returns_empty() {
        let sg = make_source_graph(vec![
            ("a.ts", vec![make_segment(0, 10)]),
        ]);
        let tg = TagGraph::new();
        // file_id 99 doesn't exist
        assert_eq!(tg.file_tag(&sg, 99), UsedTag::empty());
    }

    #[test]
    fn test_multiple_files_independent_tags() {
        let sg = make_source_graph(vec![
            ("a.ts", vec![make_segment(0, 10)]),
            ("b.ts", vec![make_segment(0, 10)]),
        ]);
        let mut tg = TagGraph::new();

        tg.set_tag(SegmentKey::new(0, 0), UsedTag::FROM_ENTRY);
        tg.set_tag(SegmentKey::new(1, 0), UsedTag::FROM_IGNORED);

        assert_eq!(tg.file_tag(&sg, 0), UsedTag::FROM_ENTRY);
        assert_eq!(tg.file_tag(&sg, 1), UsedTag::FROM_IGNORED);
    }

    #[test]
    fn test_used_tag_display() {
        let tag = UsedTag::FROM_ENTRY | UsedTag::FROM_TEST;
        assert_eq!(format!("{}", tag), "entry+test");
    }

    // -- propagate_tags_to_used tests --

    use ast_segmenter::raw_module_deps::{Symbol, SymbolTags, TaggedSymbol};
    use ast_name_tracker::visitor::HoistingLevel;
    use swc_atoms::Atom;

    /// Build a segment that imports `symbol` from `specifier`.
    fn segment_importing(specifier: &str, symbol: Symbol, is_type_only: bool) -> Segment {
        let mut deps = RawModuleDeps::default();
        let tagged = TaggedSymbol {
            symbol,
            tags: SymbolTags {
                allow_unused_comment: false,
                is_type_only,
            },
            span: make_span(0, 10),
        };
        deps.imports
            .entry(specifier.to_string())
            .or_default()
            .insert(tagged);
        Segment {
            span: make_span(0, 10),
            module_deps: deps,
            variables: VariableScope::new(),
        }
    }

    /// Build a segment that exports `name` as a local export.
    fn segment_exporting(name: &str) -> Segment {
        let mut deps = RawModuleDeps::default();
        let sym = ast_segmenter::ExportedSymbol::from(name);
        let tagged = TaggedSymbol::new(
            Symbol::from(name),
            SymbolTags::default(),
        );
        deps.exports_locals.insert(sym, tagged);
        Segment {
            span: make_span(0, 10),
            module_deps: deps,
            variables: VariableScope::new(),
        }
    }

    /// Build a segment with an escaped symbol reference (intra-file dependency).
    fn segment_with_escaped(escaped_name: &str) -> Segment {
        let mut vars = VariableScope::new();
        vars.insert_escaped(Atom::from(escaped_name));
        Segment {
            span: make_span(0, 10),
            module_deps: RawModuleDeps::default(),
            variables: vars,
        }
    }

    /// Build a segment that declares a local variable.
    fn segment_with_local_decl(name: &str, hoisting: HoistingLevel) -> Segment {
        let mut vars = VariableScope::new();
        vars.insert_local(Atom::from(name), hoisting);
        Segment {
            span: make_span(0, 10),
            module_deps: RawModuleDeps::default(),
            variables: vars,
        }
    }

    #[test]
    fn test_propagate_linear_chain() {
        // A(seg0) imports foo from B, B(seg0) exports foo
        // Root: A(0,0). Should tag both A(0,0) and B(1,0).
        let sg = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![segment_importing("./b", Symbol::Named("foo".into()), false)],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![segment_exporting("foo")],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_used(&sg, vec![SegmentKey::new(0, 0)], UsedTag::FROM_ENTRY, false);

        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_ENTRY);
    }

    #[test]
    fn test_propagate_diamond_dependency() {
        // A imports from B and C. Both B and C import from D.
        // Root: A. All should be tagged.
        //   A → B → D
        //   A → C → D
        let sg = SourceGraph::new(
            vec![
                // A: imports foo from B and bar from C
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.imports
                            .entry("./b".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(Symbol::Named("foo".into()), SymbolTags::default()));
                        deps.imports
                            .entry("./c".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(Symbol::Named("bar".into()), SymbolTags::default()));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m.insert("./c".to_string(), "c.ts".into());
                        m
                    },
                },
                // B: exports foo, imports baz from D
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.exports_locals.insert(
                            ast_segmenter::ExportedSymbol::from("foo"),
                            TaggedSymbol::new(Symbol::from("foo"), SymbolTags::default()),
                        );
                        deps.imports
                            .entry("./d".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(Symbol::Named("baz".into()), SymbolTags::default()));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./d".to_string(), "d.ts".into());
                        m
                    },
                },
                // C: exports bar, imports baz from D
                SourceFileInput {
                    source_file_path: "c.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.exports_locals.insert(
                            ast_segmenter::ExportedSymbol::from("bar"),
                            TaggedSymbol::new(Symbol::from("bar"), SymbolTags::default()),
                        );
                        deps.imports
                            .entry("./d".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(Symbol::Named("baz".into()), SymbolTags::default()));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./d".to_string(), "d.ts".into());
                        m
                    },
                },
                // D: exports baz
                SourceFileInput {
                    source_file_path: "d.ts".into(),
                    segments: vec![segment_exporting("baz")],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_used(&sg, vec![SegmentKey::new(0, 0)], UsedTag::FROM_ENTRY, false);

        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY); // A
        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_ENTRY); // B
        assert_eq!(tg.get_tag(SegmentKey::new(2, 0)), UsedTag::FROM_ENTRY); // C
        assert_eq!(tg.get_tag(SegmentKey::new(3, 0)), UsedTag::FROM_ENTRY); // D
    }

    #[test]
    fn test_propagate_type_only_skipped() {
        // A imports foo from B as type-only. B exports foo.
        // With follow_type_only=false, B should NOT be tagged.
        let sg = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![segment_importing("./b", Symbol::Named("foo".into()), true)],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![segment_exporting("foo")],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        // follow_type_only = false → skip type-only edges
        let mut tg = TagGraph::new();
        tg.propagate_tags_to_used(&sg, vec![SegmentKey::new(0, 0)], UsedTag::FROM_ENTRY, false);
        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::empty());

        // follow_type_only = true → follow type-only edges
        let mut tg2 = TagGraph::new();
        tg2.propagate_tags_to_used(&sg, vec![SegmentKey::new(0, 0)], UsedTag::FROM_ENTRY, true);
        assert_eq!(tg2.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
        assert_eq!(tg2.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_ENTRY);
    }

    #[test]
    fn test_propagate_cycle_terminates() {
        // A imports foo from B, B imports bar from A. Both export their symbol.
        // Should terminate without panic and tag both.
        let sg = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.exports_locals.insert(
                            ast_segmenter::ExportedSymbol::from("bar"),
                            TaggedSymbol::new(Symbol::from("bar"), SymbolTags::default()),
                        );
                        deps.imports
                            .entry("./b".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(Symbol::Named("foo".into()), SymbolTags::default()));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.exports_locals.insert(
                            ast_segmenter::ExportedSymbol::from("foo"),
                            TaggedSymbol::new(Symbol::from("foo"), SymbolTags::default()),
                        );
                        deps.imports
                            .entry("./a".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(Symbol::Named("bar".into()), SymbolTags::default()));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./a".to_string(), "a.ts".into());
                        m
                    },
                },
            ]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_used(&sg, vec![SegmentKey::new(0, 0)], UsedTag::FROM_ENTRY, false);

        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_ENTRY);
    }

    #[test]
    fn test_propagate_intra_file_escaped_symbol() {
        // File with 2 segments:
        //   seg0: declares local "helper" (let/const)
        //   seg1: references "helper" via escaped symbol
        // Root: seg1. Should tag both seg1 and seg0.
        let sg = SourceGraph::new(
            vec![SourceFileInput {
                source_file_path: "a.ts".into(),
                segments: vec![
                    segment_with_local_decl("helper", HoistingLevel::LetConstHoisting),
                    segment_with_escaped("helper"),
                ],
                resolved_reexport_paths: AHashMap::default(),
                resolved_import_paths: AHashMap::default(),
            }]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_used(&sg, vec![SegmentKey::new(0, 1)], UsedTag::FROM_ENTRY, false);

        assert_eq!(tg.get_tag(SegmentKey::new(0, 1)), UsedTag::FROM_ENTRY);
        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
    }

    // -- propagate_tags_to_users (upward) tests --

    #[test]
    fn test_upward_leaf_propagates_to_importer() {
        // A(seg0) imports foo from B. B(seg0) exports foo.
        // Seed: B(1,0). Should tag B(1,0) and A(0,0) (A uses B).
        let sg = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![segment_importing("./b", Symbol::Named("foo".into()), false)],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![segment_exporting("foo")],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_users(&sg, vec![SegmentKey::new(1, 0)], UsedTag::FROM_TEST, false);

        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_TEST); // B (seed)
        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_TEST); // A (importer)
    }

    #[test]
    fn test_upward_mid_graph_propagates_to_all_importers() {
        // A → B → C (A imports B, B imports C)
        // Seed: B(1,0). Should tag B and A (transitive importer), but NOT C.
        let sg = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![segment_importing("./b", Symbol::Named("foo".into()), false)],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.exports_locals.insert(
                            ast_segmenter::ExportedSymbol::from("foo"),
                            TaggedSymbol::new(Symbol::from("foo"), SymbolTags::default()),
                        );
                        deps.imports
                            .entry("./c".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(
                                Symbol::Named("bar".into()),
                                SymbolTags::default(),
                            ));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./c".to_string(), "c.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "c.ts".into(),
                    segments: vec![segment_exporting("bar")],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_users(&sg, vec![SegmentKey::new(1, 0)], UsedTag::FROM_IGNORED, false);

        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_IGNORED); // B (seed)
        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_IGNORED); // A (transitive importer)
        assert_eq!(tg.get_tag(SegmentKey::new(2, 0)), UsedTag::empty());      // C (downstream, NOT tagged)
    }

    #[test]
    fn test_upward_does_not_traverse_downward() {
        // A → B → C. Seed: B. Only A and B get tagged, not C.
        // This is the same graph as above but explicitly verifies
        // that upward propagation does NOT follow forward (downward) edges.
        let sg = SourceGraph::new(
            vec![
                SourceFileInput {
                    source_file_path: "a.ts".into(),
                    segments: vec![segment_importing("./b", Symbol::Named("x".into()), false)],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./b".to_string(), "b.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "b.ts".into(),
                    segments: vec![{
                        let mut deps = RawModuleDeps::default();
                        deps.exports_locals.insert(
                            ast_segmenter::ExportedSymbol::from("x"),
                            TaggedSymbol::new(Symbol::from("x"), SymbolTags::default()),
                        );
                        deps.imports
                            .entry("./c".to_string())
                            .or_default()
                            .insert(TaggedSymbol::new(
                                Symbol::Named("y".into()),
                                SymbolTags::default(),
                            ));
                        Segment {
                            span: make_span(0, 10),
                            module_deps: deps,
                            variables: VariableScope::new(),
                        }
                    }],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: {
                        let mut m = AHashMap::default();
                        m.insert("./c".to_string(), "c.ts".into());
                        m
                    },
                },
                SourceFileInput {
                    source_file_path: "c.ts".into(),
                    segments: vec![segment_exporting("y")],
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                },
            ]
            .into_iter(),
        );

        let mut tg = TagGraph::new();
        tg.propagate_tags_to_users(&sg, vec![SegmentKey::new(1, 0)], UsedTag::FROM_ENTRY, false);

        // B is the seed — tagged
        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_ENTRY);
        // A imports B — tagged via upward propagation
        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_ENTRY);
        // C is imported BY B (downstream) — must NOT be tagged
        assert_eq!(tg.get_tag(SegmentKey::new(2, 0)), UsedTag::empty());
    }
}
