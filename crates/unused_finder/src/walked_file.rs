use crate::import_export_info::ImportExportInfo;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UnusedFinderSourceFile {
    pub package_name: String,
    pub source_file_path: String,
    pub import_export_info: ImportExportInfo,
}

#[derive(Debug, PartialEq)]
pub enum WalkedFile {
    SourceFile(UnusedFinderSourceFile),
    Nothing,
}

impl Default for WalkedFile {
    fn default() -> Self {
        WalkedFile::Nothing
    }
}
