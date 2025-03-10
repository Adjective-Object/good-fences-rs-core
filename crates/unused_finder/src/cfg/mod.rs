use std::{
    fmt::{Display, Formatter},
    path::Path,
};

use glob_match::glob_match;
use package_match_rules::PackageMatchRules;
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
    pub test_files: Vec<String>,

    /// Globs of individual files & directories to skip during the file walk.
    ///
    /// Some internal directories are always skipped.
    /// See [crate::walk::DEFAULT_SKIPPED_DIRS] for more details.
    pub skip: Vec<String>,
}

impl UnusedFinderConfig {
    pub fn is_test_path(&self, path: &Path) -> bool {
        let mut local_string: String = "".to_string();
        self.is_test_path_str(path.to_str().unwrap_or_else(|| {
            local_string = path.to_string_lossy().into_owned();
            &local_string
        }))
    }
    pub fn is_test_path_str(&self, path: &str) -> bool {
        let relative = path.strip_prefix(&self.repo_root).unwrap_or(path);
        let relative = relative.strip_prefix('/').unwrap_or(relative);
        for test_glob in &self.test_files {
            println!("testing pattern {} against {}", test_glob, relative);
            if glob_match(test_glob, relative) {
                println!("  matched!");
                return true;
            }
        }
        false
    }
}

impl From<UnusedFinderJSONConfig> for UnusedFinderConfig {
    fn from(value: UnusedFinderJSONConfig) -> Self {
        UnusedFinderConfig {
            // raw fields that are copied from the JSON config
            report_exported_symbols: value.report_exported_symbols,
            allow_unused_types: value.allow_unused_types,
            root_paths: value.root_paths,
            repo_root: value.repo_root,
            // other fields that are processed before use
            entry_packages: value.entry_packages.into(),
            test_files: value.test_files,
            skip: value.skip,
        }
    }
}

#[cfg(test)]
mod test {
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
                "**/*Test{s,}.{ts,tsx}".to_string(),
                "**/*.stories.{ts,tsx}".to_string(),
                // allow all mocks
                "**/__mocks__/**".to_string(),
                // and jest configs
                "**/jest.config.js".to_string(),
            ],
        };

        let cases = vec![
            ("/workspaces/demo/packages/cool/cool-search-bar/src/test/CoolComponent.SomethingElse.Tests.tsx", true)
        ];

        let mut result: Vec<(&str, bool)> = vec![];
        for (path, _) in cases.iter() {
            let config = super::UnusedFinderConfig::from(config.clone());
            let actual = config.is_test_path_str(path);
            result.push((*path, actual));
        }

        pretty_assertions::assert_eq!(result, cases);
    }
}
