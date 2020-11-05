use crate::fence::Fence;
use crate::fence_collection::FenceCollection;
use crate::import_resolver::TsconfigPathsJson;
use crate::walk_dirs::SourceFile;
use find_ts_imports::SourceFileImportData;
use std::iter::Iterator;

pub fn evaluate_fences<'a, TFenceIterator>(
    fence_collection: &FenceCollection,
    tsconfig_paths_json: &TsconfigPathsJson,
    source_file: &SourceFile,
) where
    TFenceIterator: Iterator<Item = &'a Fence>,
{
    // let source_fences = fence_collection.get_fences_for_path(source_file.)
}
