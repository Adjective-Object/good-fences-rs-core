use ahashmap::{AHashMap, AHashSet};
use source_graph::{
    EdgeKind, EdgePurpose, GraphNode, GraphRevision, OccurrenceEdge, SegmentKey, SourceGraph,
};

use core::fmt::{self, Display};

#[cfg(test)]
mod shared_tests;

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
///
/// Files with no segments (e.g., empty files) are tracked via a separate
/// `file_tags` map so they can still be marked as "used" when reached by BFS.
pub struct TagGraph {
    revision: Option<GraphRevision>,
    tags: AHashMap<SegmentKey, UsedTag>,
    /// Direct file-level tags for files with no segments.
    file_tags: AHashMap<u32, UsedTag>,
}

impl TagGraph {
    /// Discard analysis state after any structural revision.
    pub fn synchronize(&mut self, source: &SourceGraph) {
        if self
            .revision
            .is_some_and(|revision| revision != source.revision())
        {
            self.tags.clear();
            self.file_tags.clear();
        }
        self.revision = Some(source.revision());
    }

    /// Report-only conservative statement-order policy, not structural effects.
    pub fn propagate_tags_for_report(
        &mut self,
        source: &SourceGraph,
        roots: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
        previous: &AHashMap<SegmentKey, SegmentKey>,
    ) {
        self.walk(source, roots, tag, follow_type_only, false, Some(previous));
    }

    fn follows(edge: &OccurrenceEdge, follow_type_only: bool) -> bool {
        edge.purpose != EdgePurpose::Evaluation && (follow_type_only || !edge.type_only)
    }

    fn propagate(
        &mut self,
        source: &SourceGraph,
        roots: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
        reverse: bool,
    ) {
        self.walk(source, roots, tag, follow_type_only, reverse, None);
    }

    fn walk(
        &mut self,
        source: &SourceGraph,
        roots: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
        reverse: bool,
        previous: Option<&AHashMap<SegmentKey, SegmentKey>>,
    ) {
        self.synchronize(source);
        let mut queue: std::collections::VecDeque<_> =
            roots.into_iter().map(GraphNode::Segment).collect();
        let mut visited = AHashSet::default();
        let mut files = AHashSet::default();
        while let Some(node) = queue.pop_front() {
            if !visited.insert(node) {
                continue;
            }
            let file_id = match node {
                GraphNode::Segment(key) => {
                    if source
                        .file_segments(key.file_id)
                        .and_then(|s| s.get(key.segment_idx as usize))
                        .is_none()
                    {
                        continue;
                    }
                    self.set_tag(key, tag);
                    if let Some(previous) = previous.and_then(|p| p.get(&key)) {
                        queue.push_back(GraphNode::Segment(*previous));
                    }
                    key.file_id
                }
                GraphNode::Module(id) => {
                    if !reverse {
                        self.set_file_tag(id, tag);
                        if let Some(segments) = source.file_segments(id) {
                            queue.extend(
                                (0..segments.len())
                                    .map(|i| GraphNode::Segment(SegmentKey::new(id, i as u32))),
                            );
                        }
                    }
                    id
                }
                GraphNode::Unlinked { .. } => continue,
            };
            if reverse {
                for edge in source
                    .incoming(node)
                    .chain(source.incoming(GraphNode::Module(file_id)))
                {
                    if Self::follows(edge, follow_type_only) {
                        queue.push_back(edge.from);
                    }
                }
            } else {
                if files.insert(file_id) {
                    if let Some(segments) = source.file_segments(file_id) {
                        for i in 0..segments.len() {
                            let site = GraphNode::Segment(SegmentKey::new(file_id, i as u32));
                            if source.outgoing(site).any(|e| {
                                e.kind == EdgeKind::Import
                                    && e.purpose == EdgePurpose::Execution
                                    && Self::follows(e, follow_type_only)
                            }) {
                                queue.push_back(site);
                            }
                        }
                    }
                }
                for edge in source.outgoing(node) {
                    if Self::follows(edge, follow_type_only) {
                        queue.push_back(edge.to);
                    }
                }
            }
        }
    }

    /// Create a new empty `TagGraph`.
    pub fn new() -> Self {
        TagGraph {
            revision: None,
            tags: AHashMap::default(),
            file_tags: AHashMap::default(),
        }
    }

    /// Get the tag for a specific segment. Returns `UsedTag::empty()` if
    /// the segment has not been tagged.
    pub fn get_tag(&self, key: SegmentKey) -> UsedTag {
        self.tags.get(&key).copied().unwrap_or_else(UsedTag::empty)
    }

    /// Set (union) a tag on a specific segment.
    pub fn set_tag(&mut self, key: SegmentKey, tag: UsedTag) {
        *self.tags.entry(key).or_insert_with(UsedTag::empty) |= tag;
    }

    /// Set (union) a tag directly on a file (for files with no segments).
    pub fn set_file_tag(&mut self, file_id: u32, tag: UsedTag) {
        *self.file_tags.entry(file_id).or_insert_with(UsedTag::empty) |= tag;
    }

    /// Derived file-level tag: union of all segment tags for segments
    /// belonging to `file_id`.
    ///
    /// Returns `UsedTag::empty()` if the file has no segments or no
    /// segments have been tagged.
    pub fn file_tag(&self, source: &SourceGraph, file_id: u32) -> UsedTag {
        if self
            .revision
            .is_some_and(|revision| revision != source.revision())
        {
            return UsedTag::empty();
        }
        // Start with any direct file-level tag (for files with no segments)
        let mut combined = self
            .file_tags
            .get(&file_id)
            .copied()
            .unwrap_or_else(UsedTag::empty);

        let segments = match source.file_segments(file_id) {
            Some(segs) => segs,
            None => return combined,
        };

        for seg_idx in 0..segments.len() {
            let key = SegmentKey::new(file_id, seg_idx as u32);
            combined |= self.get_tag(key);
        }
        combined
    }
}

impl TagGraph {
    /// Downward usage traversal over shared structural edges. Async sites remain
    /// ordinary usage dependencies; this is not a bundler activation algorithm.
    pub fn propagate_tags_to_used(
        &mut self,
        source: &SourceGraph,
        roots: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
    ) {
        self.propagate(source, roots, tag, follow_type_only, false);
    }

    /// Upward BFS: tag all segments that transitively import any of the `seeds`.
    ///
    /// Reads SourceGraph's reverse index, including namespace/module consumers.
    pub fn propagate_tags_to_users(
        &mut self,
        source: &SourceGraph,
        seeds: Vec<SegmentKey>,
        tag: UsedTag,
        follow_type_only: bool,
    ) {
        self.propagate(source, seeds, tag, follow_type_only, true);
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
    use ast_segmenter::variables::VariableScope;
    use ast_segmenter::Segment;
    use oxc_span::Span;

    fn make_span(lo: u32, hi: u32) -> Span {
        Span::new(lo, hi)
    }

    fn make_segment(lo: u32, hi: u32) -> Segment {
        Segment {
            span: make_span(lo, hi),
            module_deps: RawModuleDeps::default(),
            variables: VariableScope::new(),
        }
    }

    fn make_source_graph(file_segments: Vec<(&str, Vec<Segment>)>) -> SourceGraph {
        SourceGraph::new(
            file_segments
                .into_iter()
                .map(|(path, segments)| SourceFileInput {
                    source_file_path: path.into(),
                    segments,
                    resolved_reexport_paths: AHashMap::default(),
                    resolved_import_paths: AHashMap::default(),
                }),
        )
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
        assert_eq!(tg.get_tag(key), UsedTag::FROM_ENTRY | UsedTag::FROM_TEST);
    }

    #[test]
    fn test_file_tag_unions_all_segments() {
        let sg = make_source_graph(vec![(
            "a.ts",
            vec![
                make_segment(0, 10),
                make_segment(10, 20),
                make_segment(20, 30),
            ],
        )]);
        let mut tg = TagGraph::new();

        // Tag first segment as entry, third as test
        tg.set_tag(SegmentKey::new(0, 0), UsedTag::FROM_ENTRY);
        tg.set_tag(SegmentKey::new(0, 2), UsedTag::FROM_TEST);

        let tag = tg.file_tag(&sg, 0);
        assert_eq!(tag, UsedTag::FROM_ENTRY | UsedTag::FROM_TEST);
    }

    #[test]
    fn test_file_tag_empty_when_no_tags() {
        let sg = make_source_graph(vec![(
            "a.ts",
            vec![make_segment(0, 10), make_segment(10, 20)],
        )]);
        let tg = TagGraph::new();
        assert_eq!(tg.file_tag(&sg, 0), UsedTag::empty());
    }

    #[test]
    fn test_file_tag_unknown_file_returns_empty() {
        let sg = make_source_graph(vec![("a.ts", vec![make_segment(0, 10)])]);
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
    use ast_segmenter::variables::HoistingLevel;
    use compact_str::CompactString;

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
        let tagged = TaggedSymbol::new(Symbol::from(name), SymbolTags::default());
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
        vars.insert_escaped(CompactString::new(escaped_name));
        Segment {
            span: make_span(0, 10),
            module_deps: RawModuleDeps::default(),
            variables: vars,
        }
    }

    /// Build a segment that declares a local variable.
    fn segment_with_local_decl(name: &str, hoisting: HoistingLevel) -> Segment {
        let mut vars = VariableScope::new();
        vars.insert_local_str(name, hoisting);
        Segment {
            span: make_span(0, 10),
            module_deps: RawModuleDeps::default(),
            variables: vars,
        }
    }

    #[test]
    fn test_type_only_intra_file_edges_in_both_directions() {
        for runtime_first in [false, true] {
            let mut type_user = make_segment(10, 20);
            type_user
                .variables
                .insert_escaped_reference("Value".into(), true);
            let mut mixed_user = make_segment(20, 30);
            for is_type_only in [!runtime_first, runtime_first] {
                mixed_user
                    .variables
                    .insert_escaped_reference("Value".into(), is_type_only);
            }
            let sg = make_source_graph(vec![(
                "types.ts",
                vec![
                    segment_with_local_decl("Value", HoistingLevel::LetConstHoisting),
                    type_user,
                    mixed_user,
                ],
            )]);
            let declaration = SegmentKey::new(0, 0);
            let type_user = SegmentKey::new(0, 1);
            let mixed_user = SegmentKey::new(0, 2);
            for tag in [
                UsedTag::FROM_ENTRY,
                UsedTag::FROM_TEST,
                UsedTag::FROM_IGNORED,
            ] {
                for follow_type_only in [false, true] {
                    let expected = if follow_type_only {
                        tag
                    } else {
                        UsedTag::empty()
                    };
                    let mut down = TagGraph::new();
                    down.propagate_tags_to_used(&sg, vec![type_user], tag, follow_type_only);
                    assert_eq!(down.get_tag(type_user), tag, "explicit type root");
                    assert_eq!(down.get_tag(declaration), expected);

                    let mut mixed = TagGraph::new();
                    mixed.propagate_tags_to_used(&sg, vec![mixed_user], tag, follow_type_only);
                    assert_eq!(
                        mixed.get_tag(declaration),
                        tag,
                        "mixed reference is runtime"
                    );

                    let mut up = TagGraph::new();
                    up.propagate_tags_to_users(&sg, vec![declaration], tag, follow_type_only);
                    assert_eq!(up.get_tag(declaration), tag);
                    assert_eq!(up.get_tag(type_user), expected);
                    assert_eq!(up.get_tag(mixed_user), tag);
                }
            }
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
                        deps.imports.entry("./b".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("foo".into()), SymbolTags::default()),
                        );
                        deps.imports.entry("./c".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("bar".into()), SymbolTags::default()),
                        );
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
                        deps.imports.entry("./d".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("baz".into()), SymbolTags::default()),
                        );
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
                        deps.imports.entry("./d".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("baz".into()), SymbolTags::default()),
                        );
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
                        deps.imports.entry("./b".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("foo".into()), SymbolTags::default()),
                        );
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
                        deps.imports.entry("./a".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("bar".into()), SymbolTags::default()),
                        );
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
                        deps.imports.entry("./c".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("bar".into()), SymbolTags::default()),
                        );
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
        tg.propagate_tags_to_users(
            &sg,
            vec![SegmentKey::new(1, 0)],
            UsedTag::FROM_IGNORED,
            false,
        );

        assert_eq!(tg.get_tag(SegmentKey::new(1, 0)), UsedTag::FROM_IGNORED); // B (seed)
        assert_eq!(tg.get_tag(SegmentKey::new(0, 0)), UsedTag::FROM_IGNORED); // A (transitive importer)
        assert_eq!(tg.get_tag(SegmentKey::new(2, 0)), UsedTag::empty()); // C (downstream, NOT tagged)
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
                        deps.imports.entry("./c".to_string()).or_default().insert(
                            TaggedSymbol::new(Symbol::Named("y".into()), SymbolTags::default()),
                        );
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
