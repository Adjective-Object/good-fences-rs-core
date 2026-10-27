//! Bounded source-file closure capture; generated requests remain typed.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ast_segmenter::owned::DependencyKind;
use ast_segmenter::views::Dependency;
use import_resolver::structural::{DependencyResolver, ResolutionMode, ResolvedDependency};

use crate::assembly::{self, Code};

fn nonlocal_require(dependency: Dependency<'_>) -> bool {
    dependency.kind() == DependencyKind::Require
        && dependency.pattern().is_some_and(|parts| {
            !parts
                .first()
                .is_some_and(|part| part.starts_with("./") || part.starts_with("../"))
        })
}

fn candidate_requests(
    importer: &Path,
    target: &Path,
    dependency: Dependency<'_>,
    extensions: &[String],
    commonjs: bool,
) -> Vec<String> {
    let matches = |request: &str| {
        if !commonjs && dependency.kind() == DependencyKind::Require {
            dependency.matches_esm_require_specifier(request)
        } else {
            dependency.matches_specifier(request)
        }
    };
    if nonlocal_require(dependency) {
        return Vec::new();
    }
    // Context enumeration selects runtime inputs, not TypeScript declarations.
    // Keep explicit requests on the normal resolver/precompiler error path.
    if target
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            [".d.ts", ".d.mts", ".d.cts"]
                .iter()
                .any(|suffix| name.ends_with(suffix))
        })
    {
        return Vec::new();
    }
    if dependency.broad_context()
        && !importer
            .parent()
            .is_some_and(|directory| target.starts_with(directory))
    {
        return Vec::new();
    }
    let Some(request) = assembly::relative_request(importer, target, false) else {
        return Vec::new();
    };
    let mut requests = Vec::new();
    if dependency.kind() == DependencyKind::DynamicImport {
        if let Some(encoded) = assembly::relative_request(importer, target, true) {
            if matches(&encoded) {
                requests.push(encoded);
            }
        }
        if request.contains(['#', '?', '%']) {
            return requests;
        }
    }
    if matches(&request) {
        requests.push(request.clone());
    }
    if dependency.kind() == DependencyKind::Require {
        for extension in extensions {
            if let Some(stem) =
                request.strip_suffix(&format!(".{}", extension.trim_start_matches('.')))
            {
                if matches(stem) {
                    requests.push(stem.to_owned());
                }
                if let Some(directory) = stem.strip_suffix("/index") {
                    for alias in [directory.to_owned(), format!("{directory}/")] {
                        if matches(&alias) {
                            requests.push(alias);
                        }
                    }
                }
            }
        }
    }
    requests
}

pub(crate) fn matches_candidate(
    importer: &Path,
    target: &Path,
    dependency: Dependency<'_>,
    extensions: &[String],
    commonjs: bool,
) -> bool {
    !candidate_requests(importer, target, dependency, extensions, commonjs).is_empty()
}

pub(crate) fn resolve(
    root: &Path,
    importer: &Path,
    dependency: Dependency<'_>,
    inventory: impl Iterator<Item = impl AsRef<Path>>,
    extensions: &[String],
    resolver: &impl DependencyResolver,
    commonjs: bool,
) -> eyre::Result<BTreeMap<String, PathBuf>> {
    let mut requests = BTreeMap::new();
    for relative in inventory {
        let target = root.join(relative.as_ref());
        eyre::ensure!(
            importer.to_str().is_some() && target.to_str().is_some(),
            "context source paths must be UTF-8"
        );
        let Some(request) = assembly::relative_request(importer, &target, false) else {
            continue;
        };
        for key in candidate_requests(importer, &target, dependency, extensions, commonjs) {
            let resolved_request = if dependency.kind() == DependencyKind::Require {
                key.clone()
            } else {
                request.clone()
            };
            requests.insert(key, resolved_request);
        }
    }
    requests
        .into_iter()
        .map(|(key, request)| {
            let mode = if dependency.kind() == DependencyKind::Require {
                ResolutionMode::Require
            } else {
                ResolutionMode::Import
            };
            let target = resolver.resolve_dependency(importer, &request, mode)?;
            let ResolvedDependency::File(path) = target else {
                eyre::bail!("relative context candidate resolved externally: {request:?}");
            };
            Ok((key, path))
        })
        .collect()
}

pub(crate) fn may_match_directory(
    importer: &Path,
    directory: &Path,
    dependency: Dependency<'_>,
) -> bool {
    if nonlocal_require(dependency) {
        return false;
    }
    if dependency.broad_context() {
        return importer
            .parent()
            .is_some_and(|base| directory.starts_with(base) || base.starts_with(directory));
    }
    if importer.starts_with(directory) || directory.to_str().is_none() {
        return true;
    }
    let prefix = dependency
        .pattern()
        .as_ref()
        .and_then(|parts| parts.first())
        .unwrap_or(dependency.specifier());
    [false, dependency.kind() == DependencyKind::DynamicImport]
        .into_iter()
        .any(|url| {
            assembly::relative_request(importer, directory, url).is_some_and(|mut request| {
                request.push('/');
                request.starts_with(prefix) || prefix.starts_with(&request)
                    // Require also exposes a directory's index under its name.
                    || (dependency.kind() == DependencyKind::Require
                        && dependency.matches_specifier(request.trim_end_matches('/')))
            })
        })
}

#[derive(Clone)]
pub(crate) struct Dispatch {
    pub requests: BTreeMap<String, Code>,
    pub dynamic: bool,
    pub reject_parent: bool,
    pub runtime: String,
}

impl Dispatch {
    pub fn emit(&self, argument: Code) -> Code {
        let mut output = Code::from("((key) => {\n");
        if self.dynamic {
            // Native import coerces once and turns coercion failures into
            // rejected promises rather than synchronous throws.
            output.push_str("try { key = `${key}`; } catch (error) { return import({toString() { throw error; }}); }\n");
        } else if self.reject_parent {
            output.push_str(&format!(
                "for (let i = 0; i < key.length; i++) {{ if (key[i] === '.' && key[i + 1] === '.' && (i === 0 || key[i - 1] === '/' || key[i - 1] === '\\\\') && (i + 2 === key.length || key[i + 2] === '/' || key[i + 2] === '\\\\')) {{ const error = new {}.Error('Parent traversal is not supported in require context: ' + key); error.code = 'MODULE_NOT_FOUND'; throw error; }} }}\n",
                self.runtime
            ));
        }
        output.push_str("switch (key) {\n");
        for (request, expression) in &self.requests {
            output.push_str(&format!(
                "case {}: return ",
                serde_json::to_string(request).unwrap()
            ));
            output.push(expression.clone());
            output.push_str(";\n");
        }
        let error = format!("{}.Error", self.runtime);
        // Native coercion rejects failed lookups; argument evaluation stays inline.
        output.push_str(if self.dynamic {
            "default: return import({toString() {"
        } else {
            "default: {"
        });
        output.push_str(&format!(
            "const error = new {error}('Module not found: ' + key); error.code = '{}'; throw error;",
            if self.dynamic { "ERR_MODULE_NOT_FOUND" } else { "MODULE_NOT_FOUND" },
        ));
        output.push_str(if self.dynamic {
            "}});\n} })("
        } else {
            "}\n} })("
        });
        output.push(argument);
        output.push_str(")");
        output
    }
}
