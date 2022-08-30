use std::path::{Path, PathBuf};

use path_slash::PathExt as _;
use relative_path::RelativePath;

use crate::error::WalkDirsError;

pub fn get_slashed_path_buf<'a>(p: &'a Path) -> Result<PathBuf, WalkDirsError> {
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

pub fn slashed_as_relative_path<'a>(
    slashed: &'a PathBuf,
) -> Result<&'a RelativePath, WalkDirsError> {
    match RelativePath::from_path(slashed.as_path()) {
        Ok(rel_path) => return Ok(rel_path),
        Err(e) => {
            return Err(WalkDirsError::RelativePathError {
                path: slashed.to_path_buf(),
                err: e,
            })
        }
    }
}
