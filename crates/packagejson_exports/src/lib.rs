/// Implements the "exports" map for a package.json file.
use core::fmt;
use std::{fmt::Display, path::PathBuf, str::FromStr};

use anyhow::{anyhow, Result};
use path_clean::PathClean;
use path_slash::PathBufExt;

use packagejson::{
    exported_path::{ExportedPath, ExportedPathRef},
    PackageJsonExport, PackageJsonExports,
};

// Struct that performs the "export" field remapping from package.json
//
// This holds derived data from the "exports" field in a package.json file, and
// is computed with PackageExportRewriteData::try_from(PackageJson)
#[derive(Debug, Default, Clone, PartialEq)]
pub struct PackageExportRewriteData {
    // Subpath keys select an ordered list of conditions, never the reverse.
    exports: hashbrown::HashMap<String, Vec<(String, ExportedPath)>>,
}

fn clean_path(p: &str) -> String {
    let mut store_str = String::new();
    String::from(clean_path_avoid_alloc(p, &mut store_str))
}

// Cleans a path, removing any unnecessary characters and normalizing it
//
// If the path is already clean, this will return the original path
fn clean_path_avoid_alloc<'a>(
    // the original path to clean
    original: &'a str,
    // if the path is not clean, this will be used to store the cleaned path
    //
    // If the path is cleaned, this will be unused
    store: &'a mut String,
) -> &'a str {
    let mut o: &'a str = original;
    if o.starts_with("./") {
        o = &original[2..]
    } else if o == "." {
        return o;
    }

    let bytes = o.as_bytes();
    for (i, c) in o.chars().enumerate() {
        // if we encounter anything that could be a character in an unclean
        // path, just fall back to path.Clean

        let is_complex_path = match c {
            // escaped chars
            '\\' => true,
            // possible part of '/.' or '..'
            '.' => i > 0 && (bytes[i - 1] == b'.' || bytes[i - 1] == b'/'),
            // consecutive slashes or './'
            '/' => i > 0 && (bytes[i - 1] == b'.' || bytes[i - 1] == b'/'),
            _ => false,
        };

        if is_complex_path {
            store.clear();
            store.push_str("./");
            store.push_str(&PathBuf::from_str(o).unwrap().clean().to_slash().unwrap());
            return store;
        }
    }

    original
}

fn match_star_pattern<'a>(
    // The exports map to search against
    star_pattern: &str,
    // The import specifier to match. Must already be cleaned!C
    relative_import_specifier: &'a str,
) -> Option<&'a str> {
    let (prefix, star_suffix) = star_pattern.split_once('*')?;

    // This only handles a single "*", and not any more complex * patterns
    if let Some(remainder) = relative_import_specifier.strip_prefix(prefix) {
        // the pattern ends with the first star, so we don't need to do a suffix match.
        // The star match is the remainder of the string.
        if star_suffix.is_empty() {
            return Some(remainder);
            // check the suffix of the pattern _after_ the star matches the tail of the import
        } else if let Some(star_match) = remainder.strip_suffix(star_suffix) {
            return Some(star_match);
        }
    }

    None
}

pub enum ExportCondition<'a> {
    Default,
    Condition(&'a str),
}

impl<'a> From<&'a str> for ExportCondition<'a> {
    fn from(s: &'a str) -> Self {
        if s == "default" {
            return ExportCondition::Default;
        }

        ExportCondition::Condition(s)
    }
}

impl Display for ExportCondition<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportCondition::Default => write!(f, "default"),
            ExportCondition::Condition(cond) => write!(f, "{}", cond),
        }
    }
}

fn rewrite_star_export(
    // the export pattern that matched the specifier
    star_match: &str,
    // output path that was matched against by the export condition
    resolved_to: &str,
    out: &mut String,
) {
    out.clear();
    for c in resolved_to.chars() {
        if c == '*' {
            out.push_str(star_match);
        } else {
            out.push(c);
        }
    }
}

// remaps a matched node 14.x style deprecated directory export pattern
fn rewrite_dir_export(
    // the relative import specifier, cleaned
    clean_relative_import: &str,
    // the export pattern that matched the specifier
    directory_pattern: &str,
    // output path that was matched against by the export condition
    resolved_to: &str,
    // output string to write the resolved path to
    out: &mut String,
) {
    out.clear();
    out.push_str(resolved_to);
    out.push_str(&clean_relative_import[directory_pattern.len()..]);
}

pub struct MatchedExport<'a> {
    // the rewritten path, if any. If None, then this path is not exported.
    pub rewritten_export: ExportedPathRef<'a>,
    // export condition that was matched against, if any.
    //
    // Borrowed from the selected package export's condition list.
    pub export_kind: ExportCondition<'a>,
}

impl<'a> MatchedExport<'a> {
    fn with_kind(rewritten_export: ExportedPathRef<'a>, export_kind: ExportCondition<'a>) -> Self {
        Self {
            rewritten_export,
            export_kind,
        }
    }
}

fn reverse_match_star_pattern(star_pattern: &str, path: &str) -> bool {
    let pattern_segments = star_pattern.split('*').collect::<Vec<&str>>();
    let mut head: &str = path;
    for segment in pattern_segments {
        if segment.is_empty() {
            continue;
        }

        if let Some(next_idx) = head.find(segment) {
            head = &head[next_idx + segment.len()..];
        } else {
            return false;
        }
    }
    true
}

fn matches_exported_path(pattern: &str, target: &ExportedPath, path: &str) -> bool {
    let ExportedPath::Exported(target) = target else {
        return false;
    };
    if pattern.contains('*') {
        reverse_match_star_pattern(target, path)
    } else if pattern.ends_with('/') {
        path.starts_with(target)
    } else {
        path == target
    }
}

impl PackageExportRewriteData {
    // Rewrites an import path to a new path using only the "exports" field from package.json
    pub fn rewrite_relative_export<'a, TStr: Into<&'a str>>(
        &'a self,
        relative_import: &'a str,
        requested_export_conditions: impl Clone + IntoIterator<Item = TStr>,
        // this "out" parameter allows us to avoid allocating & copying a new string if we are resolving
        // to a non-pattern path. Since  we expect to be the more common case, we accept the awkwardness
        // of the out parameter to avoid the allocation.
        out: &'a mut String,
    ) -> Result<Option<MatchedExport<'a>>> {
        // destination String which may be used to store a mutated cleaned path, if the input path is not already clean
        let mut clean_dest = String::new();
        let clean_relative_import = clean_path_avoid_alloc(relative_import, &mut clean_dest);
        if std::path::Path::new(clean_relative_import)
            .components()
            .any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(anyhow!(
                "package subpath escapes package root: {relative_import}"
            ));
        }

        let selected = self
            .exports
            .get_key_value(clean_relative_import)
            .filter(|(pattern, _)| !pattern.contains('*') && !pattern.ends_with('/'))
            .or_else(|| {
                self.exports
                    .iter()
                    .filter(|(pattern, _)| {
                        pattern.ends_with('/')
                            && clean_relative_import.starts_with(pattern.as_str())
                    })
                    .max_by_key(|(pattern, _)| pattern.len())
            })
            .or_else(|| {
                self.exports
                    .iter()
                    .filter(|(pattern, _)| {
                        match_star_pattern(pattern, clean_relative_import).is_some()
                    })
                    .max_by_key(|(pattern, _)| (pattern.find('*').unwrap(), pattern.len()))
            });
        let Some((pattern, conditions)) = selected else {
            return Ok(None);
        };
        let Some((condition, target)) = conditions.iter().find(|(condition, _)| {
            condition == "default"
                || requested_export_conditions
                    .clone()
                    .into_iter()
                    .any(|active| active.into() == condition)
        }) else {
            return Ok(None);
        };
        let rewritten = target.as_ref().map_export(|target| {
            if let Some(matched) = match_star_pattern(pattern, clean_relative_import) {
                rewrite_star_export(matched, target, out);
                out as &str
            } else if pattern.ends_with('/') {
                rewrite_dir_export(clean_relative_import, pattern, target, out);
                out as &str
            } else {
                target
            }
        });
        Ok(Some(MatchedExport::with_kind(
            rewritten,
            condition.as_str().into(),
        )))
    }

    /// Checks if a given package-relative path is exported or not.
    /// Returns the set of conditions that export that path.
    pub fn get_exported_conditions<'a>(&'a self, package_relative_path: &str) -> Vec<&'a str> {
        let mut clean_dest = String::new();
        let cleaned_path = clean_path_avoid_alloc(package_relative_path, &mut clean_dest);

        let mut accum: Vec<&str> = Vec::new();

        for (pattern, conditions) in &self.exports {
            for (condition, target) in conditions {
                if matches_exported_path(pattern, target, cleaned_path)
                    && !accum.contains(&condition.as_str())
                {
                    accum.push(condition.as_str());
                }
            }
        }

        accum
    }

    /// Checks if a given package-relative path is exported or not.
    /// Returns the set of conditions that export that path.
    pub fn is_exported(&self, package_relative_path: &str) -> bool {
        let mut clean_dest = String::new();
        let cleaned_path = clean_path_avoid_alloc(package_relative_path, &mut clean_dest);

        self.exports.iter().any(|(pattern, conditions)| {
            conditions
                .iter()
                .any(|(_, target)| matches_exported_path(pattern, target, cleaned_path))
        })
    }
}

impl TryFrom<&PackageJsonExports> for PackageExportRewriteData {
    type Error = anyhow::Error;

    fn try_from(exports_map: &PackageJsonExports) -> Result<Self> {
        let mut resolution_data = PackageExportRewriteData::default();
        for (export_path, exported) in exports_map.iter() {
            if !export_path.starts_with("./") && export_path != "." {
                return Err(anyhow!(
                    "package.json exports fields must either be '.' or start with './'"
                ));
            }

            let export_path_star_ct = export_path.chars().filter(|c| *c == '*').count();
            if export_path_star_ct > 1 {
                return Err(anyhow!(
                    "Invalid star pattern '{}' in package.json exports field: \
                            Star patterns may contain at most a single star match.",
                    export_path
                ));
            }
            let conditions = match exported {
                PackageJsonExport::Single(target) => vec![(
                    "default".to_owned(),
                    ExportedPath::from(target.clone()).map_export(clean_path),
                )],
                PackageJsonExport::Conditional(conditions) => conditions
                    .iter()
                    .map(|(condition, target)| (condition.clone(), target.map_export(clean_path)))
                    .collect(),
            };
            resolution_data
                .exports
                .insert(clean_path(export_path), conditions);
        }
        Ok(resolution_data)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::{hash_map::HashMap, BTreeMap};
    use test_tmpdir::map2;

    #[test]
    fn string_roots_expose_package_files_and_explicit_maps_keep_encapsulation() {
        for json in [
            r#"{"exports":"./index.mjs"}"#,
            r#"{"exports":{".":"./index.mjs","./*":"./*","./private/*":null}}"#,
        ] {
            let package: packagejson::PackageJson = serde_json::from_str(json).unwrap();
            let rewrites =
                PackageExportRewriteData::try_from(package.exports.as_ref().unwrap()).unwrap();
            let mut output = String::new();
            assert_eq!(
                rewrites
                    .rewrite_relative_export(".", ["import"], &mut output)
                    .unwrap()
                    .unwrap()
                    .rewritten_export,
                ExportedPathRef::Exported("./index.mjs")
            );
            assert_eq!(
                rewrites
                    .rewrite_relative_export("./data/nested/file.json", ["import"], &mut output)
                    .unwrap()
                    .unwrap()
                    .rewritten_export,
                ExportedPathRef::Exported("./data/nested/file.json")
            );
            assert!(rewrites
                .rewrite_relative_export("../outside.json", ["import"], &mut output)
                .is_err());
            assert!(rewrites
                .rewrite_relative_export("./../../outside.json", ["import"], &mut output)
                .is_err());
            if json.contains("./*") {
                assert_eq!(
                    rewrites
                        .rewrite_relative_export("./private/file.json", ["import"], &mut output)
                        .unwrap()
                        .unwrap()
                        .rewritten_export,
                    ExportedPathRef::Private
                );
            } else {
                assert_eq!(
                    rewrites
                        .rewrite_relative_export("./index.mjs", ["import"], &mut output)
                        .unwrap()
                        .unwrap()
                        .rewritten_export,
                    ExportedPathRef::Exported("./index.mjs")
                );
            }
        }
    }

    #[test]
    fn star_patterns_choose_the_most_specific_key_before_conditions() {
        let exports = serde_json::from_str(
            r#"{
                "./*": {"import": "./generic/*.mjs"},
                "./feature/*": "./specific/*.mjs",
                "./feature/*.mjs": "./suffix/*.mjs",
                "./feature/private/*": null
            }"#,
        )
        .unwrap();
        for _ in 0..16 {
            let rewrites = PackageExportRewriteData::try_from(&exports).unwrap();
            for (request, expected) in [
                ("./feature/x", ExportedPathRef::Exported("./specific/x.mjs")),
                (
                    "./feature/x.mjs",
                    ExportedPathRef::Exported("./suffix/x.mjs"),
                ),
                ("./feature/private/x", ExportedPathRef::Private),
            ] {
                let mut out = String::new();
                let result = rewrites
                    .rewrite_relative_export(request, ["import"], &mut out)
                    .unwrap()
                    .unwrap();
                assert_eq!(result.rewritten_export, expected);
            }
        }
    }

    #[test]
    fn conditions_follow_declaration_order_for_every_subpath_kind() {
        for key in [".", "./feature/*", "./feature/"] {
            for (conditions, expected) in [
                (
                    r#"{"default":"./fallback","import":"./mode"}"#,
                    "./fallback",
                ),
                (r#"{"import":"./mode","default":"./fallback"}"#, "./mode"),
                (
                    r#"{"import":"./old","default":"./fallback","import":"./replacement"}"#,
                    "./replacement",
                ),
                (r#"{"default":null,"import":"./mode"}"#, ""),
            ] {
                let exports =
                    serde_json::from_str(&format!(r#"{{"{key}":{conditions}}}"#)).unwrap();
                let rewrites = PackageExportRewriteData::try_from(&exports).unwrap();
                let mut out = String::new();
                let request = if key == "." { "." } else { "./feature/" };
                let resolved = rewrites
                    .rewrite_relative_export(request, ["import"], &mut out)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    resolved.rewritten_export,
                    if expected.is_empty() {
                        ExportedPathRef::Private
                    } else {
                        ExportedPathRef::Exported(expected)
                    }
                );
            }
        }
    }

    #[test]
    fn inactive_selected_conditions_do_not_fall_back_to_broader_subpaths() {
        let exports = serde_json::from_str(
            r#"{
                "./*": "./generic/*.mjs",
                "./feature/*": {"browser":"./browser/*.mjs"},
                "./exact": {"browser":"./browser.mjs"}
            }"#,
        )
        .unwrap();
        let rewrites = PackageExportRewriteData::try_from(&exports).unwrap();
        for request in ["./feature/x", "./exact"] {
            assert!(rewrites
                .rewrite_relative_export(request, ["node", "import"], &mut String::new())
                .unwrap()
                .is_none());
        }
    }

    struct TestCase {
        exports: &'static str,
        expected: HashMap<&'static str, Vec<&'static str>>,
    }

    fn run_test_case(test_case: TestCase) {
        let exports: packagejson::PackageJsonExports =
            serde_json::from_str(test_case.exports).unwrap();
        let parsed_exports = PackageExportRewriteData::try_from(&exports).unwrap();
        let mut these_results = BTreeMap::<&'static str, Vec<String>>::new();
        let mut these_expected = BTreeMap::<&'static str, Vec<String>>::new();
        for (input, output) in test_case.expected {
            let conditions: Vec<_> = parsed_exports
                .get_exported_conditions(input)
                .iter()
                .map(|x| x.to_string())
                .collect();
            assert_eq!(!conditions.is_empty(), parsed_exports.is_exported(input));
            these_results.insert(input, conditions);
            these_expected.insert(input, output.iter().map(|x| x.to_string()).collect());
        }

        let e: String = serde_json::to_string_pretty(&these_expected).unwrap();
        let r: String = serde_json::to_string_pretty(&these_results).unwrap();
        pretty_assertions::assert_eq!(e, r)
    }

    #[test]
    fn test_unconditional_literal_export() {
        run_test_case(TestCase {
            exports: r#"{
                    ".": "./index.ts"
                }"#,
            expected: map2!(
                "./index.ts" => vec!["default"],
                "./index.js" => vec![],
                "./foo.js" => vec![]
            ),
        });
    }

    #[test]
    fn test_conditional_literal_export() {
        run_test_case(TestCase {
            exports: r#"{
                    ".": {
                        "source": "./index.ts",
                        "default": "./index.js"
                    }
                }"#,
            expected: map2!(
                "./index.ts" => vec!["source"],
                "./index.js" => vec!["default"],
                "./foo.js" => vec![]
            ),
        });
    }

    #[test]
    fn test_unconditional_dir_export() {
        run_test_case(TestCase {
            exports: r#"{
                    "./foo/": "./src/foo/"
                }"#,
            expected: map2!(
                "./src/foo.js" => vec![],
                "./src/bar/foo/bar.js" => vec![],
                "./src/foo/bar.js" => vec!["default"],
                "./src/foo/foo.js" => vec!["default"]
            ),
        });
    }

    #[test]
    fn test_conditional_dir_export() {
        run_test_case(TestCase {
            exports: r#"{
                    "./foo/": {
                        "import": "./src/foo_esm/",
                        "default": "./src/foo_cjs/"
                    }
                }"#,
            expected: map2!(
                "./src/foo.js" => vec![],
                "./src/foo_esm/bar.js" => vec!["import"],
                "./src/foo_esm/foo.js" => vec!["import"],
                "./src/foo_cjs/bar.js" => vec!["default"],
                "./src/foo_cjs/foo.js" => vec!["default"]
            ),
        });
    }

    #[test]
    fn test_unconditional_single_star_export() {
        run_test_case(TestCase {
            exports: r#"{
                    "./*dex": "./_*dex.ts"
                }"#,
            expected: map2!(
                "./index.ts" => vec![],
                "./_index.ts" => vec!["default"],
                "./_edex.ts" => vec!["default"],
                "./_eex.ts" => vec![],
                "./not.js" => vec![]
            ),
        })
    }

    #[test]
    fn test_conditional_single_star_export() {
        run_test_case(TestCase {
            exports: r#"{
                    "./*dex": {
                        "import": "./_*dex.ts",
                        "default": "./dex_*.ts"
                    }
                }"#,
            expected: map2!(
                "./index.ts" => vec![],
                "./_index.ts" => vec!["import"],
                "./dex_index.ts" => vec!["default"],
                "./dex_index.js" => vec![],
                "./_edex.ts" => vec!["import"],
                "./_eex.ts" => vec![],
                "./not.js" => vec![]
            ),
        })
    }

    #[test]
    fn test_unconditional_multi_star_export() {
        run_test_case(TestCase {
            exports: r#"{
                    "./*dex": "./_*d*.ts"
                }"#,
            expected: map2!(
                "./_indett.ts" => vec!["default"],
                "./indett.ts" => vec![]
            ),
        })
    }

    #[test]
    fn test_conditional_multi_star_export() {
        run_test_case(TestCase {
            exports: r#"{
                    "./*dex": {
                        "source": "./_*d*.ts",
                        "default": "./_*d*.js"
                    }
                }"#,
            expected: map2!(
                "./_indett.ts" => vec!["source"],
                "./indett.ts" => vec![],
                "./_indett.js" => vec!["default"],
                "./indett.js" => vec![]
            ),
        })
    }
}
