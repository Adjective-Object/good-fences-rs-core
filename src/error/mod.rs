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

#[derive(Debug)]
pub struct ResolvedImportNotFound {
    pub project_local_path_str: String,
    pub source_file_path: String,
    pub import_specifier: String,
}

#[derive(Debug)]
pub enum EvaluateFencesError {
    IgnoredDir(ResolvedImportNotFound), // In case the resolve_ts_import finds a file but it matches any ignoredDirs and is not in source_file_map
    NotScanned(ResolvedImportNotFound), // In case resolve_ts_import finds a file but is not in source_file_map and does not match any ignoredDirs (e.g. running good-fences only on packages and not in shared)
    ImportNotResolved {
        // In case the resolve_ts_import fails to find file
        import_specifier: String,
        source_file_path: String,
    },
}

impl Error for EvaluateFencesError {}

impl Display for EvaluateFencesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvaluateFencesError::IgnoredDir(err) => {
                write!(
                    f,
                    "Resolved import path {} for {} specifier at {} matched ignored dirs regex, excluded from file scanning",
                    err.project_local_path_str, err.import_specifier, err.source_file_path
                )
            }
            EvaluateFencesError::NotScanned(err) => {
                write!(
                    f,
                    "could not find project local path {} imported by {} with specifier {}",
                    err.project_local_path_str, err.source_file_path, err.import_specifier
                )
            }
            EvaluateFencesError::ImportNotResolved {
                import_specifier,
                source_file_path,
            } => {
                write!(
                    f,
                    "Unable to resolve import at with specifier {} at {}",
                    import_specifier, source_file_path
                )
            }
        }
    }
}

// equivalent to napi::Error, but declared separately so
// it can be used in tested modules
//
// Test modules can't reference napi::Error directly, since
// that would lead to a reference to `napi_delete_reference`,
// which only exists when the library is linked with node.
#[derive(Debug)]
pub struct NapiLikeError {
    pub status: napi::Status,
    pub message: String,
}

impl AsRef<str> for NapiLikeError {
    fn as_ref(&self) -> &str {
        &self.message
    }
}

impl Display for NapiLikeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Status: {}. {}", self.status, self.message)
    }
}
