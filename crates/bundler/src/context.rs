//! Bounded source-file closure capture; generated requests remain typed.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use ast_segmenter::owned::DependencyKind;
use ast_segmenter::views::Dependency;
use import_resolver::structural::{DependencyResolver, ResolutionMode, ResolvedDependency};

use crate::assembly::{self, Code};

pub(crate) fn resolve(
    root: &Path,
    importer: &Path,
    dependency: Dependency<'_>,
    inventory: impl Iterator<Item = impl AsRef<Path>>,
    extensions: &[String],
    resolver: &impl DependencyResolver,
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
        if dependency.kind() == DependencyKind::DynamicImport {
            let encoded = assembly::relative_request(importer, &target, true)
                .expect("the unencoded relative request exists");
            if dependency.matches_specifier(&encoded) {
                requests.insert(encoded, request.clone());
            }
            if request.contains(['#', '?', '%']) {
                continue;
            }
        }
        if dependency.matches_specifier(&request) {
            requests.insert(request.clone(), request.clone());
        }
        if dependency.kind() == DependencyKind::Require {
            for extension in extensions {
                if let Some(stem) =
                    request.strip_suffix(&format!(".{}", extension.trim_start_matches('.')))
                {
                    if dependency.matches_specifier(stem) {
                        requests.insert(stem.to_owned(), stem.to_owned());
                    }
                    if let Some(directory) = stem.strip_suffix("/index") {
                        for alias in [directory.to_owned(), format!("{directory}/")] {
                            if dependency.matches_specifier(&alias) {
                                requests.insert(alias.clone(), alias);
                            }
                        }
                    }
                }
            }
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
    pub runtime: String,
}

impl Dispatch {
    pub fn emit(&self, argument: Code) -> Code {
        let mut output = Code::from("((key) => { switch (key) {\n");
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
