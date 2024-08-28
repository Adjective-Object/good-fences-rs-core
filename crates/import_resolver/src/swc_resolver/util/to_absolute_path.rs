use std::path::{Path, PathBuf};

use anyhow::Error;
use path_clean::PathClean;

pub fn to_absolute_path(path: &Path) -> Result<PathBuf, Error> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    }
    .clean();

    Ok(absolute_path)
}
