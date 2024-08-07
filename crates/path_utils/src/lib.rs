use relative_path::RelativePathBuf;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use path_slash::PathExt;
pub trait ToSlashed {
    fn to_slashed(&self) -> anyhow::Result<PathBuf>;
}

impl ToSlashed for Path {
    fn to_slashed(&self) -> anyhow::Result<PathBuf> {
        let slashed: PathBuf;
        #[cfg(target_os = "windows")]
        {
            // required for Path.to_slash()
            use path_slash::PathExt;
            slashed = match p.to_slash() {
                Some(path) => PathBuf::from(path.to_string().as_str()),
                None => return Err(WalkDirsError::SlashError(p.to_string_lossy().to_string())),
            };
        }
        #[cfg(not(target_os = "windows"))]
        {
            slashed = PathBuf::from(self)
        }
        return Ok(slashed);
        }
}

pub trait FromSlashed {
    fn from_slashed(p: &str) -> Self;
}

impl FromSlashed for PathBuf {
    fn from_slashed(p: &str) -> PathBuf {
        let slashed: PathBuf;
        #[cfg(target_os = "windows")]
        {
            // required for PathBufExt.to_slash()
            use path_slash::PathBufExt;
            slashed = PathBuf::from_slash(p);
        }
        #[cfg(not(target_os = "windows"))]
        {
            slashed = PathBuf::from(p)
        }
        return slashed;
    }
}


fn as_relative_slash_path<P: AsRef<Path>>(
    p: P,
) -> Result<RelativePathBuf> {
    let pref = p.as_ref();
    let relative_fence_path: RelativePathBuf = RelativePathBuf::from_path(pref)
        .with_context(|| {
            let pref_str = pref.to_string_lossy();  
            format!("failed to convert path to relative-path: \"{pref_str}\"")
        })?;
    let slashed_pbuf = PathBuf::from(relative_fence_path.as_str()).to_slash().map(|s| s.to_string())
    .with_context(|| {
        let rel_fence_str = relative_fence_path.as_str();
        format!("failed to convert relative-path to a slashed path: \"{rel_fence_str}\"")
    })?;
    Ok(RelativePathBuf::from(slashed_pbuf))
}