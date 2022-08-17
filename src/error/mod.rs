use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub enum GetImportErrorKind {
    ParseTsFileError,
    FileDoesNotExist,
    ReadImportError,
    ReadTsFileError,
}

#[derive(Debug)]
pub struct GetImportError {
    pub kind: GetImportErrorKind,
    pub swc_parser_errrors: Option<Vec<swc_ecma_parser::error::Error>>,
    pub io_errors: Option<Vec<std::io::Error>>,
    pub file_path: Option<String>,
}

impl GetImportError {
    pub fn new(
        kind: GetImportErrorKind,
        file_path: Option<String>,
        swc_parser_errrors: Option<Vec<swc_ecma_parser::error::Error>>,
        io_errors: Option<Vec<std::io::Error>>,
    ) -> GetImportError {
        Self {
            swc_parser_errrors,
            kind,
            file_path,
            io_errors,
        }
    }
}

impl Error for GetImportError {}

impl Display for GetImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            GetImportErrorKind::ParseTsFileError => {

            },
            GetImportErrorKind::FileDoesNotExist => {

            },
            GetImportErrorKind::ReadImportError => {

            },
            GetImportErrorKind::ReadTsFileError => {
                if let Some(io_errors) = &self.io_errors {
                    for io_error in io_errors {
                        write!(f, "IO Error {}", io_error.to_string()).unwrap();
                    }
                }
            },
        }
        Ok(())
    }
}
