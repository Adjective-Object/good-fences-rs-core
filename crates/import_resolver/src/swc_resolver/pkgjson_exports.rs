use core::fmt;
use std::{fmt::Display, path::PathBuf, str::FromStr};

use anyhow::{anyhow, Result};
use hashbrown::Equivalent;
use path_clean::PathClean;
use path_slash::PathBufExt;

use packagejson::{
    exported_path::{ExportedPath, ExportedPathRef},
    PackageJsonExport, PackageJsonExports,
};

use super::common::AHashMap;

// Pair path, export-condfition of form ('package-name/imported-path', 'import')
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
struct ExportKey(String, String);

// Implement Equivalent for our custom export key so we can do a key comparison
// without having to copy strings to consturct a new ExportKey
impl Equivalent<ExportKey> for (&str, &str) {
    fn equivalent(&self, key: &ExportKey) -> bool {
        *self.0 == key.0 && *self.1 == key.1
    }
}

// Struct that performs the "export" field remapping from package.json
//
// This holds derived data from the "exports" field in a package.json file, and
// is computed with PackageExportRewriteData::try_from(PackageJson)
#[derive(Debug, Default, Clone)]
pub struct PackageExportRewriteData {
    // Pre-computed export map for static exports
    // Map is of form ('package-name/imported-path', 'import') => "/absolute/path/to/exported"
    static_exports: hashbrown::HashMap<ExportKey, ExportedPath>,

    // Dynamic directory exports
    // Map of form:
    // {
    //  <export_condition> => [("./local-import/", "./remapped-path/"), ..(contd.)]
    // }
    directory_exports: hashbrown::HashMap<String, Vec<(String, ExportedPath)>>,

    // Star exports
    // Map of form:
    // {
    //  <export_condition> => [("./local-import/*-foo", "./cjs/remapped-*"), ..(contd.)]
    // }
    star_exports: hashbrown::HashMap<String, Vec<(String, ExportedPath)>>,
}

fn clean_path(p: &str) -> String {
    let mut store_str = String::new();
    return String::from(clean_path_avoid_alloc(p, &mut store_str));
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

        return ExportCondition::Condition(s);
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

fn resolve_export_condition<'a, TStr: Into<&'a str>, TItem, TOut>(
    // The export conditions to resolve against
    conditions: &'a hashbrown::HashMap<String, TItem>,
    // The requested export conditions (expected to be a pointer type, which is why its Clone here)
    requested_conditions: impl Clone + IntoIterator<Item = TStr>,
    // callback to map over members
    cb: impl Fn(&'a TItem) -> Option<TOut>,
) -> Option<(TOut, ExportCondition<'a>)> {
    for condition in requested_conditions {
        // skip the default condition, as we will check it last
        let cond_ref: &'a str = condition.into();
        if cond_ref == "default" {
            continue;
        }

        if let Some(Some(resolved)) = conditions.get(cond_ref).map(&cb) {
            return Some((resolved, ExportCondition::Condition(cond_ref)));
        }
    }

    if let Some(Some(resolved)) = conditions.get("default").map(&cb) {
        return Some((resolved, ExportCondition::Default));
    }

    None
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
    // Borrowed from the input list of requested export conditions
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

        // Check for literal matches in the resolution data
        // try all literal matches before we try any directory or star matches
        for export_condition in requested_export_conditions
            .clone()
            .into_iter()
            .map(|x| x.into())
            .chain(std::iter::once("default"))
        {
            let export_key = ExportKey(clean_relative_import.to_string(), export_condition.into());
            if let Some(matched) = self.static_exports.get(&export_key) {
                return Ok(Some(MatchedExport::with_kind(
                    matched.as_ref(),
                    export_condition.into(),
                )));
            }
        }

        // Check for directory matches in the resolution data
        let directory_match = resolve_export_condition(
            &self.directory_exports,
            requested_export_conditions.clone(),
            |x: &Vec<(String, ExportedPath)>| {
                x.iter()
                    .filter_map(|(directory_pattern, directory_export)| {
                        if clean_relative_import.starts_with(directory_pattern) {
                            Some((directory_pattern, directory_export))
                        } else {
                            None
                        }
                    })
                    .next()
            },
        );
        if let Some(((directory_pattern, directory_export), export_condition)) = directory_match {
            return Ok(Some(MatchedExport::with_kind(
                directory_export.as_ref().map_export(|v| {
                    rewrite_dir_export(clean_relative_import, directory_pattern, v, out);
                    out as &str
                }),
                export_condition,
            )));
        }

        // check for star matches in the resolution data
        let star_match = resolve_export_condition(
            &self.star_exports,
            requested_export_conditions.clone(),
            |x: &Vec<(String, ExportedPath)>| {
                x.iter()
                    .filter_map(|(star_pattern, star_export)| {
                        match_star_pattern(star_pattern, clean_relative_import)
                            .map(|star_match| (star_match, star_export))
                    })
                    .next()
            },
        );
        if let Some(((star_match, target), export_condition)) = star_match {
            return Ok(Some(MatchedExport::with_kind(
                // if we have a star match, rewrite the path and store it in `out`. Otherwise,
                // do nothing
                target.as_ref().map_export(|v| {
                    rewrite_star_export(star_match, v, out);
                    out as &str
                }),
                export_condition,
            )));
        }

        // No matches found
        Ok(None)
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

            // for simple exports, simulate a conditional exports map with a single entry, "default"
            // If needed, store it on the stack in my_cond_exp, allowing conditional_exports to be a reference type
            let my_cond_exp: AHashMap<String, ExportedPath>;
            let conditional_exports = match exported {
                PackageJsonExport::Single(export_target) => {
                    let entry = (
                        "default".to_string(),
                        match export_target {
                            Some(v) => ExportedPath::Exported(v.to_string()),
                            None => ExportedPath::Private,
                        },
                    );
                    my_cond_exp = AHashMap::from_iter(vec![entry].drain(..));
                    &my_cond_exp
                }
                PackageJsonExport::Conditional(conditional_exports) => conditional_exports,
            };

            let export_path_star_ct = export_path.chars().filter(|c| *c == '*').count();
            if export_path_star_ct == 1 {
                // star pattern
                for (export_condition, export_target) in conditional_exports.iter() {
                    resolution_data
                        .star_exports
                        .entry_ref(export_condition)
                        .or_insert_with(Vec::new)
                        .push((
                            clean_path(export_path),
                            export_target.map_export(clean_path),
                        ));
                }
            } else if export_path_star_ct > 1 {
                return Err(anyhow!(
                    "Invalid star pattern '{}' in package.json exports field: \
                            Star patterns may contain at most a single star match.",
                    export_path
                ));
            } else if export_path.ends_with('/') {
                // deprecated node 14.x directory pattern
                for (export_condition, export_target) in conditional_exports.iter() {
                    resolution_data
                        .star_exports
                        .entry_ref(export_condition)
                        .or_insert_with(Vec::new)
                        .push((
                            clean_path(export_path),
                            export_target.map_export(clean_path),
                        ));
                }
            } else {
                // literal export
                for (export_condition, export_target) in conditional_exports.iter() {
                    resolution_data.static_exports.insert(
                        ExportKey(clean_path(export_path), export_condition.clone()),
                        export_target.map_export(clean_path),
                    );
                }
            }
        }
        Ok(resolution_data)
    }
}
