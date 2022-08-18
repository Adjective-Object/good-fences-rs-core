use std::{error::Error, fmt::Display};

#[derive(Debug, PartialEq)]
pub enum GetImportErrorKind {
    ParseTsFileError,
    FileDoesNotExist,
    ReadImportError,
    ReadTsFileError,
}

#[derive(Debug)]
pub struct GetImportError {
    pub kind: GetImportErrorKind,
    pub parser_errors: Option<Vec<String>>,
    pub io_errors: Option<Vec<std::io::Error>>,
    pub file_path: Option<String>,
}

impl GetImportError {
    pub fn new(
        kind: GetImportErrorKind,
        file_path: Option<String>,
        parser_errors: Option<Vec<String>>,
        io_errors: Option<Vec<std::io::Error>>,
    ) -> GetImportError {
        Self {
            parser_errors,
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
            GetImportErrorKind::ParseTsFileError => {}
            GetImportErrorKind::FileDoesNotExist => {}
            GetImportErrorKind::ReadImportError => {}
            GetImportErrorKind::ReadTsFileError => {
                if let Some(io_errors) = &self.io_errors {
                    for e in io_errors {
                        write!(f, "IO Error {}", e.to_string()).unwrap();
                    }
                }
                if let Some(parser_errrors) = &self.parser_errors {
                    for err in parser_errrors {
                        write!(f, "Parser error: {}", err);
                    }
                }
            }
        }
        Ok(())
    }
}
