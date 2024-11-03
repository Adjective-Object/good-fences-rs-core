use std::path::{Path, PathBuf};

use ahashmap::{AHashMap, AHashSet, ARandomState};
use anyhow::Result;
use swc_common::{FileName, Span};
use swc_ecma_ast::ModuleExportName;
use swc_ecma_loader::resolve::Resolve;

#[derive(Debug, PartialEq, Eq, Hash, Clone, PartialOrd, Ord)]
pub enum ExportedSymbol {
    // A named export
    Named(String),
    // The default export
    Default,
    // A namespace export
    Namespace,
    ExecutionOnly, // in case of `import './foo';` this executes code in file but imports nothing
}

impl std::fmt::Display for ExportedSymbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportedSymbol::Named(name) => write!(f, "{}", name),
            ExportedSymbol::Default => write!(f, "default"),
            ExportedSymbol::Namespace => write!(f, "*"),
            ExportedSymbol::ExecutionOnly => write!(f, "import '<path>'"),
        }
    }
}

impl Default for ExportedSymbol {
    fn default() -> Self {
        Self::Default
    }
}

impl From<&ModuleExportName> for ExportedSymbol {
    fn from(e: &ModuleExportName) -> Self {
        match e.atom().as_str() {
            "default" => ExportedSymbol::Default,
            _ => ExportedSymbol::Named(e.atom().as_str().to_string()),
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq, Clone, Hash)]
pub struct ExportedSymbolMetadata {
    pub span: Span,
    pub allow_unused: bool,
}

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub struct ReExportedSymbol {
    /// The symbol being re-exported from another module
    pub imported: ExportedSymbol,
    /// If the symbol is renamed, this field contains the new name.
    ///  (e.g. the export { _ as foo } from './foo' generates `renamed_to: Some("foo".to_string())`)
    pub renamed_to: Option<ExportedSymbol>,
}

/// Represents the raw import/export information from a file, where import
/// specifiers are not yet resolved to their final paths.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RawImportExportInfo {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_path_ids: AHashMap<String, AHashSet<ExportedSymbol>>,
    // require('foo') generates ['foo']
    pub require_paths: AHashSet<String>,
    // import('./foo') generates ["./foo"]
    pub imported_paths: AHashSet<String>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_ids: AHashMap<String, AHashSet<ReExportedSymbol>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exported_ids: AHashMap<ExportedSymbol, ExportedSymbolMetadata>,
    // `import './foo'`
    pub executed_paths: AHashSet<String>,
}

/// Represents the raw import/export information from a file, where import
/// specifiers are not yet resolved to their final paths.
#[derive(Default, Debug, PartialEq, Eq, Clone)]
pub struct ResolvedImportExportInfo {
    // `import foo, {bar as something} from './foo'` generates `{ "./foo": ["default", "bar"] }`
    pub imported_symbols: AHashMap<PathBuf, AHashSet<ExportedSymbol>>,
    // require('foo') generates ['foo']
    pub require_paths: AHashSet<PathBuf>,
    // import('./foo') generates ["./foo"]
    pub imported_paths: AHashSet<PathBuf>,
    // `export {default as foo, bar} from './foo'` generates { "./foo": ["default", "bar"] }
    pub export_from_symbols: AHashMap<PathBuf, AHashSet<ReExportedSymbol>>,
    // `export default foo` and `export {foo}` generate `Default` and `Named("foo")` respectively
    pub exported_ids: AHashMap<ExportedSymbol, ExportedSymbolMetadata>,
    // `import './foo'`
    pub executed_paths: AHashSet<PathBuf>,
}

impl ResolvedImportExportInfo {
    pub fn num_exported_symbols(&self) -> usize {
        self.exported_ids.len() + self.export_from_symbols.len()
    }

    /// Returns an iterator over all the imports originating from this file.
    pub fn iter_exported_symbols(&self) -> impl Iterator<Item = (Option<&Path>, &ExportedSymbol)> {
        let export_from_symbols = self.export_from_symbols.iter().flat_map(|(path, symbols)| {
            symbols.iter().map(|symbol| {
                (
                    Some(path.as_path()),
                    symbol.renamed_to.as_ref().unwrap_or(&symbol.imported),
                )
            })
        });

        let exported_ids = self.exported_ids.keys().map(|symbol| (None, symbol));

        exported_ids.chain(export_from_symbols)
    }

    /// Returns an iterator over all the imports originating from this file.
    pub fn iter_imported_symbols(&self) -> impl Iterator<Item = (&PathBuf, ExportedSymbol)> {
        let imported_symbols = self
            .imported_symbols
            .iter()
            .flat_map(|(path, symbols)| symbols.iter().map(move |symbol| (path, symbol.clone())));

        let require_imports = self
            .require_paths
            .iter()
            .map(|path| (path, ExportedSymbol::Namespace));

        let imported_paths = self
            .imported_paths
            .iter()
            .map(|path| (path, ExportedSymbol::Namespace));

        let re_exports = self.export_from_symbols.iter().flat_map(|(path, symbols)| {
            symbols
                .iter()
                .map(move |symbol| (path, symbol.imported.clone()))
        });

        let executed_paths = self
            .executed_paths
            .iter()
            .map(|path| (path, ExportedSymbol::ExecutionOnly));

        imported_symbols
            .chain(require_imports)
            .chain(imported_paths)
            .chain(re_exports)
            .chain(executed_paths)
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Hash, Default)]
pub struct ExportedItem {
    pub metadata: ExportedSymbolMetadata,
    pub source_file_path: Option<String>,
}

impl RawImportExportInfo {
    pub fn new() -> Self {
        Self {
            imported_path_ids: AHashMap::default(),
            require_paths: AHashSet::default(),
            imported_paths: AHashSet::default(),
            export_from_ids: AHashMap::default(),
            exported_ids: AHashMap::default(),
            executed_paths: AHashSet::default(),
        }
    }
}

impl Default for RawImportExportInfo {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_hashmap<T>(
    from_file: &FileName,
    resolver: impl Resolve,
    mut map: AHashMap<String, T>,
) -> Result<AHashMap<PathBuf, T>, anyhow::Error> {
    let mut accum = AHashMap::with_capacity_and_hasher(map.len(), ARandomState::new());
    for (import_specifier, imported_symbols) in map.drain() {
        let resolved = resolver.resolve(from_file, &import_specifier)?;
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
}

fn resolve_hashset(
    from_file: &FileName,
    resolver: impl Resolve,
    mut set: AHashSet<String>,
) -> Result<AHashSet<PathBuf>, anyhow::Error> {
    let mut accum = AHashSet::with_capacity_and_hasher(set.len(), ARandomState::new());
    for import_specifier in set.drain() {
        let resolved = resolver.resolve(from_file, &import_specifier)?;
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

        Ok(ResolvedImportExportInfo {
            imported_symbols: resolve_hashmap(&from_file, &resolver, imported_path_ids)?,
            require_paths: resolve_hashset(&from_file, &resolver, require_paths)?,
            imported_paths: resolve_hashset(&from_file, &resolver, imported_paths)?,
            export_from_symbols: resolve_hashmap(&from_file, &resolver, export_from_ids)?,
            exported_ids,
            executed_paths: resolve_hashset(&from_file, &resolver, executed_paths)?,
        })
    }
}
