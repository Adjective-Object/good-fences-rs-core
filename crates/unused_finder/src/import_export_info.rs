use crate::node_visitor::{ExportedItemMetadata, ImportedItem};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ImportExportInfo {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_path_ids: HashMap<String, HashSet<ImportedItem>>,
    // require('foo') generates ['foo']
    pub require_paths: HashSet<String>,
    // import('./foo') generates ["./foo"]
    pub imported_paths: HashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: HashMap<String, HashSet<ImportedItem>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exported_ids: HashSet<ExportedItem>,
    // `import './foo'`
    pub executed_paths: HashSet<String>,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Default)]
pub struct ExportedItem {
    pub metadata: ExportedItemMetadata,
    pub source_file_path: PathBuf,
}

impl ImportExportInfo {
    pub fn new() -> Self {
        Self {
            imported_path_ids: HashMap::new(),
            require_paths: HashSet::new(),
            imported_paths: HashSet::new(),
            export_from_ids: HashMap::new(),
            exported_ids: HashSet::new(),
            executed_paths: HashSet::new(),
        }
    }
}

impl Default for ImportExportInfo {
    fn default() -> Self {
        Self::new()
    }
}
