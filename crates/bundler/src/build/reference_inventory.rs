//! Test-only exhaustive oracle for linked graph and exclusion regressions.
use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Inventory {
    files: BTreeMap<PathBuf, Option<[u8; 32]>>,
    directories: BTreeMap<PathBuf, BTreeMap<std::ffi::OsString, PathBuf>>,
    aliases: BTreeMap<PathBuf, PathBuf>,
    configs: BTreeMap<PathBuf, Option<(PathBuf, [u8; 32])>>,
    #[cfg(test)]
    directory_reads: usize,
}

impl Inventory {
    pub(super) fn recheck(&self, root: &Path) -> eyre::Result<()> {
        ensure!(
            inventory(root)? == *self,
            "resolution inputs changed during build"
        );
        Ok(())
    }
    pub(crate) fn candidates(
        &self,
        root: &Path,
        importer: &Path,
        dependency: ast_segmenter::views::Dependency<'_>,
    ) -> impl Iterator<Item = PathBuf> {
        self.context_candidates(root, importer, dependency)
            .0
            .into_iter()
    }

    pub(super) fn context_candidates(
        &self,
        root: &Path,
        importer: &Path,
        dependency: ast_segmenter::views::Dependency<'_>,
    ) -> (BTreeSet<PathBuf>, usize) {
        let _span = tracing::info_span!(
            "inventory.context_candidates",
            files = self.files.len(),
            directories = self.directories.len()
        )
        .entered();
        struct Walk<'a> {
            inventory: &'a Inventory,
            root: &'a Path,
            importer: &'a Path,
            dependency: ast_segmenter::views::Dependency<'a>,
            candidates: BTreeSet<PathBuf>,
            visits: usize,
        }
        impl Walk<'_> {
            fn visit(
                &mut self,
                anchor: &Path,
                alias: &Path,
                target: &Path,
                active: &mut BTreeSet<PathBuf>,
            ) {
                if !crate::context::may_match_directory(self.importer, alias, self.dependency)
                    || !active.insert(target.to_owned())
                {
                    return;
                }
                // Physical ancestor projections must extend some non-cyclic
                // root route, not invent aliases through a cut ancestor cycle.
                if anchor != self.root && !self.inventory.reachable(self.root, anchor, active) {
                    active.remove(target);
                    return;
                }
                self.visits += 1;
                for (name, child) in &self.inventory.directories[target] {
                    let path = alias.join(name);
                    if self.inventory.directories.contains_key(child) {
                        self.visit(anchor, &path, child, active);
                    } else {
                        self.candidates.insert(path);
                    }
                }
                active.remove(target);
            }
        }
        let mut walk = Walk {
            inventory: self,
            root,
            importer,
            dependency,
            candidates: self.files.keys().cloned().collect(),
            visits: 0,
        };
        walk.visit(root, root, root, &mut BTreeSet::new());
        for directory in self.directories.keys() {
            if directory != root && importer.starts_with(directory) {
                walk.visit(directory, directory, directory, &mut BTreeSet::new());
            }
        }
        (walk.candidates, walk.visits)
    }

    #[cfg(test)]
    pub(super) fn directory_reads(&self) -> usize {
        self.directory_reads
    }

    fn reachable(&self, root: &Path, target: &Path, blocked: &BTreeSet<PathBuf>) -> bool {
        let mut pending = vec![root];
        let mut seen = BTreeSet::new();
        while let Some(directory) = pending.pop() {
            if directory == target {
                return true;
            }
            if blocked.contains(directory) || !seen.insert(directory) {
                continue;
            }
            if let Some(entries) = self.directories.get(directory) {
                pending.extend(entries.values().map(PathBuf::as_path));
            }
        }
        false
    }
}

fn excluded(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name == ".git"
            || name == "target"
            || name.to_string_lossy().starts_with(".bundler-")
            || segment_cache::is_cache_artifact(path)
            || crate::upstream::is_cache_artifact(path)
    })
}

/// Only directory edges reachable from the input root are traversed. Ancestor
/// config probes match resolver ancestry but never enumerate ancestor trees.
pub(super) fn inventory(root: &Path) -> eyre::Result<Inventory> {
    let _span = tracing::info_span!("build.inventory").entered();
    fn visit(
        root: &Path,
        scope: &Path,
        dir: &Path,
        canonical: &Path,
        scopes: &mut BTreeSet<PathBuf>,
        out: &mut Inventory,
    ) -> eyre::Result<()> {
        scopes.insert(scope.to_owned());
        if out.directories.contains_key(canonical) {
            return Ok(());
        }
        let _span = tracing::info_span!("inventory.directory").entered();
        out.directories
            .insert(canonical.to_owned(), BTreeMap::new());
        #[cfg(test)]
        {
            out.directory_reads += 1;
        }
        let mut entries = tracing::info_span!("inventory.read_directory").in_scope(|| {
            fs::read_dir(canonical)
                .wrap_err_with(|| format!("inventory directory {}", dir.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(eyre::Report::from)
        })?;
        entries.sort_by_key(|entry| entry.file_name());
        let _entries_span =
            tracing::info_span!("inventory.entries", entries = entries.len()).entered();
        for entry in entries {
            let path = dir.join(entry.file_name());
            if excluded(&path) {
                continue;
            }
            // Canonicalize every entry: this follows both symlinks and Windows
            // junctions, and reports dangling links rather than hiding them.
            let target = entry
                .path()
                .canonicalize()
                .wrap_err_with(|| format!("inventory target {}", path.display()))?;
            ensure!(
                !target
                    .strip_prefix(root)
                    .or_else(|_| target.strip_prefix(scope))
                    .unwrap_or_else(|_| Path::new(target.file_name().unwrap_or_default()))
                    .ancestors()
                    .any(excluded),
                "inventory target is reserved/excluded: {} -> {}",
                path.display(),
                target.display()
            );
            let metadata = fs::metadata(&target)
                .wrap_err_with(|| format!("inventory metadata {}", path.display()))?;
            out.directories
                .get_mut(canonical)
                .unwrap()
                .insert(entry.file_name(), target.clone());
            if metadata.is_dir() {
                // A newly reached external tree owns its exclusions, not its
                // unrelated absolute ancestors (which may be named `target`).
                let scope = if target.starts_with(scope) {
                    scope
                } else {
                    &target
                };
                visit(root, scope, &path, &target, scopes, out)?;
            } else if metadata.is_file() {
                out.aliases.entry(target.clone()).or_insert(path.clone());
                let config = path
                    .extension()
                    .is_some_and(|e| e == "json" || e == "jsonc")
                    || target
                        .extension()
                        .is_some_and(|e| e == "json" || e == "jsonc");
                let digest = if config {
                    Some(
                        out.files
                            .get(&target)
                            .copied()
                            .flatten()
                            .map(Ok)
                            .unwrap_or_else(|| fs::read(&target).map(|bytes| hash(&bytes)))?,
                    )
                } else {
                    None
                };
                let previous = out.files.entry(target).or_insert(digest);
                if digest.is_some() {
                    *previous = digest;
                }
            } else {
                eyre::bail!("unsupported inventory input: {}", path.display());
            }
        }
        Ok(())
    }

    let mut result = Inventory::default();
    let canonical_root = root
        .canonicalize()
        .wrap_err_with(|| format!("inventory target {}", root.display()))?;
    let mut scopes = BTreeSet::new();
    visit(
        root,
        &canonical_root,
        root,
        &canonical_root,
        &mut scopes,
        &mut result,
    )?;
    // A later link can expose the containing tree of an earlier alias. Validate
    // every edge against the outermost known scope, independent of visit order.
    let validation_span = tracing::info_span!(
        "inventory.validate_scopes",
        directories = result.directories.len(),
        files = result.files.len()
    )
    .entered();
    for (directory, entries) in &result.directories {
        for (name, target) in entries {
            if let Some(scope) = target
                .ancestors()
                .filter(|ancestor| scopes.contains(*ancestor))
                .last()
            {
                ensure!(
                    !target
                        .strip_prefix(scope)
                        .unwrap()
                        .ancestors()
                        .any(excluded),
                    "inventory target is reserved/excluded: {} -> {}",
                    directory.join(name).display(),
                    target.display()
                );
            }
        }
    }
    drop(validation_span);
    let ancestors_span =
        tracing::info_span!("inventory.config_ancestors", files = result.files.len()).entered();
    let mut ancestors = BTreeSet::new();
    for file in result.files.keys() {
        for directory in file.parent().into_iter().flat_map(Path::ancestors) {
            ancestors.insert(directory.to_owned());
            if directory == root {
                break;
            }
        }
    }
    drop(ancestors_span);
    let _probes_span =
        tracing::info_span!("inventory.config_probes", directories = ancestors.len()).entered();
    for directory in ancestors {
        for name in ["package.json", "tsconfig.json"] {
            let path = directory.join(name);
            let config = match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let target = path
                        .canonicalize()
                        .wrap_err_with(|| format!("inventory config target {}", path.display()))?;
                    let digest = result
                        .files
                        .get(&target)
                        .copied()
                        .flatten()
                        .map(Ok)
                        .unwrap_or_else(|| fs::read(&path).map(|bytes| hash(&bytes)))?;
                    Some((target, digest))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error).wrap_err_with(|| path.display().to_string()),
            };
            result.configs.insert(path, config);
        }
    }
    Ok(result)
}
