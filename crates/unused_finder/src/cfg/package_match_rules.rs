use std::path::Path;

use ahashmap::AHashSet;

use super::{ConfigError, GlobInterp, PatErr};

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
    pub fn matches(&self, package_path: &Path, package_name: &str) -> bool {
        if self.names.contains(package_name) {
            return true;
        }
        for pattern in &self.name_patterns {
            if pattern.matches(package_name) {
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

impl PackageMatchRules {
    pub fn empty() -> Self {
        Self::default()
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
            if let Some(trimmed) = item.as_ref().strip_prefix("./") {
                match glob::Pattern::new(trimmed) {
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
            return Err(ConfigError::InvalidGlobPatterns(super::ErrList(errs)));
        }

        Ok(Self {
            names,
            name_patterns,
            path_patterns,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_package_match_path() {
        let package_match_rules_strs = vec!["./shared/**"];
        let package_match_rules = PackageMatchRules::try_from(package_match_rules_strs).unwrap();
        assert!(
            package_match_rules.matches(Path::new("shared/n/my-pkg/package.json"), "@me/my-pkg")
        );
    }
}
