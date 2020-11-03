use crate::fence::Fence;
use crate::walk_dirs::SourceFile;
use find_ts_imports::SourceFileImportData;
use std::iter::Iterator;

pub fn evaluate_fences<'a, TFenceIterator>(imports: SourceFileImportData, fences: TFenceIterator)
where
    TFenceIterator: Iterator<Item = &'a Fence>,
{
}
