use std::path::{Path, PathBuf};

use ahashmap::{AHashMap, AHashSet, ARandomState};
use anyhow::Result;
use swc_core::{
    common::{FileName, Span},
    ecma::loader::resolve::Resolve,
};

/// Represents an import of a module from another module
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

/// Represents the raw import/export information from a file, where import
/// specifiers are not yet resolved to their final paths.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RawImportExportInfo {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_path_ids: AHashMap<String, AHashSet<ImportedItem>>,
    // require('foo') generates ['foo']
    pub require_paths: AHashSet<String>,
    // import('./foo') generates ["./foo"]
    pub imported_paths: AHashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: AHashMap<String, AHashSet<ImportedItem>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exported_ids: AHashSet<ExportedItem>,
    // `import './foo'`
    pub executed_paths: AHashSet<String>,
}

/// Represents the raw import/export information from a file, where import
/// specifiers are not yet resolved to their final paths.
#[derive(Default, Debug, PartialEq, Eq, Clone)]
pub struct ResolvedImportExportInfo {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_path_ids: AHashMap<PathBuf, AHashSet<ImportedItem>>,
    // require('foo') generates ['foo']
    pub require_paths: AHashSet<PathBuf>,
    // import('./foo') generates ["./foo"]
    pub imported_paths: AHashSet<PathBuf>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: AHashMap<PathBuf, AHashSet<ImportedItem>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exported_ids: AHashSet<ExportedItem>,
    // `import './foo'`
    pub executed_paths: AHashSet<PathBuf>,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Default)]
pub struct ExportedItem {
    pub metadata: ExportedItemMetadata,
    pub source_file_path: Option<String>,
}

impl RawImportExportInfo {
    pub fn new() -> Self {
        Self {
            imported_path_ids: AHashMap::default(),
            require_paths: AHashSet::default(),
            imported_paths: AHashSet::default(),
            export_from_ids: AHashMap::default(),
            exported_ids: AHashSet::default(),
            executed_paths: AHashSet::default(),
        }
    }
}

impl Default for RawImportExportInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl RawImportExportInfo {
    pub fn try_resolve(
        self,
        from_file_path: &Path,
        resolver: impl Resolve,
    ) -> Result<ResolvedImportExportInfo, anyhow::Error> {
        let RawImportExportInfo {
            imported_path_ids,
            require_paths,
            imported_paths,
            export_from_ids,
            exported_ids,
            executed_paths,
        } = self;

        let from_file = FileName::Real(from_file_path.to_path_buf());

        let resolve_hashmap = |mut map: AHashMap<String, AHashSet<ImportedItem>>| {
            let mut accum = AHashMap::with_capacity_and_hasher(map.len(), ARandomState::new());
            for (import_specifier, imported_symbols) in map.drain() {
                let resolved = resolver.resolve(&from_file, &import_specifier)?;
                match resolved.filename {
                    FileName::Real(resolved_path) => {
                        accum.insert(resolved_path, imported_symbols);
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "resolved to a non-file path?: {:?}",
                            resolved
                        ));
                    }
                }
            }
            Ok(accum)
        };

        let resolve_hashset =
            |mut set: AHashSet<String>| -> Result<AHashSet<PathBuf>, anyhow::Error> {
                let mut accum = AHashSet::with_capacity_and_hasher(set.len(), ARandomState::new());
                for import_specifier in set.drain() {
                    let resolved = resolver.resolve(&from_file, &import_specifier)?;
                    let resolved_str = match resolved.filename {
                        FileName::Real(path) => path,
                        _ => {
                            return Err(anyhow::anyhow!(
                                "resolved to a non-file path?: {:?}",
                                resolved
                            ));
                        }
                    };
                    accum.insert(resolved_str);
                }

                Ok(accum)
            };

        Ok(ResolvedImportExportInfo {
            imported_path_ids: resolve_hashmap(imported_path_ids)?,
            require_paths: resolve_hashset(require_paths)?,
            imported_paths: resolve_hashset(imported_paths)?,
            export_from_ids: resolve_hashmap(export_from_ids)?,
            exported_ids,
            executed_paths: resolve_hashset(executed_paths)?,
        })
    }
}
