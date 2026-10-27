use super::*;

pub(super) fn target<'a>(
    path: &Path,
    dependency: ast_segmenter::views::Dependency<'_>,
    resolved: Option<&'a DependencyTarget>,
    options: &BuildOptions,
) -> eyre::Result<&'a PathBuf> {
    ensure!(
        options.resolver.target_env == TargetEnv::Browser,
        "{}: module worker URLs require the browser target at bytes {}..{}",
        path.display(),
        dependency.span().start,
        dependency.span().end
    );
    let Some(DependencyTarget::File(target)) = resolved else {
        eyre::bail!(
            "{}: module worker URL must resolve to a checked local file at bytes {}..{}",
            path.display(),
            dependency.span().start,
            dependency.span().end
        );
    };
    ensure!(
        crate::resource::kind(target, &options.loaders) == crate::resource::ResourceKind::Script,
        "{}: module worker URL requires a script target: {}",
        path.display(),
        target.display()
    );
    Ok(target)
}

pub(super) fn discover(
    path: &Path,
    source: &Source,
    resolved: &HashMap<ast_segmenter::owned::DependencyId, DependencyTarget>,
    inventory: &Inventory,
    options: &BuildOptions,
    entries: &mut BTreeMap<String, (PathBuf, bool)>,
    output_entries: &mut BTreeMap<String, Entry>,
) -> eyre::Result<Vec<crate::css_loading::UrlRelocation>> {
    let mut relocations = Vec::new();
    for dependency in source
        .record
        .module()
        .facts()
        .dependencies()
        .iter()
        .filter(|dependency| dependency.kind() == DependencyKind::Url)
    {
        let target = target(path, dependency, resolved.get(&dependency.id()), options)?;
        let name = if let Some((name, _)) = entries.iter().find(|(_, (entry, _))| entry == target) {
            name.clone()
        } else {
            let digest: String = hash(target.to_string_lossy().as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let name = format!("worker:{digest}");
            ensure!(
                !entries.contains_key(&name),
                "discovered worker entry name collision: {name}"
            );
            entries.insert(name.clone(), (target.clone(), false));
            output_entries.insert(
                name.clone(),
                Entry {
                    path: inventory.relative(inventory.root(), target),
                    output_name: Some(PathBuf::from(format!("workers/{digest}.mjs"))),
                    namespace: false,
                },
            );
            name
        };
        relocations.push(crate::css_loading::UrlRelocation::Worker {
            dependency: dependency.id(),
            entry: name,
            base: options.output.base_url.clone(),
        });
    }
    Ok(relocations)
}
