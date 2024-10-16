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
    canonical_root: PathBuf,
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
macro_rules! amap(
    { $($key:expr => $value:expr),+ } => {
        {
            let mut m = ahashmap::AHashMap::default();
            $(
                m.insert(String::from($key), $value);
            )+
            m
        }
    };
);

#[macro_export]
macro_rules! map2(
    { $($key:expr => $value:expr),+ } => {
        {
            let mut m = ::std::collections::HashMap::new();
            $(
                m.insert($key, $value);
            )+
            m
        }
    };
);

#[macro_export]
macro_rules! set(
    { $($item:expr ),+ } => {
        {
            let mut m = ::std::collections::HashSet::new();
            $(
                m.insert($item);
            )+
            m
        }
    };
);

#[macro_export]
macro_rules! aset(
    { $($item:expr ),+ } => {
        {
            let mut m = ahashmap::AHashSet::default();
            $(
                m.insert($item);
            )+
            m
        }
    };
);

#[macro_export]
macro_rules! test_tmpdir(
    { $($key:expr => $value:expr),+ } => {
        {
            use test_tmpdir::TmpDir;
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
        let root = tempfile::tempdir().unwrap();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        TmpDir {
            tmp_root: root,
            canonical_root,
        }
    }

    pub fn new_with_content(content: &HashMap<String, &str>) -> TmpDir {
        let out = Self::new();
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
        &self.canonical_root
    }

    pub fn root_join<S: AsRef<str>>(&self, other: S) -> PathBuf {
        self.canonical_root
            .to_owned()
            .join(PathBuf::from_slash(other))
    }
}
