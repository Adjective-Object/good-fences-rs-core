use crate::import_export_info::ImportExportInfo;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UnusedFinderSourceFile {
    pub package_name: String,
    pub source_file_path: String,
    pub import_export_info: ImportExportInfo,
}

#[derive(Debug, PartialEq)]
#[derive(Default)]
pub enum WalkedFile {
    SourceFile(Box<UnusedFinderSourceFile>),
    #[default]
    Nothing,
}

