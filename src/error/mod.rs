use std::{error::Error, fmt::Display, path::PathBuf};

#[derive(Debug)]
pub enum GetImportError {
    ParseTsFileError {
        filepath: String,
        parser_errors: Vec<String>,
    },
    FileDoesNotExist {
        filepath: PathBuf,
    },
    ReadImportError {
        io_errors: Vec<std::io::Error>,
    },
    PathError {
        filepath: PathBuf,
    },
}

impl Error for GetImportError {}

impl Display for GetImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GetImportError::ParseTsFileError {
                filepath,
                parser_errors,
            } => {
                write!(f, "Error parsing {} :\n {:?}", filepath, parser_errors)
            }
            GetImportError::FileDoesNotExist { filepath } => {
                write!(f, "Could not read file {:?}", filepath)
            }
            GetImportError::ReadImportError { io_errors } => write!(f, "{:?}", io_errors),
            GetImportError::PathError { filepath } => {
                write!(f, "Error reading {:?} path", filepath)
            }
        }
    }
}
