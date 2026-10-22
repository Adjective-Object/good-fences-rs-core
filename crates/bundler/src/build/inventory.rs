//! Demand admission is explicit; membership checks never perform filesystem IO.
use std::{ffi::OsString, sync::Mutex};

use ftree_cache::inputs::{InputKind, PathInput};

use super::*;

#[derive(Default)]
struct State {
    files: BTreeSet<PathBuf>,
    // Only checked directory links, with their named project routes.
    scopes: BTreeMap<PathBuf, PathBuf>,
    edges: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    aliases: BTreeMap<PathBuf, PathBuf>,
    inputs: BTreeMap<PathBuf, PathInput>,
    directories: BTreeMap<PathBuf, Vec<OsString>>,
    contexts: Vec<Context>,
    #[cfg(test)]
    directory_reads: usize,
}

#[derive(Clone)]
struct Context {
    importer: PathBuf,
    record: CacheRecord,
    dependency: ast_segmenter::owned::DependencyId,
    extensions: Vec<String>,
    candidates: BTreeMap<PathBuf, PathBuf>,
}

pub(super) type ExpandedContext = (
    PathBuf,
    ast_segmenter::owned::DependencyId,
    BTreeMap<String, PathBuf>,
);

pub(crate) struct Inventory {
    root: PathBuf,
    // Workers only consult membership and seal their own package-format reads.
    // Admission/context expansion remain coordinator-owned. Styles borrows this
    // same inventory throughout its transitive resource discovery.
    state: Mutex<State>,
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

fn within(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        let mut depth = 0usize;
        relative.components().all(|part| match part {
            std::path::Component::ParentDir => {
                if depth == 0 {
                    false
                } else {
                    depth -= 1;
                    true
                }
            }
            std::path::Component::Normal(name) => {
                depth += 1;
                !excluded(Path::new(name))
            }
            std::path::Component::CurDir => true,
            _ => false,
        })
    })
}

fn absolute_route(path: &Path) -> eyre::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            match prefix.kind() {
                Prefix::Disk(_) => {
                    let mut extended = std::ffi::OsString::from(r"\\?\");
                    extended.push(path.as_os_str());
                    return Ok(extended.into());
                }
                Prefix::UNC(server, share) => {
                    let mut extended = PathBuf::from(r"\\?\UNC");
                    extended.push(server);
                    extended.push(share);
                    for component in path.components().skip(2) {
                        extended.push(component);
                    }
                    return Ok(extended);
                }
                _ => {}
            }
        }
    }
    Ok(path)
}

fn checked_route(root: &Path, state: &State, route: &Path) -> Option<PathBuf> {
    let scopes = || std::iter::once(root).chain(state.scopes.keys().map(PathBuf::as_path));
    if scopes().any(|scope| within(route, scope)) {
        return Some(route.into());
    }
    if scopes().any(|scope| {
        route.strip_prefix(scope).is_ok_and(|relative| {
            relative.components().any(|part| {
                matches!(part, std::path::Component::Normal(_))
                    && excluded(Path::new(part.as_os_str()))
            })
        })
    }) {
        return None;
    }
    // Alternate host spellings (notably Windows case) and links back into a
    // known tree still need an actual checked ancestor, never a sibling search.
    for ancestor in route.ancestors() {
        let Ok(canonical) = ancestor.canonicalize() else {
            continue;
        };
        if scopes().any(|scope| canonical == scope) {
            let relative = route.strip_prefix(ancestor).ok()?;
            let normalized = canonical.join(relative);
            if within(&normalized, &canonical) {
                return Some(normalized);
            }
        }
    }
    None
}

impl Inventory {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            state: Mutex::new(State::default()),
        }
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.state.lock().unwrap().files.contains(path)
    }

    pub(super) fn can_admit(&self, route: &Path) -> bool {
        let Ok(route) = absolute_route(route) else {
            return false;
        };
        let state = self.state.lock().unwrap();
        checked_route(&self.root, &state, &route).is_some()
    }

    fn check_scopes(&self, state: &State, path: &Path) -> eyre::Result<()> {
        for scope in std::iter::once(&self.root).chain(state.scopes.keys()) {
            ensure!(
                !path.starts_with(scope) || within(path, scope),
                "inventory target is reserved/excluded: {}",
                path.display()
            );
        }
        Ok(())
    }

    fn observe(state: &mut State, input: PathInput) -> eyre::Result<()> {
        if let Some(before) = state.inputs.get(input.path()) {
            ensure!(
                before.kind() == input.kind()
                    && before.canonical() == input.canonical()
                    && before.identity() == input.identity()
                    && before.links() == input.links()
                    && (before.content_seal().is_none()
                        || input.content_seal().is_none()
                        || before.content_seal() == input.content_seal()),
                "resolution inputs changed during build: {}",
                input.path().display()
            );
        }
        if input.content_seal().is_some() || !state.inputs.contains_key(input.path()) {
            state.inputs.insert(input.path().into(), input);
        }
        Ok(())
    }

    pub(crate) fn probe_file(&self, path: &Path) -> eyre::Result<bool> {
        let input = PathInput::capture(&absolute_route(path)?)?;
        let file = input.kind() == InputKind::File;
        Self::observe(&mut self.state.lock().unwrap(), input)?;
        Ok(file)
    }

    /// Admit a lexical file route, never an arbitrary canonical external target.
    /// Directory links authorize their trees; file links authorize just the file.
    pub(crate) fn admit(&self, route: &Path) -> eyre::Result<PathBuf> {
        self.admit_path(route, InputKind::File)
    }

    fn admit_path(&self, route: &Path, kind: InputKind) -> eyre::Result<PathBuf> {
        let _span = tracing::info_span!("inventory.admit").entered();
        let route = absolute_route(route)?;
        let mut state = self.state.lock().unwrap();
        if kind == InputKind::File && state.files.contains(&route) {
            let input = PathInput::capture(&route)?;
            ensure!(
                input.kind() == InputKind::File && input.canonical() == Some(route.as_path()),
                "admitted source identity changed: {}",
                route.display()
            );
            Self::observe(&mut state, input)?;
            return Ok(route);
        }
        let original = route;
        let route = checked_route(&self.root, &state, &original).ok_or_else(|| {
            eyre::eyre!(
                "source outside reachable build inventory (or reserved/excluded): {}",
                original.display()
            )
        })?;
        if route != original {
            Self::observe(&mut state, PathInput::capture(&original)?)?;
        }
        let scope = std::iter::once(&self.root)
            .chain(state.scopes.keys())
            .filter(|scope| within(&route, scope))
            .max_by_key(|scope| scope.components().count())
            .cloned()
            .ok_or_else(|| {
                eyre::eyre!(
                    "source outside reachable build inventory (or reserved/excluded): {}",
                    route.display()
                )
            })?;
        let alias_root = state.scopes.get(&scope).cloned().unwrap_or(scope.clone());
        let mut prefix = scope.clone();
        let mut canonical = scope.clone();
        let mut scopes_changed = false;
        for part in route.strip_prefix(&scope)?.components() {
            let parent = canonical.clone();
            prefix.push(part);
            let input = PathInput::capture(&prefix)
                .wrap_err_with(|| format!("inventory target {}", prefix.display()))?;
            ensure!(
                matches!(input.kind(), InputKind::File | InputKind::Directory),
                "inventory target is missing or unsupported: {}",
                prefix.display()
            );
            canonical = input.canonical().expect("existing input").into();
            self.check_scopes(&state, &canonical)?;
            let linked = input.links().iter().any(|(path, _)| path == &prefix);
            ensure!(
                !linked || !excluded(&canonical),
                "inventory target is reserved/excluded: {} -> {}",
                prefix.display(),
                canonical.display()
            );
            let known = std::iter::once(&self.root)
                .chain(state.scopes.keys())
                .any(|scope| canonical.starts_with(scope));
            ensure!(
                known || linked,
                "source outside reachable build inventory: {}",
                prefix.display()
            );
            if input.kind() == InputKind::Directory && linked && !canonical.starts_with(&self.root)
            {
                scopes_changed |= !state.scopes.contains_key(&canonical);
                let alias = alias_root.join(prefix.strip_prefix(&scope)?);
                state
                    .scopes
                    .entry(canonical.clone())
                    .and_modify(|before| {
                        if alias < *before {
                            *before = alias.clone();
                        }
                    })
                    .or_insert(alias);
            }
            if input.kind() == InputKind::Directory {
                state
                    .edges
                    .entry(parent)
                    .or_default()
                    .insert(canonical.clone());
            }
            Self::observe(&mut state, input)?;
        }
        ensure!(
            if kind == InputKind::File {
                canonical.is_file()
            } else {
                canonical.is_dir()
            },
            "inventory input has the wrong file kind"
        );
        self.check_scopes(&state, &canonical)?;
        if kind == InputKind::File {
            let alias = alias_root.join(route.strip_prefix(&scope)?);
            state
                .aliases
                .entry(canonical.clone())
                .and_modify(|before| {
                    if alias < *before {
                        *before = alias.clone();
                    }
                })
                .or_insert(alias);
            state.files.insert(canonical.clone());
        }
        // A newly checked outer tree can expose an earlier cross-tree bypass.
        if scopes_changed {
            for file in &state.files {
                self.check_scopes(&state, file)?;
            }
            for scope in state.scopes.keys() {
                self.check_scopes(&state, scope)?;
            }
        }
        Ok(canonical)
    }

    pub(crate) fn relative(&self, root: &Path, path: &Path) -> PathBuf {
        if let Ok(relative) = path.strip_prefix(root) {
            return relative.into();
        }
        self.state.lock().unwrap().aliases[path]
            .strip_prefix(root)
            .expect("admitted external source has a named project route")
            .into()
    }

    pub(super) fn package_format(&self, source: &Path) -> eyre::Result<ModuleFormat> {
        for directory in source.parent().into_iter().flat_map(Path::ancestors) {
            let path = directory.join("package.json");
            let mut input = PathInput::capture(&path)?;
            let bytes = if input.kind() == InputKind::File {
                let bytes = fs::read(&path)?;
                input.seal_content(&bytes);
                Some(bytes)
            } else {
                None
            };
            input.validate()?;
            Self::observe(&mut self.state.lock().unwrap(), input)?;
            if let Some(bytes) = bytes {
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).wrap_err_with(|| path.display().to_string())?;
                return match json.get("type").and_then(|v| v.as_str()) {
                    Some("module") => Ok(ModuleFormat::Esm),
                    Some("commonjs") | None => Ok(ModuleFormat::CommonJs),
                    Some(other) => {
                        eyre::bail!("{}: unsupported package type {other}", path.display())
                    }
                };
            }
            if directory == self.root {
                break;
            }
        }
        Ok(ModuleFormat::CommonJs)
    }

    pub(super) fn candidates(
        &self,
        importer: &Path,
        record: &CacheRecord,
        dependency: ast_segmenter::views::Dependency<'_>,
        extensions: &[String],
    ) -> eyre::Result<std::collections::btree_map::IntoKeys<PathBuf, PathBuf>> {
        let candidates = self.scan(importer, dependency, extensions, false)?;
        self.state.lock().unwrap().contexts.push(Context {
            importer: importer.into(),
            record: record.clone(),
            dependency: dependency.id(),
            extensions: extensions.into(),
            candidates: candidates.clone(),
        });
        Ok(candidates.into_keys())
    }

    fn scan(
        &self,
        importer: &Path,
        dependency: ast_segmenter::views::Dependency<'_>,
        extensions: &[String],
        fresh: bool,
    ) -> eyre::Result<BTreeMap<PathBuf, PathBuf>> {
        let _span = tracing::info_span!("inventory.context_candidates").entered();
        let (scopes, mut candidates) = {
            let state = self.state.lock().unwrap();
            (
                std::iter::once(self.root.clone())
                    .chain(state.scopes.keys().cloned())
                    .collect::<Vec<_>>(),
                state
                    .files
                    .iter()
                    .filter(|path| {
                        crate::context::matches_candidate(importer, path, dependency, extensions)
                    })
                    .map(|path| (path.clone(), path.clone()))
                    .collect::<BTreeMap<_, _>>(),
            )
        };
        let mut directories = BTreeMap::new();
        for scope in &scopes {
            // Root routes cover lexical aliases; physical projections cover
            // contexts in canonical external importers, never arbitrary ancestors.
            if !crate::context::may_match_directory(importer, scope, dependency) {
                continue;
            }
            self.visit(
                scope,
                scope,
                scope,
                dependency,
                importer,
                extensions,
                fresh,
                &mut directories,
                &mut BTreeSet::new(),
                &mut candidates,
            )?;
        }
        Ok(candidates)
    }

    /// Later literal/resource admissions can expose physical projections for an
    /// earlier context. Settle those before graph construction, not at recheck.
    pub(super) fn expand_contexts(
        &self,
        resolver: &impl DependencyResolver,
    ) -> eyre::Result<Vec<ExpandedContext>> {
        let mut contexts = self.state.lock().unwrap().contexts.clone();
        contexts.sort_by(|a, b| (&a.importer, a.dependency.0).cmp(&(&b.importer, b.dependency.0)));
        let mut changed = Vec::new();
        for context in contexts {
            let dependency = context
                .record
                .module()
                .facts()
                .dependencies()
                .iter()
                .find(|dependency| dependency.id() == context.dependency)
                .expect("retained context occurrence");
            let candidates =
                self.scan(&context.importer, dependency, &context.extensions, false)?;
            if candidates == context.candidates {
                continue;
            }
            let resolved = crate::context::resolve(
                &self.root,
                &context.importer,
                dependency,
                candidates.keys(),
                &context.extensions,
                resolver,
            )
            .wrap_err_with(|| {
                format!(
                    "{}: context at bytes {}..{}",
                    context.importer.display(),
                    dependency.span().start,
                    dependency.span().end
                )
            })?;
            let mut state = self.state.lock().unwrap();
            let before = state
                .contexts
                .iter_mut()
                .find(|before| {
                    before.importer == context.importer && before.dependency == context.dependency
                })
                .expect("retained context occurrence");
            before.candidates = candidates;
            changed.push((context.importer, context.dependency, resolved));
        }
        Ok(changed)
    }

    #[expect(clippy::too_many_arguments)]
    fn visit(
        &self,
        anchor: &Path,
        alias: &Path,
        target: &Path,
        dependency: ast_segmenter::views::Dependency<'_>,
        importer: &Path,
        extensions: &[String],
        fresh: bool,
        directories: &mut BTreeMap<PathBuf, Vec<OsString>>,
        active: &mut BTreeSet<PathBuf>,
        candidates: &mut BTreeMap<PathBuf, PathBuf>,
    ) -> eyre::Result<()> {
        if !crate::context::may_match_directory(importer, alias, dependency)
            || !active.insert(target.into())
        {
            return Ok(());
        }
        if !fresh {
            ensure!(
                self.admit_path(alias, InputKind::Directory)? == target,
                "context directory route changed"
            );
        }
        if anchor != self.root && !self.reachable(anchor, active) {
            active.remove(target);
            return Ok(());
        }
        let cached = if fresh {
            directories.get(target).cloned()
        } else {
            self.state.lock().unwrap().directories.get(target).cloned()
        };
        let entries = if let Some(entries) = cached {
            entries
        } else {
            let _span = tracing::info_span!("inventory.read_directory").entered();
            let mut entries = fs::read_dir(target)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort();
            let mut state = self.state.lock().unwrap();
            #[cfg(test)]
            {
                state.directory_reads += 1;
            }
            if fresh {
                directories.insert(target.into(), entries.clone());
            } else {
                state.directories.insert(target.into(), entries.clone());
            }
            entries
        };
        for name in entries {
            let path = alias.join(name);
            if excluded(&path) {
                continue;
            }
            let matches =
                crate::context::matches_candidate(importer, &path, dependency, extensions);
            if !matches && !crate::context::may_match_directory(importer, &path, dependency) {
                continue;
            }
            let canonical = path
                .canonicalize()
                .wrap_err_with(|| format!("inventory target {}", path.display()))?;
            if canonical.is_dir() {
                self.visit(
                    anchor,
                    &path,
                    &canonical,
                    dependency,
                    importer,
                    extensions,
                    fresh,
                    directories,
                    active,
                    candidates,
                )?;
            } else if matches {
                if !fresh {
                    self.admit(&path)?;
                }
                candidates.insert(path, canonical);
            }
        }
        active.remove(target);
        Ok(())
    }

    fn reachable(&self, target: &Path, blocked: &BTreeSet<PathBuf>) -> bool {
        let state = self.state.lock().unwrap();
        let mut pending = vec![self.root.as_path()];
        let mut seen = BTreeSet::new();
        while let Some(directory) = pending.pop() {
            if directory == target {
                return true;
            }
            if blocked.contains(directory) || !seen.insert(directory) {
                continue;
            }
            if let Some(children) = state.edges.get(directory) {
                pending.extend(children.iter().map(PathBuf::as_path));
            }
        }
        false
    }

    pub(super) fn recheck(&self) -> eyre::Result<()> {
        let _span = tracing::info_span!("inventory.recheck").entered();
        // Workers have joined. Drop the mutex before recursive context scans.
        let contexts = {
            let state = self.state.lock().unwrap();
            for input in state.inputs.values() {
                input.validate()?;
            }
            for file in &state.files {
                self.check_scopes(&state, file)?;
            }
            for scope in state.scopes.keys() {
                self.check_scopes(&state, scope)?;
            }
            state.contexts.clone()
        };
        for context in contexts {
            let dependency = context
                .record
                .module()
                .facts()
                .dependencies()
                .iter()
                .find(|dependency| dependency.id() == context.dependency)
                .expect("retained context occurrence");
            ensure!(
                self.scan(&context.importer, dependency, &context.extensions, true)?
                    == context.candidates,
                "context candidates changed during build"
            );
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn directory_reads(&self) -> usize {
        self.state.lock().unwrap().directory_reads
    }
}
