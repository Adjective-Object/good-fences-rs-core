extern crate path_slash;

use path_slash::PathBufExt;
use std::{
    collections::HashMap,
    fs::File,
    io::{Error, Write},
    path::{Path, PathBuf},
};

pub struct TmpDir {
    tmp_root: tempfile::TempDir,
}

#[macro_export]
macro_rules! map(
    { $($key:expr => $value:expr),+ } => {
        {
            let mut m = ::std::collections::HashMap::new();
            $(
                m.insert(String::from($key), $value);
            )+
            m
        }
    };
);

#[macro_export]
macro_rules! test_tmpdir(
    { $($key:expr => $value:expr),+ } => {
        {
            let mut m = ::std::collections::HashMap::new();
            $(
                m.insert(String::from($key), $value);
            )+

            TmpDir::new_with_content(&m)
        }
    };
);

impl Default for TmpDir {
    fn default() -> Self {
        Self::new()
    }
}

impl TmpDir {
    pub fn new() -> TmpDir {
        TmpDir {
            tmp_root: tempfile::tempdir().unwrap(),
        }
    }

    pub fn new_with_content(content: &HashMap<String, &str>) -> TmpDir {
        let root = tempfile::tempdir().unwrap();
        let out = TmpDir { tmp_root: root };
        out.write_batch(content).unwrap();
        out
    }

    pub fn write_batch(&self, content: &HashMap<String, &str>) -> Result<(), Error> {
        for (path, content) in content {
            // mkdir -p
            std::fs::create_dir_all(self.tmp_root.path().join(path).parent().unwrap())?;
            // write the actual file
            let mut file = File::create(self.tmp_root.path().join(path))?;
            file.write_all(content.as_bytes())?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        self.tmp_root.path()
    }

    pub fn root_join<S: AsRef<str>>(&self, other: S) -> PathBuf {
        self.tmp_root
            .path()
            .to_owned()
            .join(PathBuf::from_slash(other))
    }
}
