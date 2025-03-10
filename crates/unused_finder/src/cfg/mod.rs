use core::fmt;
use std::{
    fmt::{Debug, Display, Formatter},
    path::Path,
};

use globset::GlobMatcher;
use itertools::Itertools;
use package_match_rules::PackageMatchRules;
use rayon::iter::Either;
use schemars::JsonSchema;
use serde::Deserialize;

pub mod package_match_rules;

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
pub struct PatErr(usize, GlobInterp, globset::Error);

impl Display for PatErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "In {} pattern at idx {}: {:#}", self.1, self.0, self.2)
    }
}

impl PartialEq for PatErr {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
            && self.1 == other.1
            && self.2.glob() == other.2.glob()
            && self.2.kind() == other.2.kind()
    }
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ConfigError {
    #[error("Error parsing package match rules: {0}")]
    InvalidPackageMatchGlob(ErrList<PatErr>),
    #[error("Error parsing testFile glob(s): {0}")]
    InvalidTestsGlob(ErrList<PatErr>),
}

/// A JSON serializable proxy for the UnusedFinderConfig struct
///
/// This struct is used to deserialize the UnusedFinderConfig struct
/// from a config file to with serde / over the debug bridge for napi
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
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
    /// Note that the only files that are considered roots are the ones
    /// that are _explicitly exported_, either as an entry in the package's "exports" config,
    /// or as a main/module export
    ///
    /// Items are parsed in one of three ways:
    /// 1. If the item starts with "./", it is treated as a path glob, and evaluated
    ///    against the paths of package folders, relative to the repo root.
    /// 2. If the item contains any of "~)('!*", it is treated as a name-glob, and evaluated
    ///    as a glob against the names of packages.
    /// 3. Otherwise, the item is treated as the name of an individual package, and matched
    ///    literally.
    pub entry_packages: Vec<String>,
    /// List of glob patterns to mark as "tests".
    /// These files will be marked as used, and all of their transitive
    /// dependencies will also be marked as used
    ///
    /// glob patterns are matched against the relative file path from the
    /// root of the repository
    #[serde(default)]
    pub test_files: Vec<String>,
}

#[derive(Default, Clone)]
pub struct GlobGroup {
    pub globset: globset::GlobSet,
    // keep these around for debugging
    pub globs: Vec<globset::Glob>,
}

impl Debug for GlobGroup {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.globs.iter().map(|m| m.glob()).join(", "))
    }
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

    /// List of globs that will be matched against files in the repository
    ///
    /// Matches are made against the relative file paths from the repo root.
    /// A matching file will be tagged as a "test" file, and will be excluded
    /// from the list of unused files
    pub test_files: GlobGroup,

    /// Globs of individual files & directories to skip during the file walk.
    ///
    /// Some internal directories are always skipped.
    /// See [crate::walk::DEFAULT_SKIPPED_DIRS] for more details.
    pub skip: Vec<String>,
}

impl UnusedFinderConfig {
    pub fn is_test_path(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(&self.repo_root).unwrap_or(path);
        let relative = relative.strip_prefix("/").unwrap_or(relative);
        self.test_files.globset.is_match(relative)
    }

    pub fn is_test_path_str(&self, path: &str) -> bool {
        let relative = path.strip_prefix(&self.repo_root).unwrap_or(path);
        let relative = relative.strip_prefix('/').unwrap_or(relative);
        self.test_files.globset.is_match(relative)
    }
}

impl TryFrom<UnusedFinderJSONConfig> for UnusedFinderConfig {
    type Error = ConfigError;
    fn try_from(value: UnusedFinderJSONConfig) -> std::result::Result<Self, Self::Error> {
        let (test_globs, test_glob_errs): (Vec<globset::Glob>, Vec<_>) =
            value.test_files.iter().partition_map(|pat| {
                println!("compile glob {}", pat);
                match globset::Glob::new(pat) {
                    Ok(pat) => Either::Left(pat),
                    Err(err) => Either::Right(PatErr(0, GlobInterp::Path, err)),
                }
            });
        if !test_glob_errs.is_empty() {
            return Err(ConfigError::InvalidTestsGlob(ErrList(test_glob_errs)));
        }

        let mut set_builder = globset::GlobSetBuilder::new();
        for glob in test_globs.iter() {
            set_builder.add(glob.clone());
        }
        let globset = set_builder.build().map_err(|err| {
            ConfigError::InvalidTestsGlob(ErrList(vec![PatErr(0, GlobInterp::Path, err)]))
        })?;

        Ok(UnusedFinderConfig {
            // raw fields that are copied from the JSON config
            report_exported_symbols: value.report_exported_symbols,
            allow_unused_types: value.allow_unused_types,
            root_paths: value.root_paths,
            repo_root: value.repo_root,
            // other fields that are processed before use
            entry_packages: value.entry_packages.try_into()?,
            test_files: GlobGroup {
                globset,
                globs: test_globs,
            },
            skip: value.skip,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_invalid_glob_unclosed_alternate() {
        let json_config = r#"{
            "repoRoot": "/path/to/repo",
            "rootPaths": ["src"],
            "entryPackages": [],
            "testFiles": ["@foo/{cb,"],
            "skip": []
        }"#;

        let config: UnusedFinderJSONConfig = serde_json::from_str(json_config).unwrap();
        let err: ConfigError = UnusedFinderConfig::try_from(config).unwrap_err();
        assert_eq!(format!("{}", err), "Error parsing testFile glob(s): In path pattern at idx 0: error parsing glob '@foo/{cb,': unclosed alternate group; missing '}' (maybe escape '{' with '[{]'?)\n");
    }

    #[test]
    fn test_invalid_glob_nested_alternate() {
        let json_config = r#"{
            "repoRoot": "/path/to/repo",
            "rootPaths": ["src"],
            "entryPackages": ["@foo/{a,{cb, d}}"],
            "skip": []
        }"#;

        let config: UnusedFinderJSONConfig = serde_json::from_str(json_config).unwrap();
        let err: ConfigError = UnusedFinderConfig::try_from(config).unwrap_err();
        assert_eq!(format!("{}", err), "Error parsing package match rules: In name pattern at idx 0: error parsing glob '@foo/{a,{cb, d}}': nested alternate groups are not allowed\n");
    }

    #[test]
    fn test_invalid_glob_unclosed_charclass() {
        let json_config = r#"{
            "repoRoot": "/path/to/repo",
            "rootPaths": ["src"],
            "entryPackages": ["@foo/[a"],
            "testFiles": [
                "@foo/[a"
            ],
            "skip": []
        }"#;

        let config: UnusedFinderJSONConfig = serde_json::from_str(json_config).unwrap();
        let err: ConfigError = UnusedFinderConfig::try_from(config).unwrap_err();
        assert_eq!(format!("{}", err), "Error parsing testFile glob(s): In path pattern at idx 0: error parsing glob '@foo/[a': unclosed character class; missing ']'\n");
    }

    #[test]
    fn test_glob_match_test() {
        let config = super::UnusedFinderJSONConfig {
            repo_root: "/workspaces/demo".to_string(),
            root_paths: vec!["src".to_string()],
            skip: vec!["target".to_string()],
            report_exported_symbols: true,
            allow_unused_types: false,
            entry_packages: vec!["@myorg/*".to_string()],
            test_files: vec![
                // just test this pattern
                "**/*{Test,Tests}.{ts,tsx}".to_string(),
            ],
        };

        let cases = vec![
            ("/workspaces/demo/packages/cool/cool-search-bar/src/test/CoolComponent.SomethingElse.Tests.tsx", true),
            ("/workspaces/demo/packages/cool-common/forms/cool-forms-view-view/src/test/utils/infoBarUtilsTests/someTests.ts", true),
        ];

        let mut result: Vec<(&str, bool)> = vec![];
        for (path, _) in cases.iter() {
            let config = super::UnusedFinderConfig::try_from(config.clone()).unwrap();
            let actual = config.is_test_path_str(path);
            result.push((*path, actual));
        }

        pretty_assertions::assert_eq!(result, cases);
    }
}
