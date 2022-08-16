#[doc("inline")]
use std::{fmt::Display, error::Error};

#[derive(Debug)]
pub enum GetImportError {
    /**
     * - String: file_path
     */
    ParseTsFileError(String),
    /**
     * 0 -> file_path
     * 1 -> import_path
     */
    ReadingImportError(String, String),
    ReadTsFileError(Option<String>),
}

impl Error for GetImportError {}

impl Display for GetImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GetImportError::ParseTsFileError(file_path) => write!(f, "Error parsing file: {}", file_path),
            GetImportError::ReadingImportError(file_path, import) => write!(f, "Error reading import names of {} inside file {}", import, file_path),
            GetImportError::ReadTsFileError(path_opt) => {
                match path_opt {
                    Some(file_path) => write!(f, "Could not read TS file {}", &file_path),
                    None => write!(f, "Invalid path, could not read TS file"),
                }
            },
        }
    }

}
