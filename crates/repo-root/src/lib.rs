use std::path::{Path, PathBuf};

pub fn find_git_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap();
    find_git_root_from(&cwd)
}

pub fn find_git_root_from(search_start: &Path) -> PathBuf {
    let mut path = PathBuf::from(search_start);
    loop {
        if path.join(".git").exists() {
            break;
        }
        if !path.pop() {
            break;
        }
    }
    path
}
