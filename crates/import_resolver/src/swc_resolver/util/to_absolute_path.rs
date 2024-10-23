use std::path::{Path, PathBuf};

use anyhow::Error;
use path_clean::PathClean;

pub fn to_absolute_path(base: &Path, path: &Path) -> Result<PathBuf, Error> {
    if !base.is_absolute() {
        return Err(Error::msg("Base path must be absolute"));
    }

    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path).to_path_buf()
    }
    .clean();

    Ok(absolute_path)
}
