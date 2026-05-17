/// Uniquely identifies a segment within the source graph.
///
/// A segment is a contiguous portion of a source file (typically one
/// top-level statement or declaration). `file_id` indexes into the
/// graph's file list, and `segment_idx` indexes into that file's
/// segment vector.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Ord, PartialOrd)]
pub struct SegmentKey {
    pub file_id: u32,
    pub segment_idx: u32,
}

impl SegmentKey {
    pub fn new(file_id: u32, segment_idx: u32) -> Self {
        Self {
            file_id,
            segment_idx,
        }
    }
}
