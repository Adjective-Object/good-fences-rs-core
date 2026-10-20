//! Public output names project canonical native files, not low-level root IDs.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use eyre::{ensure, OptionExt, WrapErr};

use super::BuildOptions;
use crate::{
    assembly::{Code, RequestKind},
    link::{Artifacts, RUNTIME_PATH},
};

pub(super) fn apply(
    root: &Path,
    options: &BuildOptions,
    artifacts: &mut Artifacts<Code>,
) -> eyre::Result<BTreeMap<PathBuf, PathBuf>> {
    let mut names = BTreeMap::new();
    let mut renames: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut aliases = BTreeSet::new();
    let mut packages = BTreeMap::new();
    for (name, entry) in &options.entries {
        let original = artifacts
            .entries
            .get(name)
            .ok_or_eyre("missing emitted public entry")?;
        let commonjs = original.extension().is_some_and(|e| e == "cjs");
        let mut requested = match &entry.output_name {
            Some(output) => output.clone(),
            None => root
                .join(&entry.path)
                .canonicalize()?
                .strip_prefix(root)?
                .to_owned(),
        };
        if entry.output_name.is_none()
            && entry
                .path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            requested.set_extension(if commonjs { "cjs" } else { "mjs" });
        }
        let mut path = output_path(
            requested
                .to_str()
                .ok_or_eyre("non-UTF-8 entry output path")?,
        )
        .wrap_err_with(|| format!("entry {name:?} output {}", requested.display()))?;
        if path.extension().is_none() {
            let extension = entry
                .path
                .extension()
                .and_then(|s| s.to_str())
                .filter(|s| matches!(*s, "mjs" | "cjs" | "js"))
                .unwrap_or(if commonjs { "cjs" } else { "mjs" });
            path.set_extension(extension);
        }
        let extension = path.extension().and_then(|s| s.to_str());
        ensure!(
            extension == Some("js") || extension == Some(if commonjs { "cjs" } else { "mjs" }),
            "entry output {name:?}: incompatible output extension for {}",
            if commonjs { "CommonJS" } else { "ESM" }
        );
        if extension == Some("js") {
            let package = path.parent().unwrap().join("package.json");
            let format = if commonjs { "commonjs" } else { "module" };
            if let Some(previous) = packages.insert(package.clone(), format) {
                ensure!(
                    previous == format,
                    "entry output {name:?}: incompatible .js module formats in {}",
                    package.display()
                );
            }
        }
        names.insert(name.clone(), path.clone());
        if let Some(canonical) = renames.get(original) {
            if &path != canonical {
                aliases.insert((path, canonical.clone(), original.clone(), commonjs));
            }
        } else {
            renames.insert(original.clone(), path);
        }
    }
    let project = |path: &PathBuf| renames.get(path).cloned().unwrap_or_else(|| path.clone());
    let mut destinations = BTreeSet::new();
    let mut portable_destinations = BTreeSet::new();
    for path in artifacts.files.keys().map(&project).chain(
        aliases
            .iter()
            .map(|(alias, _, _, _)| alias.clone())
            .chain(packages.keys().cloned()),
    ) {
        ensure!(
            portable_destinations.insert(PathBuf::from(
                path.to_str()
                    .ok_or_eyre("non-UTF-8 output path")?
                    .to_lowercase()
            )),
            "entry output collides with generated artifact: {}",
            path.display()
        );
        destinations.insert(path);
    }
    for path in &destinations {
        let portable = PathBuf::from(path.to_str().unwrap().to_lowercase());
        ensure!(
            !portable
                .ancestors()
                .skip(1)
                .any(|p| portable_destinations.contains(p)),
            "entry output file/directory collision: {}",
            path.display()
        );
    }
    let mut destinations: BTreeMap<_, _> = artifacts
        .files
        .keys()
        .map(|id| (id.clone(), project(id)))
        .collect();
    for (index, (alias, _, original, commonjs)) in aliases.iter().enumerate() {
        let source = if *commonjs {
            let mut source = Code::request(PathBuf::from(RUNTIME_PATH), RequestKind::Require)
                .surrounded("const {defineProperty} = require(", ");\n");
            source.push(
                Code::request(original.clone(), RequestKind::Require).surrounded(
                    "defineProperty(module, 'exports', {get: () => require(",
                    ")});\n",
                ),
            );
            source.push(
                Code::request(original.clone(), RequestKind::Require)
                    .surrounded("require(", ");\n"),
            );
            source
        } else {
            let mut source = Code::request(original.clone(), RequestKind::ReExport)
                .surrounded("export * from ", ";\n");
            if artifacts.files[original].default_export {
                source.push(
                    Code::request(original.clone(), RequestKind::ReExport)
                        .surrounded("export {default} from ", ";\n"),
                );
                source.default_export = true;
            }
            source
        };
        let mut id = PathBuf::from(format!(".bundler-alias-{index}"));
        while artifacts.files.contains_key(&id) {
            id.push("alias");
        }
        destinations.insert(id.clone(), alias.clone());
        artifacts.files.insert(id, source);
    }
    for (package, format) in packages {
        destinations.insert(package.clone(), package.clone());
        artifacts
            .files
            .insert(package, Code::from(format!("{{\"type\":\"{format}\"}}\n")));
    }
    artifacts.entries = names;
    for path in artifacts
        .modules
        .values_mut()
        .chain(artifacts.members.values_mut())
        .chain(artifacts.cjs_proxies.values_mut().map(|(path, _)| path))
        .chain(artifacts.cjs_namespaces.values_mut())
        .chain(artifacts.async_facades.values_mut())
    {
        *path = project(path);
    }
    for paths in artifacts.chunk_artifacts.values_mut() {
        *paths = paths.iter().map(&project).collect();
        for (alias, canonical, _, _) in &aliases {
            if paths.contains(canonical) {
                paths.insert(alias.clone());
            }
        }
    }
    Ok(destinations)
}

fn output_path(name: &str) -> eyre::Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && !name.ends_with('/')
            && !name
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c)),
        "unsafe or nonportable output path"
    );
    let path = Path::new(name);
    ensure!(!path.is_absolute(), "absolute output path");
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir if result.as_os_str().is_empty() => continue,
            Component::Normal(value) => {
                let value = value.to_str().ok_or_eyre("non-UTF-8 output path")?;
                let stem = value.split('.').next().unwrap().to_ascii_lowercase();
                ensure!(
                    !value.starts_with(".bundler-")
                        && !value.ends_with(['.', ' '])
                        && !matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
                        && !(stem.len() == 4
                            && (stem.starts_with("com") || stem.starts_with("lpt"))
                            && matches!(stem.as_bytes()[3], b'1'..=b'9')),
                    "reserved output path component: {value}"
                );
                result.push(value);
            }
            _ => eyre::bail!("output path contains traversal or non-normal components"),
        }
    }
    ensure!(!result.as_os_str().is_empty(), "empty output path");
    ensure!(
        !segment_cache::is_cache_artifact(&result),
        "reserved source-cache output path"
    );
    Ok(result)
}
