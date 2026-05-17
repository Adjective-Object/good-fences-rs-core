use crate::segment_key::SegmentKey;

/// A directed edge between two segments in the source graph.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct SegmentEdge {
    pub from: SegmentKey,
    pub to: SegmentKey,
    pub is_type_only: bool,
}
