//! Best-effort candidates only: package metadata never authorizes an import.
use super::*;
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

struct Progress {
    start: Instant,
    packages: AtomicU64,
    directories: AtomicU64,
    files: AtomicU64,
    next_package: AtomicU64,
    next_directory: AtomicU64,
    next_file: AtomicU64,
}

impl Progress {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            packages: AtomicU64::new(0),
            directories: AtomicU64::new(0),
            files: AtomicU64::new(0),
            next_package: AtomicU64::new(0),
            next_directory: AtomicU64::new(0),
            next_file: AtomicU64::new(0),
        }
    }

    fn ready(&self, next: &AtomicU64) -> bool {
        let elapsed = self.start.elapsed().as_millis() as u64;
        let previous = next.load(Ordering::Relaxed);
        elapsed >= previous
            && next
                .compare_exchange(
                    previous,
                    elapsed.saturating_add(1_000),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
    }

    fn package(&self) {
        let packages = self.packages.fetch_add(1, Ordering::Relaxed) + 1;
        if self.ready(&self.next_package) {
            tracing::info!(target: "bundler::progress::discovery", packages,
                "Discovering dependency packages");
        }
    }

    fn directory(&self, root: &Path, path: &Path) {
        let directories = self.directories.fetch_add(1, Ordering::Relaxed) + 1;
        if self.ready(&self.next_directory) {
            let relative = path.strip_prefix(root).unwrap_or(path);
            let relative = if relative.as_os_str().is_empty() {
                Path::new(".")
            } else {
                relative
            };
            tracing::info!(target: "bundler::progress::discovery", directories,
                directory = ?relative,
                "Traversing local source directory");
        }
    }

    fn file(&self, root: &Path, path: &Path) {
        let files = self.files.fetch_add(1, Ordering::Relaxed) + 1;
        if self.ready(&self.next_file) {
            tracing::info!(target: "bundler::progress::discovery", files,
                file = ?path.strip_prefix(root).unwrap_or(path),
                "Discovered local source file");
        }
    }
}

#[derive(Debug)]
struct IgnoreFile {
    matcher: ignore::gitignore::Gitignore,
    parent: Option<Arc<Self>>,
}

impl IgnoreFile {
    fn push(head: &mut Option<Arc<Self>>, root: &Path, path: &Path) {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        builder.add(path);
        let Ok(matcher) = builder.build() else { return };
        if !matcher.is_empty() {
            *head = Some(Arc::new(Self {
                matcher,
                parent: head.clone(),
            }));
        }
    }
}

#[derive(Debug, Clone, Default)]
struct WalkState {
    selected: bool,
    ignores: Option<Arc<IgnoreFile>>,
    gitignores: Option<Arc<IgnoreFile>>,
    excludes: Option<Arc<IgnoreFile>>,
    global: Option<Arc<IgnoreFile>>,
}

impl WalkState {
    fn ignored(&self, path: &Path, directory: bool) -> bool {
        for head in [
            &self.ignores,
            &self.gitignores,
            &self.excludes,
            &self.global,
        ] {
            let mut current = head.as_deref();
            while let Some(file) = current {
                match file.matcher.matched(path, directory) {
                    ignore::Match::Ignore(_) => return true,
                    ignore::Match::Whitelist(_) => return false,
                    ignore::Match::None => current = file.parent.as_deref(),
                }
            }
        }
        false
    }
}

fn manifest(package: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&fs::read(package.join("package.json")).ok()?).ok()
}

fn owner(root: &Path, entry: &Path) -> PathBuf {
    let directory = entry.parent().unwrap_or(root);
    for ancestor in directory.ancestors() {
        if ancestor.join("package.json").is_file() {
            return ancestor.into();
        }
        if ancestor == root {
            break;
        }
    }
    if entry.starts_with(root) {
        root.into()
    } else {
        directory.into()
    }
}

fn installed(
    package: &Path,
    name: &str,
    installations: &parking_lot::RwLock<HashMap<PathBuf, Option<PathBuf>>>,
) -> Option<PathBuf> {
    if name.starts_with('.')
        || name.starts_with('/')
        || name.contains('\\')
        || name.split('/').any(|part| part == ".." || part.is_empty())
    {
        return None;
    }
    package
        .ancestors()
        .filter(|ancestor| {
            ancestor
                .file_name()
                .is_none_or(|name| name != "node_modules")
        })
        .find_map(|ancestor| {
            let candidate = name
                .split('/')
                .fold(ancestor.join("node_modules"), |path, part| path.join(part));
            let cached = installations.read().get(&candidate).cloned();
            cached.unwrap_or_else(|| {
                let selected = candidate
                    .join("package.json")
                    .is_file()
                    .then(|| candidate.canonicalize().ok())
                    .flatten();
                installations
                    .write()
                    .entry(candidate)
                    .or_insert(selected)
                    .clone()
            })
        })
}

fn local(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        !relative
            .components()
            .any(|part| part.as_os_str() == "node_modules")
    })
}

pub(super) fn candidates(
    root: &Path,
    entries: impl Iterator<Item = PathBuf>,
    options: &BuildOptions,
) -> eyre::Result<BTreeSet<PathBuf>> {
    options
        .execution
        .scoped_pool(|| candidates_inner(root, entries, options))?
}

fn candidates_inner(
    root: &Path,
    entries: impl Iterator<Item = PathBuf>,
    options: &BuildOptions,
) -> eyre::Result<BTreeSet<PathBuf>> {
    let _span = tracing::info_span!("build.discover_local").entered();
    tracing::info!(target: "bundler::progress", "Discovering local source candidates");
    let progress = Arc::new(Progress::new());
    let mut visited = HashSet::new();
    let pending: Vec<_> = entries
        .map(|entry| owner(root, &entry))
        .filter(|package| visited.insert(package.clone()))
        .collect();
    let mut roots = BTreeSet::new();
    let installations = parking_lot::RwLock::new(HashMap::new());
    options.execution.replenish_stage(
        crate::execution::WorkStage::Discovery,
        pending,
        |package: PathBuf| {
            progress.package();
            let mut dependencies = Vec::new();
            if let Some(json) = manifest(&package) {
                for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
                    if let Some(declared) = json[field].as_object() {
                        for (name, version) in declared {
                            let selected =
                                installed(&package, name, &installations).or_else(|| {
                                    if field == "peerDependencies" {
                                        return None;
                                    }
                                    let value = version.as_str()?;
                                    let relative = ["file:", "link:", "portal:"]
                                        .iter()
                                        .find_map(|prefix| value.strip_prefix(prefix))?;
                                    let path = package.join(relative).canonicalize().ok()?;
                                    (local(root, &path) && path.join("package.json").is_file())
                                        .then_some(path)
                                });
                            if let Some(selected) = selected {
                                dependencies.push(selected);
                            }
                        }
                    }
                }
            }
            (package, dependencies)
        },
        |(package, dependencies)| {
            if local(root, &package) {
                roots.insert(package);
            }
            Ok(dependencies
                .into_iter()
                .filter(|package| visited.insert(package.clone()))
                .collect())
        },
    )?;
    if roots.is_empty() {
        return Ok(BTreeSet::new());
    }
    let packages = roots.len();
    let branches: HashSet<_> = roots
        .iter()
        .flat_map(|path| path.ancestors().take_while(|path| path.starts_with(root)))
        .map(Path::to_owned)
        .collect();
    // The iterator can block: never leave its only worker waiting on itself.
    let parallelism = if rayon::current_thread_index().is_some() {
        jwalk::Parallelism::Serial
    } else if let Some(pool) = crate::execution::owned_pool() {
        jwalk::Parallelism::RayonExistingPool(pool)
    } else {
        jwalk::Parallelism::Serial
    };
    let root = root.to_owned();
    let mut inherited = WalkState::default();
    for ancestor in root
        .ancestors()
        .skip(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        IgnoreFile::push(&mut inherited.ignores, ancestor, &ancestor.join(".ignore"));
        IgnoreFile::push(
            &mut inherited.gitignores,
            ancestor,
            &ancestor.join(".gitignore"),
        );
        IgnoreFile::push(
            &mut inherited.excludes,
            ancestor,
            &ancestor.join(".git").join("info").join("exclude"),
        );
    }
    let (global, _) = ignore::gitignore::GitignoreBuilder::new(&root).build_global();
    inherited.global = Some(Arc::new(IgnoreFile {
        matcher: global,
        parent: None,
    }));
    let candidates = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let discovered = candidates.clone();
    let css = options.css.clone();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let parent = tracing::Span::current();
    let walk = jwalk::WalkDirGeneric::<(WalkState, ())>::new(&root)
        .parallelism(parallelism)
        .skip_hidden(false)
        .follow_links(false)
        .process_read_dir(move |state, children| {
            tracing::dispatcher::with_default(&dispatch, || {
                let Some(first) = children.iter().flatten().next() else {
                    return;
                };
                if first.depth() == 0 {
                    *state = inherited.clone();
                    return;
                }
                let depth = first.depth() - 1;
                let _span =
                    tracing::info_span!(parent: &parent, "discovery.directory", depth).entered();
                let directory = first.parent_path.clone();
                progress.directory(&root, &directory);
                let has = |name| {
                    children
                        .iter()
                        .flatten()
                        .any(|entry| entry.file_name() == name)
                };
                state.selected =
                    roots.contains(directory.as_ref()) || (state.selected && !has("package.json"));
                if has(".ignore") {
                    IgnoreFile::push(&mut state.ignores, &directory, &directory.join(".ignore"));
                }
                if has(".git") {
                    state.gitignores = None;
                    state.excludes = None;
                    IgnoreFile::push(
                        &mut state.excludes,
                        &directory,
                        &directory.join(".git").join("info").join("exclude"),
                    );
                }
                if has(".gitignore") {
                    IgnoreFile::push(
                        &mut state.gitignores,
                        &directory,
                        &directory.join(".gitignore"),
                    );
                }
                let mut files = Vec::new();
                children.retain(|entry| {
                    let Ok(entry) = entry else { return false };
                    let name = entry.file_name().to_string_lossy();
                    if matches!(name.as_ref(), "node_modules" | ".git" | "target")
                        || name.starts_with(".bundler-")
                    {
                        return false;
                    }
                    let path = entry.path();
                    let directory = entry.file_type().is_dir();
                    if state.ignored(&path, directory) {
                        return false;
                    }
                    if directory {
                        return state.selected || branches.contains(&path);
                    }
                    if state.selected
                        && entry.file_type().is_file()
                        && crate::resource::kind(&path, &css)
                            == crate::resource::ResourceKind::Script
                        && !segment_cache::is_cache_artifact(&path)
                        && !crate::upstream::is_cache_artifact(&path)
                    {
                        // Canonical roots and a non-following walk already give canonical routes.
                        progress.file(&root, &path);
                        files.push(path);
                    }
                    false
                });
                discovered.lock().extend(files);
            });
        });
    for _ in walk {}
    let candidates: BTreeSet<_> = candidates.lock().drain(..).collect();
    tracing::info!(target: "bundler::progress", packages, candidates = candidates.len(),
        "Discovered local source candidates");
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::{layer::Context, prelude::*, Layer};

    #[derive(Default)]
    struct Fields(BTreeMap<String, String>);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.insert(field.name().into(), format!("{value:?}"));
        }
    }

    #[derive(Default)]
    struct Rendezvous {
        arrivals: parking_lot::Mutex<usize>,
        ready: parking_lot::Condvar,
        timed_out: std::sync::atomic::AtomicBool,
    }

    impl Rendezvous {
        fn meet(&self) {
            let mut arrivals = self.arrivals.lock();
            *arrivals += 1;
            self.ready.notify_all();
            if self
                .ready
                .wait_while_for(
                    &mut arrivals,
                    |arrivals| *arrivals < 2,
                    Duration::from_secs(10),
                )
                .timed_out()
            {
                self.timed_out.store(true, Ordering::Relaxed);
            }
        }
    }

    #[derive(Default)]
    struct Observed {
        package_calls: AtomicU64,
        package_threads: parking_lot::Mutex<HashSet<std::thread::ThreadId>>,
        walk_threads: parking_lot::Mutex<HashSet<std::thread::ThreadId>>,
        package: Rendezvous,
        walk: Rendezvous,
        events: parking_lot::Mutex<Vec<Fields>>,
    }

    struct Observer(Arc<Observed>);

    impl<S: tracing::Subscriber> Layer<S> for Observer {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _: &tracing::Id,
            _: Context<'_, S>,
        ) {
            let thread = std::thread::current().id();
            match attrs.metadata().name() {
                "discovery.package" => {
                    self.0.package_threads.lock().insert(thread);
                    if matches!(self.0.package_calls.fetch_add(1, Ordering::Relaxed), 1 | 2) {
                        self.0.package.meet();
                    }
                }
                "discovery.directory" => {
                    self.0.walk_threads.lock().insert(thread);
                    let mut fields = Fields::default();
                    attrs.record(&mut fields);
                    if fields.0.get("depth").is_some_and(|depth| depth == "1") {
                        self.0.walk.meet();
                    }
                }
                _ => {}
            }
        }

        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            if event.metadata().target() == "bundler::progress::discovery" {
                let mut fields = Fields::default();
                event.record(&mut fields);
                self.0.events.lock().push(fields);
            }
        }
    }

    #[test]
    fn package_probes_and_directory_reads_are_concurrent_and_bounded() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r#"{"dependencies":{"a":"file:a","b":"file:b"}}"#,
            "main.mjs" => "",
            "a/package.json" => "{}",
            "a/source.mjs" => "",
            "b/package.json" => "{}",
            "b/source.mjs" => ""
        };
        let root = dir.root().canonicalize().unwrap();
        let mut options = BuildOptions::default();
        options.execution =
            crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(2).unwrap());
        // Other tests can first register walk callsites without a subscriber.
        // Keep tracing's single-dispatch shortcut from caching them as disabled.
        let _other_dispatch = tracing::Dispatch::new(tracing_subscriber::registry());
        for shared in [false, true] {
            let observed = Arc::new(Observed::default());
            let subscriber = tracing_subscriber::registry().with(Observer(observed.clone()));
            let discover =
                || candidates(&root, std::iter::once(root.join("main.mjs")), &options).unwrap();
            let found = tracing::subscriber::with_default(subscriber, || {
                if shared {
                    options.execution.scoped_pool(discover).unwrap()
                } else {
                    discover()
                }
            });
            assert_eq!(found.len(), 3);
            assert!(!observed.package.timed_out.load(Ordering::Relaxed));
            assert!(!observed.walk.timed_out.load(Ordering::Relaxed));
            let package_threads = observed.package_threads.lock();
            let walk_threads = observed.walk_threads.lock();
            assert_eq!(package_threads.len(), 2, "{package_threads:?}");
            assert_eq!(*package_threads, *walk_threads);
            assert!(!walk_threads.contains(&std::thread::current().id()));
            let events = observed.events.lock();
            assert!(events.iter().any(|event| event.0.contains_key("directory")));
            assert!(events.iter().any(|event| event.0.contains_key("file")));
        }
    }

    #[test]
    fn directory_walk_reuses_installed_caller_pools_including_one_worker() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => "{}",
            "main.mjs" => "",
            "src/other.mjs" => ""
        };
        let root = dir.root().canonicalize().unwrap();
        for workers in [1, 2] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            pool.install(|| {
                let mut options = BuildOptions::default();
                options.execution = crate::execution::ExecutionOptions::new(
                    std::num::NonZeroUsize::new(workers).unwrap(),
                );
                let found =
                    candidates(&root, std::iter::once(root.join("main.mjs")), &options).unwrap();
                assert_eq!(found.len(), 2);
                assert!(crate::execution::owned_pool().is_none());
            });
        }
    }

    #[test]
    fn traversal_progress_samples_files_and_directories_instead_of_flooding() {
        let observed = Arc::new(Observed::default());
        let subscriber = tracing_subscriber::registry().with(Observer(observed.clone()));
        tracing::subscriber::with_default(subscriber, || {
            let mut progress = Progress::new();
            progress.start = Instant::now() + Duration::from_secs(60);
            for _ in 0..1_000 {
                progress.directory(Path::new("root"), Path::new("root").join("src").as_path());
                progress.file(
                    Path::new("root"),
                    Path::new("root").join("entry.mjs").as_path(),
                );
            }
            assert_eq!(observed.events.lock().len(), 2);
            progress.start = Instant::now() - Duration::from_secs(1);
            progress.directory(Path::new("root"), &Path::new("root").join("next"));
            progress.file(Path::new("root"), &Path::new("root").join("next.mjs"));
        });
        let events = observed.events.lock();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].0["directory"], format!("{:?}", Path::new("src")));
        assert_eq!(events[1].0["file"], format!("{:?}", Path::new("entry.mjs")));
        assert_eq!(events[2].0["directories"], "1001");
        assert_eq!(events[3].0["files"], "1001");
    }

    #[test]
    fn selected_subtrees_keep_inherited_nested_and_hidden_ignore_semantics() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => "{}",
            ".gitignore" => "generated/\n*.skip.mjs\n",
            ".ignore" => "forced.mjs\n",
            ".git/info/exclude" => "excluded.mjs\n",
            "unselected/source.mjs" => "",
            "packages/app/package.json" => "{}",
            "packages/app/main.mjs" => "",
            "packages/app/.hidden/allowed.mjs" => "",
            "packages/app/generated/never.mjs" => "",
            "packages/app/src/.gitignore" => "!keep.skip.mjs\n!forced.mjs\n",
            "packages/app/src/keep.skip.mjs" => "",
            "packages/app/src/drop.skip.mjs" => "",
            "packages/app/src/forced.mjs" => "",
            "packages/app/src/excluded.mjs" => ""
        };
        let root = dir.root().canonicalize().unwrap();
        let found = candidates(
            &root,
            std::iter::once(root.join("packages").join("app").join("main.mjs")),
            &BuildOptions::default(),
        )
        .unwrap();
        let expected: BTreeSet<_> = [
            Path::new("packages").join("app").join("main.mjs"),
            Path::new("packages")
                .join("app")
                .join(".hidden")
                .join("allowed.mjs"),
            Path::new("packages")
                .join("app")
                .join("src")
                .join("keep.skip.mjs"),
        ]
        .into_iter()
        .map(|path| root.join(path))
        .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn multi_root_walk_follows_runtime_metadata_but_not_dev_or_external_sources() {
        let dir = test_tmpdir::test_tmpdir! {
            "package.json" => r#"{
                "dependencies":{"a":"file:packages/a","provider":"1"},
                "optionalDependencies":{"opt":"file:packages/opt"},
                "peerDependencies":{"missing":"*"},
                "devDependencies":{"dev":"file:packages/dev"}
            }"#,
            "main.mjs" => "",
            ".gitignore" => "generated/",
            "generated/ignored.mjs" => "",
            "packages/a/package.json" => r#"{"dependencies":{"root":"file:../.."}}"#,
            "packages/a/a.ts" => "",
            "packages/b/package.json" => "{}",
            "packages/b/b.mjs" => "",
            "packages/opt/package.json" => "{}",
            "packages/opt/opt.mjs" => "",
            "packages/dev/package.json" => "{}",
            "packages/dev/dev.mjs" => "",
            "packages/unselected/package.json" => "{}",
            "packages/unselected/other.mjs" => "",
            "node_modules/provider/package.json" => r#"{"dependencies":{"b":"file:../../packages/b"}}"#,
            "node_modules/provider/never.mjs" => "",
            "target/never.mjs" => "",
            ".bundler-stage/never.mjs" => ""
        };
        let root = dir.root().canonicalize().unwrap();
        let found = candidates(
            &root,
            std::iter::once(root.join("main.mjs")),
            &BuildOptions::default(),
        )
        .unwrap();
        let paths: Vec<_> = found
            .iter()
            .map(|path| path.strip_prefix(&root).unwrap().to_owned())
            .collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("main.mjs"),
                PathBuf::from("packages").join("a").join("a.ts"),
                PathBuf::from("packages").join("b").join("b.mjs"),
                PathBuf::from("packages").join("opt").join("opt.mjs"),
            ]
        );
    }
}
