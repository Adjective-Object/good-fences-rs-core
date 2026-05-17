use ahashmap::AHashMap;
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
}
