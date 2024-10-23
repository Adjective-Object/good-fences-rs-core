use std::{
    fmt::{Display, Formatter},
    path::Path,
};

use ahashmap::AHashSet;
use serde::Deserialize;

#[derive(Debug, Eq, PartialEq)]
pub struct ErrList<E>(Vec<E>);

impl<E: Display> Display for ErrList<E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for err in &self.0 {
            writeln!(f, "{}", err)?;
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum GlobInterp {
    Name,
    Path,
}

impl Display for GlobInterp {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name => write!(f, "name"),
            Self::Path => write!(f, "path"),
        }
    }
}

#[derive(Debug)]
pub struct PatErr(usize, GlobInterp, glob::PatternError);

impl Display for PatErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "In {} pattern at idx {}: {:#}", self.1, self.0, self.2)
    }
}

impl PartialEq for PatErr {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
            && self.1 == other.1
            && self.2.pos == other.2.pos
            && self.2.msg == other.2.msg
    }
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("Error parsing package match rules: {0}")]
    InvalidGlobPatterns(ErrList<PatErr>),
}

/// A JSON serializable proxy for the UnusedFinderConfig struct
///
/// This struct is used to serialize the UnusedFinderConfig struct to JSON
/// with serde, or to recieve the config to JS via napi.
#[cfg_attr(feature = "napi", napi(object))]
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedFinderJSONConfig {
    /// Path to the root directory of the repository.
    #[serde(default)]
    pub repo_root: String,
    /// Root paths to walk as source files
    ///
    /// These can be either absolute paths, or paths relative to the repo root
    pub root_paths: Vec<String>,
    /// A List of globs.
    /// Matching files and directories won't be scanned during the file walk
    ///
    /// Matches are made against the names of the individual directories,
    /// NOT the full directory paths
    #[serde(default)]
    pub skip: Vec<String>,
    /// If true, individual exported symbols are also tracked
    #[serde(default)]
    pub report_exported_symbols: bool,
    /// List of packages that should be considered "entry" packages
    /// All transitive imports from the exposed exports of these packages
    /// will be considered used
    ///
    /// Items are parsed in one of three ways:
    /// 1. If the item starts with "./", it is treated as a path glob, and evaluated against the paths of package folders, relative to the repo root.
    /// 2. If the item contains any of "~)('!*", it is treated as a name-glob, and evaluated as a glob against the names of packages.
    /// 3. Otherwise, the item is treated as the name of an individual package, and matched literally.
    pub entry_packages: Vec<String>,
}

/// Configuration for the unused symbols finder
#[derive(Debug, Default, Clone)]
pub struct UnusedFinderConfig {
    /// If true, the finder should report exported symbols that are not used anywhere in the project
    pub report_exported_symbols: bool,

    /// Path to the root directory of the repository
    pub repo_root: String,

    /// Pats to walk as "internal" source files
    pub root_paths: Vec<String>,

    /// packages we should consider as "entry" packages
    pub entry_packages: PackageMatchRules,

    /// Globs of individual files & directories to skip during the file walk.
    ///
    /// Some internal directories are always skipped.
    /// See [crate::walk::DEFAULT_SKIPPED_DIRS] for more details.
    pub skip: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct PackageMatchRules {
    pub names: AHashSet<String>,
    pub name_patterns: Vec<glob::Pattern>,
    pub path_patterns: Vec<glob::Pattern>,
}

impl PackageMatchRules {
    /// Check if a package matches the rules
    ///
    /// * `package_path` - The path to the package, relative to the repo root
    /// * `package_name` - The name of the package
    pub fn matches(&self, package_path: &Path, pacakge_name: &str) -> bool {
        if self.names.contains(pacakge_name) {
            return true;
        }
        for pattern in &self.name_patterns {
            if pattern.matches(pacakge_name) {
                return true;
            }
        }
        for pattern in &self.path_patterns {
            // empty string (with no heap allocated data)
            //
            // This is used to avoid allocating a new string for the path
            // in the case that the path is already a string-safe OsString
            let mut path_string: String = String::new();
            let path_string_ref = package_path.to_str().unwrap_or_else(|| {
                path_string = package_path.to_string_lossy().into_owned();
                &path_string
            });
            if pattern.matches(path_string_ref) {
                return true;
            }
        }

        false
    }
}

impl<T: AsRef<str> + ToString> TryFrom<Vec<T>> for PackageMatchRules {
    type Error = ConfigError;
    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        let mut names = AHashSet::with_capacity_and_hasher(value.len(), Default::default());
        let mut name_patterns = Vec::new();
        let mut path_patterns = Vec::new();
        let mut errs: Vec<PatErr> = Vec::new();
        for (i, item) in value.into_iter().enumerate() {
            if item.as_ref().starts_with("./") {
                match glob::Pattern::new(item.as_ref()) {
                    Err(e) => errs.push(PatErr(i, GlobInterp::Path, e)),
                    Ok(r) => path_patterns.push(r),
                };
            } else if item.as_ref().chars().any(|c| "~)('!*".contains(c)) {
                match glob::Pattern::new(item.as_ref()) {
                    Err(e) => errs.push(PatErr(i, GlobInterp::Name, e)),
                    Ok(r) => name_patterns.push(r),
                };
            } else {
                names.insert(item.to_string());
            }
        }

        if !errs.is_empty() {
            return Err(ConfigError::InvalidGlobPatterns(ErrList(errs)));
        }

        Ok(Self {
            names,
            name_patterns,
            path_patterns,
        })
    }
}

impl TryFrom<UnusedFinderJSONConfig> for UnusedFinderConfig {
    type Error = ConfigError;
    fn try_from(value: UnusedFinderJSONConfig) -> std::result::Result<Self, Self::Error> {
        Ok(UnusedFinderConfig {
            // raw fields that are copied from the JSON config
            report_exported_symbols: value.report_exported_symbols,
            root_paths: value.root_paths,
            repo_root: value.repo_root,
            // other fields that are processed before use
            entry_packages: value.entry_packages.try_into()?,
            skip: value.skip,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_invalid_glob_path_err() {
        let json_config = r#"{
            "repoRoot": "/path/to/repo",
            "rootPaths": ["src"],
            "entryPackages": ["./***"],
            "skip": []
        }"#;

        let config: UnusedFinderJSONConfig = serde_json::from_str(json_config).unwrap();
        let err: ConfigError = UnusedFinderConfig::try_from(config).unwrap_err();
        let expected_err = ConfigError::InvalidGlobPatterns(ErrList(vec![PatErr(
            0,
            GlobInterp::Path,
            glob::PatternError {
                pos: 4,
                msg: "wildcards are either regular `*` or recursive `**`",
            },
        )]));

        assert_eq!(err, expected_err);
    }

    #[test]
    fn test_invalid_glob_name_err() {
        let json_config = r#"{
            "repoRoot": "/path/to/repo",
            "rootPaths": ["src"],
            "entryPackages": ["@foo/***"],
            "skip": []
        }"#;

        let config: UnusedFinderJSONConfig = serde_json::from_str(json_config).unwrap();
        let err: ConfigError = UnusedFinderConfig::try_from(config).unwrap_err();
        let expected_err = ConfigError::InvalidGlobPatterns(ErrList(vec![PatErr(
            0,
            GlobInterp::Name,
            glob::PatternError {
                pos: 7,
                msg: "wildcards are either regular `*` or recursive `**`",
            },
        )]));

        assert_eq!(err, expected_err);
    }

    #[test]
    fn test_invalid_glob_multi_err() {
        let json_config = r#"{
            "repoRoot": "/path/to/repo",
            "rootPaths": ["src"],
            "entryPackages": [
                "my-pkg1",
                "@foo/***",
                "my-pkg2-*",
                "./foo/***"
            ],
            "skip": []
        }"#;

        let config: UnusedFinderJSONConfig = serde_json::from_str(json_config).unwrap();
        let err: ConfigError = UnusedFinderConfig::try_from(config).unwrap_err();
        let expected_err = ConfigError::InvalidGlobPatterns(ErrList(vec![
            PatErr(
                1,
                GlobInterp::Name,
                glob::PatternError {
                    pos: 7,
                    msg: "wildcards are either regular `*` or recursive `**`",
                },
            ),
            PatErr(
                3,
                GlobInterp::Path,
                glob::PatternError {
                    pos: 8,
                    msg: "wildcards are either regular `*` or recursive `**`",
                },
            ),
        ]));

        assert_eq!(expected_err, err);
    }
}
