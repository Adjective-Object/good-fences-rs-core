use std::{error::Error, fmt::Display, path::PathBuf};

use relative_path::FromPathError;

#[derive(Debug)]
pub enum GetImportError {
    ParseTsFileError {
        filepath: String,
        parser_errors: Vec<String>,
    },
    FileDoesNotExist {
        filepath: String,
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
                write!(
                    f,
                    "Error parsing {} :\n {}",
                    filepath,
                    parser_errors.join("\n")
                )
            }
            GetImportError::FileDoesNotExist {
                filepath,
                io_errors,
            } => write!(
                f,
                "IO Errors found while trying to parse {} : {:?}",
                filepath, io_errors
            ),
            GetImportError::PathError { filepath } => {
                write!(f, "Error reading {:?} path", filepath)
            }
        }
    }
}

#[derive(Debug)]
pub enum WalkDirsError {
    SlashError(String),
    RelativePathError { path: PathBuf, err: FromPathError },
}

impl Error for WalkDirsError {}

impl Display for WalkDirsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalkDirsError::SlashError(slash_error) => {
                write!(f, "Error creating agnostic slash path {}", slash_error)
            }
            WalkDirsError::RelativePathError { path, err } => write!(
                f,
                "Error converting relative path {:?} to local slashed path: {:?}",
                path, err
            ),
        }
    }
}

#[derive(Debug)]
pub enum OpenTsConfigError {
    SerdeError(serde_json::Error),
    IOError(std::io::Error),
}

impl Error for OpenTsConfigError {}

impl Display for OpenTsConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenTsConfigError::SerdeError(err) => {
                write!(f, "Error parsing specified tsconfig file {}", err)
            }
            OpenTsConfigError::IOError(err) => {
                write!(f, "Error opening specified tsconfig file {}", err)
            }
        }
    }
}
