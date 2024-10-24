use path_clean::PathClean;
use std::path::{Path, PathBuf};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Base path is not absolute")]
    BasePathNotAbsolute,
}

pub fn join_abspath(base: impl AsRef<Path>, path: impl AsRef<Path>) -> Result<PathBuf, Error> {
    let base = base.as_ref();
    let path = path.as_ref();
    if !base.is_absolute() {
        return Err(Error::BasePathNotAbsolute);
    }

    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path).to_path_buf()
    }
    .clean();

    Ok(absolute_path)
}
