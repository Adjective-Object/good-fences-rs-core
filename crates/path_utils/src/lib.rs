use anyhow::{Context, Result};
use path_slash::PathExt;
use relative_path::RelativePathBuf;
use std::path::{Path, PathBuf};

pub fn as_relative_slash_path<P: AsRef<Path>>(p: P) -> Result<RelativePathBuf> {
    let pref = p.as_ref();
    let relative_fence_path: RelativePathBuf =
        RelativePathBuf::from_path(pref).with_context(|| {
            let pref_str = pref.to_string_lossy();
            format!("failed to convert path to relative-path: \"{pref_str}\"")
        })?;
    let slashed_pbuf = PathBuf::from(relative_fence_path.as_str())
        .to_slash()
        .map(|s| s.to_string())
        .with_context(|| {
            let rel_fence_str = relative_fence_path.as_str();
            format!("failed to convert relative-path to a slashed path: \"{rel_fence_str}\"")
        })?;
    Ok(RelativePathBuf::from(slashed_pbuf))
}
