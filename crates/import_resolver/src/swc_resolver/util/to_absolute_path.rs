pub fn to_absolute_path(path: &Path) -> Result<PathBuf, Error> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir()?.join(path)
    }
    .clean();

    Ok(absolute_path)
}
