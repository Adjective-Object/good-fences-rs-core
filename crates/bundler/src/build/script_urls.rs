//! Literal JS resources are independent classic-script realms, never parent imports.
use super::*;

fn script_options(options: &BuildOptions, inventory: &Inventory, target: &Path) -> BuildOptions {
    let mut script = options.clone();
    script.entries = BTreeMap::from([(
        "script".into(),
        Entry {
            // A fresh child must re-admit the checked project route, not inherit
            // authority over the parent's canonical external destination.
            path: inventory.relative(inventory.root(), target),
            output_name: Some(output_path(target)),
            namespace: false,
        },
    )]);
    script.output.synthesize_async_boundaries = false;
    script.output.lazy_runtimes.clear();
    script
}

fn output_path(target: &Path) -> PathBuf {
    let digest: String = hash(target.to_string_lossy().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    PathBuf::from(format!("scripts/{digest}.js"))
}

fn count(counters: &mut Counters, child: &Counters) {
    counters.source_parses += child.source_parses;
    counters.speculative_parses += child.speculative_parses;
    counters.memory_hits += child.memory_hits;
    counters.disk_hits += child.disk_hits;
    counters.resolutions += child.resolutions;
    counters.resolution_cache_hits += child.resolution_cache_hits;
    counters.resolution_cache_misses += child.resolution_cache_misses;
    counters.inventory_directory_reads += child.inventory_directory_reads;
}

pub(super) fn prepare(
    root: &Path,
    inventory: &Inventory,
    sources: &BTreeMap<PathBuf, Source>,
    resolutions: &BTreeMap<PathBuf, HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>>,
    sessions: &mut BTreeMap<PathBuf, Box<BuildSession>>,
    options: &BuildOptions,
    classic: bool,
    relocations: &mut BTreeMap<PathBuf, Vec<crate::css_loading::UrlRelocation>>,
    counters: &mut Counters,
    warnings: &mut Vec<String>,
) -> eyre::Result<BTreeMap<PathBuf, BuildResult>> {
    let mut targets = BTreeSet::new();
    for (path, source) in sources {
        for dependency in source
            .record
            .module()
            .facts()
            .dependencies()
            .iter()
            .filter(|dependency| dependency.kind() == DependencyKind::Url)
        {
            ensure!(!classic && options.resolver.target_env == TargetEnv::Browser,
                "{}: resource URLs require a browser module, not a classic script or Node target at bytes {}..{}",
                path.display(), dependency.span().start, dependency.span().end);
            let Some(DependencyTarget::File(target)) = resolutions[path].get(&dependency.id())
            else {
                eyre::bail!(
                    "{}: resource URL must resolve to a checked local file at bytes {}..{}",
                    path.display(),
                    dependency.span().start,
                    dependency.span().end
                );
            };
            ensure!(
                crate::resource::kind(target, &options.css)
                    == crate::resource::ResourceKind::Script,
                "{}: resource URL supports JavaScript classic-script targets only: {}",
                path.display(),
                target.display()
            );
            targets.insert(target.clone());
            relocations.entry(path.clone()).or_default().push(
                crate::css_loading::UrlRelocation::Script {
                    dependency: dependency.id(),
                    target: output_path(target),
                    base: options.output.base_url.clone(),
                },
            );
        }
    }
    sessions.retain(|path, _| targets.contains(path));
    let mut scripts = BTreeMap::new();
    for target in targets {
        if !sessions.contains_key(&target) {
            sessions.insert(target.clone(), Box::new(BuildSession::new(root)?));
        }
        let built = sessions
            .get_mut(&target)
            .expect("script session")
            .build_classic_script(&script_options(options, inventory, &target))
            .wrap_err_with(|| {
                format!("building URL classic-script resource {}", target.display())
            })?;
        count(counters, &built.counters);
        warnings.extend(built.warnings.iter().cloned());
        scripts.insert(target, built);
    }
    Ok(scripts)
}

pub(super) fn recheck(
    inventory: &Inventory,
    sessions: &mut BTreeMap<PathBuf, Box<BuildSession>>,
    scripts: &BTreeMap<PathBuf, BuildResult>,
    options: &BuildOptions,
    counters: &mut Counters,
) -> eyre::Result<()> {
    for (target, before) in scripts {
        let after = sessions
            .get_mut(target)
            .expect("script session")
            .build_classic_script(&script_options(options, inventory, target))
            .wrap_err_with(|| {
                format!(
                    "rechecking URL classic-script resource {}",
                    target.display()
                )
            })?;
        count(counters, &after.counters);
        ensure!(
            after.product.fingerprints == before.product.fingerprints,
            "URL classic-script resource changed during build: {}",
            target.display()
        );
    }
    Ok(())
}

pub(super) fn merge(
    artifacts: &mut Artifacts,
    scripts: &BTreeMap<PathBuf, BuildResult>,
) -> eyre::Result<()> {
    let mut names: BTreeSet<String> = artifacts
        .file_bytes()
        .map(|(path, _)| path.to_string_lossy().to_lowercase())
        .collect();
    for script in scripts.values() {
        let child = &script.product.artifacts;
        for (path, _) in child.file_bytes() {
            let name = path.to_string_lossy().to_lowercase();
            ensure!(
                !names.iter().any(|existing| existing == &name
                    || existing.starts_with(&(name.clone() + "/"))
                    || name.starts_with(&(existing.clone() + "/"))),
                "URL classic-script artifact collision: {}",
                path.display()
            );
            names.insert(name);
        }
        artifacts.files.extend(child.files.clone());
        artifacts.resources.extend(child.resources.clone());
        artifacts
            .classic_scripts
            .extend(child.classic_scripts.iter().cloned());
    }
    Ok(())
}
