use std::fmt::{Display, Formatter};

use package_match_rules::PackageMatchRules;
use serde::Deserialize;

mod package_match_rules;

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
/// This struct is used to deserialize the UnusedFinderConfig struct
/// from a config file to with serde / over the debug bridge for napi
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
    /// If true, type-only exports will not be reported as used.
    /// However, the transitive dependencies of unused types will still be
    /// reported as unused.
    #[serde(default)]
    pub allow_unused_types: bool,
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
    /// If true, type-only exports will not be reported as used.
    /// However, the transitive dependencies of unused types will still be
    /// reported as unused.
    pub allow_unused_types: bool,

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

impl TryFrom<UnusedFinderJSONConfig> for UnusedFinderConfig {
    type Error = ConfigError;
    fn try_from(value: UnusedFinderJSONConfig) -> std::result::Result<Self, Self::Error> {
        Ok(UnusedFinderConfig {
            // raw fields that are copied from the JSON config
            report_exported_symbols: value.report_exported_symbols,
            allow_unused_types: value.allow_unused_types,
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
                pos: 2,
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
                    pos: 6,
                    msg: "wildcards are either regular `*` or recursive `**`",
                },
            ),
        ]));

        assert_eq!(expected_err, err);
    }
}
