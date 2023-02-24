use crate::error::WalkDirsError;
use relative_path::RelativePathBuf;
use std::path::{Path, PathBuf};

pub fn get_slashed_path_buf(p: &Path) -> Result<PathBuf, WalkDirsError> {
    let slashed: PathBuf;
    #[cfg(target_os = "windows")]
    {
        slashed = match p.to_slash() {
            Some(path) => PathBuf::from(path.to_string().as_str()),
            None => return Err(WalkDirsError::SlashError(p.to_string_lossy().to_string())),
        };
    }
    #[cfg(not(target_os = "windows"))]
    {
        slashed = PathBuf::from(p)
    }
    return Ok(slashed);
}

pub fn slashed_as_relative_path(slashed: &PathBuf) -> Result<RelativePathBuf, WalkDirsError> {
    match RelativePathBuf::from_path(slashed) {
        Ok(rel_path) => Ok(rel_path),
        Err(e) => Err(WalkDirsError::RelativePathError {
            path: slashed.to_path_buf(),
            err: e,
        }),
    }
}

pub fn as_slashed_pathbuf(p: &str) -> PathBuf {
    let slashed: PathBuf;
    #[cfg(target_os = "windows")]
    {
        slashed = PathBuf::from_slash(p);
    }
    #[cfg(not(target_os = "windows"))]
    {
        slashed = PathBuf::from(p)
    }
    return slashed;
}
