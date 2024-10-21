use std::{
    fmt::Debug,
    io::{BufRead, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use path_slash::PathBufExt;

#[derive(PartialEq)]
pub struct IgnoreFile {
    pub path: PathBuf,
    pub patterns: Vec<IgnorePattern>,
}

impl Debug for IgnoreFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IgnoreFile")
            .field("path", &self.path)
            .field(
                "patterns",
                &self
                    .patterns
                    .iter()
                    .map(|p| p.pattern.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl IgnoreFile {
    pub fn read(path: PathBuf) -> Result<Self> {
        let f = std::fs::File::open(&path)
            .with_context(|| format!("Failed to open ignore file at path: {}", path.display()))?;
        Self::from_reader(path, f)
    }

    pub fn from_reader(path: PathBuf, r: impl Read) -> Result<Self> {
        // read as lines
        let lines = std::io::BufReader::new(r).lines();
        let mut patterns = Vec::new();
        for line in lines {
            let line = line.unwrap();
            if let Some(pattern) = IgnorePattern::from_line(&line)? {
                patterns.push(pattern);
            }
        }

        Ok(Self {
            path: path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| path),
            patterns,
        })
    }

    pub fn is_ignored(&self, path: &Path) -> bool {
        let relative_path = match pathdiff::diff_paths(path, &self.path) {
            Some(p) => p,
            None => return false,
        };
        let relative_slash = match relative_path.to_slash() {
            Some(p) => p,
            None => return false,
        };

        let mut ignored = false;
        for pattern in self.patterns.iter() {
            if pattern.pattern.matches(relative_slash.as_ref()) {
                ignored = !pattern.negated;
            }
        }

        ignored
    }
}

#[derive(Debug, PartialEq)]
pub struct IgnorePattern {
    pub pattern: glob::Pattern,
    pub negated: bool,
}

impl IgnorePattern {
    pub fn from_line(line: &str) -> Result<Option<Self>, anyhow::Error> {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            return Ok(None);
        }
        match trimmed_line.bytes().next().map(|b| b as char) {
            None | Some('#') => Ok(None),
            Some('!') => Ok(Some(IgnorePattern {
                pattern: Self::glob_line(&trimmed_line[1..])?,
                negated: true,
            })),
            _ => Ok(Some(IgnorePattern {
                pattern: Self::glob_line(trimmed_line)?,
                negated: false,
            })),
        }
    }

    fn glob_line(line: &str) -> Result<glob::Pattern, anyhow::Error> {
        if line.ends_with('/') {
            // support trailing slashes for recursibe globs
            Ok(glob::Pattern::new(&format!("{}**", line))
                .with_context(|| format!("Failed to parse glob pattern: {}", line))?)
        } else {
            glob::Pattern::new(line)
                .with_context(|| format!("Failed to parse glob pattern: {}", line))
        }
    }
}

#[cfg(test)]
mod test {
    use super::IgnoreFile;
    use path_slash::PathBufExt;
    use std::path::PathBuf;

    fn ignore_file(path: &str, content: &str) -> IgnoreFile {
        IgnoreFile::from_reader(PathBuf::from(path), std::io::Cursor::new(content)).unwrap()
    }

    #[test]
    fn test_parse() {
        let ignore_file = ignore_file(
            "path/to/.unusedignore",
            r#"
                    foo/file.js
                    # check that we are trimming whitespace at the tail
                    !bar/baz.js   "#,
        );

        // check that the patterns are correctly parsed
        assert_eq!(ignore_file.patterns.len(), 2);
        assert_eq!(ignore_file.patterns[0].pattern.as_str(), "foo/file.js");
        assert_eq!(ignore_file.patterns[0].negated, false);
        assert_eq!(ignore_file.patterns[1].pattern.as_str(), "bar/baz.js");
        assert_eq!(ignore_file.patterns[1].negated, true);
    }

    #[test]
    fn test_folder_pattern() {
        let ignore_file = ignore_file(
            "path/to/.unusedignore",
            r#"
                    foo/
                    "#,
        );

        // check behaviour
        assert!(ignore_file.is_ignored(&PathBuf::from_slash("path/to/foo/file.js")));
    }

    #[test]
    fn test_recursive_wildcards() {
        let ignore_file = ignore_file(
            "path/to/.unusedignore",
            r#"
                    foo/**
                    "#,
        );

        // check behaviour
        assert!(ignore_file.is_ignored(&PathBuf::from_slash("path/to/foo/deep/nested/file.js")));
    }

    #[test]
    fn test_nomatch_non_specified() {
        let ignore_file = ignore_file(
            "path/to/.unusedignore",
            r#"
                    foo.ts
                    "#,
        );

        // check behaviour
        assert!(!ignore_file.is_ignored(&PathBuf::from_slash("path/to/bar.ts")));
    }

    #[test]
    fn test_positive_pattern() {
        let ignore_file = ignore_file(
            "path/to/.unusedignore",
            r#"
                    foo/file.js
                    bar/*
                    "#,
        );

        // check behaviour
        assert!(ignore_file.is_ignored(&PathBuf::from_slash("path/to/foo/file.js")));
        assert!(ignore_file.is_ignored(&PathBuf::from_slash("path/to/bar/anything.js")));
    }

    #[test]
    fn test_negative_pattern() {
        let ignore_file = ignore_file(
            "path/to/.unusedignore",
            r#"
                    bar/*
                    !bar/*.nomatch
                    "#,
        );

        // check behaviour
        assert!(ignore_file.is_ignored(&PathBuf::from_slash("path/to/bar/anything.js")));
        assert!(!ignore_file.is_ignored(&PathBuf::from_slash("path/to/bar/anything.nomatch")));
    }
}
