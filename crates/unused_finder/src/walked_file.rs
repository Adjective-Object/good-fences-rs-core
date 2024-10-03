use crate::parse::FileImportExportInfo;
use packagejson::PackageJson;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UnusedFinderSourceFile {
    pub owned_package: String,
    pub is_exported: bool,
    pub source_file_path: String,
    pub import_export_info: FileImportExportInfo,
}

#[derive(Debug, PartialEq)]
pub enum WalkedFile {
    SourceFile(UnusedFinderSourceFile),
    // (path-to-package, package.json contents)
    PackageJson(String, PackageJson),
}
