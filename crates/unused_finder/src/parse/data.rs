use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
use swc_core::common::Span;

// Represents an import of a module from another module
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ImportedItem {
    Named(String),
    Default,
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl From<&ExportKind> for ImportedItem {
    fn from(e: &ExportKind) -> Self {
        match e {
            ExportKind::Named(name) => ImportedItem::Named(name.clone()),
            ExportKind::Default => ImportedItem::Default,
            ExportKind::Namespace => ImportedItem::Namespace,
            ExportKind::ExecutionOnly => ImportedItem::ExecutionOnly,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ExportKind {
    Named(String),
    Default,
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl std::fmt::Display for ExportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportKind::Named(name) => write!(f, "{}", name),
            ExportKind::Default => write!(f, "default"),
            ExportKind::Namespace => write!(f, "*"),
            ExportKind::ExecutionOnly => write!(f, "import '<path>'"),
        }
    }
}

impl Default for ExportKind {
    fn default() -> Self {
        Self::Default
    }
}

impl From<&ImportedItem> for ExportKind {
    fn from(i: &ImportedItem) -> Self {
        match i {
            ImportedItem::Named(named) => ExportKind::Named(named.clone()),
            ImportedItem::Default => ExportKind::Default,
            ImportedItem::Namespace => ExportKind::Namespace,
            ImportedItem::ExecutionOnly => ExportKind::ExecutionOnly,
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq, Clone, Hash)]
pub struct ExportedItemMetadata {
    pub export_kind: ExportKind,
    pub span: Span,
    pub allow_unused: bool,
}

impl ExportedItemMetadata {
    pub fn new(export_type: ExportKind, span: Span, allow_unused: bool) -> Self {
        Self {
            export_kind: export_type,
            span,
            allow_unused,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FileImportExportInfo {
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

impl FileImportExportInfo {
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

impl Default for FileImportExportInfo {
    fn default() -> Self {
        Self::new()
    }
}
