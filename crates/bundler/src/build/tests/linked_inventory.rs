use super::*;

fn directory_link(target: &Path, alias: &Path) {
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, alias).unwrap();
    #[cfg(windows)]
    {
        // Junctions do not require the symlink privilege or Developer Mode.
        let ordinary = |path: &Path| {
            path.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_owned()
        };
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(ordinary(alias))
            .arg(ordinary(target))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

fn remove_directory_link(alias: &Path) {
    #[cfg(unix)]
    fs::remove_file(alias).unwrap();
    #[cfg(windows)]
    fs::remove_dir(alias).unwrap();
}

fn linked_options() -> BuildOptions {
    let mut config = options();
    config.entries.get_mut("main").unwrap().output_name = Some("main.mjs".into());
    config
}

#[test]
fn persistent_resolution_requests_with_external_links_keep_fresh_scoped_fallbacks() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(&root, "main.mjs", "export {value} from 'pkg';");
    write(
        &package,
        "package.json",
        r#"{"name":"pkg","exports":"./index.mjs"}"#,
    );
    write(&package, "index.mjs", "export {value} from './value.mjs';");
    write(&package, "value.mjs", "export const value = 1;");
    directory_link(&package, &root.join("node_modules/pkg"));
    let config = linked_options();
    let first = BuildSession::new(&root).unwrap().build(&config).unwrap();
    let warm = BuildSession::new(&root).unwrap().build(&config).unwrap();
    assert_eq!(warm.product.artifacts, first.product.artifacts);
    assert!(warm.counters.resolutions < first.counters.resolutions);
    assert!(warm
        .warnings
        .iter()
        .any(|w| w.contains("uncacheable external path")));
    write(&package, "other.mjs", "export const value = 2;");
    write(
        &package,
        "package.json",
        r#"{"name":"pkg","exports":"./other.mjs"}"#,
    );
    let changed = BuildSession::new(&root).unwrap().build(&config).unwrap();
    assert_ne!(changed.product.artifacts, warm.product.artifacts);
    remove_directory_link(&root.join("node_modules/pkg"));
}

#[test]
fn source_maps_in_checked_external_packages_reuse_borrowed_sidecars() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(
        &root,
        "main.mjs",
        "import {value} from './linked/value.mjs'; console.log(value);",
    );
    write(
        &package,
        "value.mjs",
        "export const value = 1;\n//# sourceMappingURL=value.mjs.map",
    );
    write(
        &package,
        "value.mjs.map",
        r#"{"version":3,"sources":["original.ts"],"sourcesContent":["export const value = 1;"],"names":[],"mappings":"AAAA"}"#,
    );
    directory_link(&package, &root.join("linked"));
    let mut config = linked_options();
    config.output.source_maps = true;
    let mut session = BuildSession::new(&root).unwrap();
    let first = session.build(&config).unwrap();
    assert!(first.product.artifacts.files.iter().any(|(path, text)| {
        path.extension().is_some_and(|extension| extension == "map") && text.contains("original.ts")
    }));
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    let disk = BuildSession::new(&root).unwrap().build(&config).unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.product.artifacts, first.product.artifacts);
    remove_directory_link(&root.join("linked"));
}

#[test]
fn directory_symlink_inside_root_and_self_cycle_are_supported() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "import {value as a} from './real/value.mjs'; import {value as b} from './alias/value.mjs'; console.log(a === b);");
    write(dir.root(), "real/value.mjs", "export const value = {};");
    let link = |target: &Path, alias: &Path| {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target, alias).unwrap();
    };
    link(&dir.root().join("real"), &dir.root().join("alias"));
    link(&dir.root().join("real"), &dir.root().join("real/cycle"));
    let mut session = BuildSession::new(dir.root()).unwrap();
    let built = fresh_parity(&mut session, &linked_options());
    assert_eq!(session.sources.len(), 2);
    assert_eq!(observe(&built.product.artifacts), b"true\n");
    remove_directory_link(&dir.root().join("alias"));
    remove_directory_link(&dir.root().join("real/cycle"));
}

fn fresh_parity(session: &mut BuildSession, config: &BuildOptions) -> BuildResult {
    let warm = session.build(config).unwrap();
    let mut fresh_config = config.clone();
    fresh_config.cache = CacheMode::Disabled;
    let fresh = BuildSession::new(session.root())
        .unwrap()
        .build(&fresh_config)
        .unwrap();
    assert_eq!(warm.product.fingerprints, fresh.product.fingerprints);
    assert_eq!(warm.product.artifacts, fresh.product.artifacts);
    warm
}

#[test]
fn aliases_and_directory_cycles_share_one_canonical_module() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(
        &package,
        "package.json",
        r#"{"type":"module","exports":"./index.js"}"#,
    );
    write(&package, "index.js", "export const value = {};");
    write(
        &root,
        "main.mjs",
        "import {value as a} from 'a'; import {value as b} from 'b'; console.log(a === b);",
    );
    directory_link(&package, &root.join("node_modules/a"));
    directory_link(&package, &root.join("node_modules/b"));
    directory_link(&root, &package.join("cycle"));
    let mut session = BuildSession::new(&root).unwrap();
    let built = fresh_parity(&mut session, &linked_options());
    assert_eq!(session.sources.len(), 2);
    assert!(built
        .product
        .graph
        .file_id(&package.join("index.js"))
        .is_some());
    assert!(built
        .product
        .graph
        .file_id(&root.join("node_modules/a/index.js"))
        .is_none());
    assert_eq!(observe(&built.product.artifacts), b"true\n");
    assert!(Rc::ptr_eq(
        &built.product,
        &session.build(&linked_options()).unwrap().product
    ));
    // Remove actual junctions explicitly before the managed directory teardown.
    remove_directory_link(&package.join("cycle"));
    remove_directory_link(&root.join("node_modules/a"));
    remove_directory_link(&root.join("node_modules/b"));
}

#[test]
fn linked_packages_ignore_reserved_names_above_inventory_roots() {
    let dir = TmpDir::new();
    for name in ["target", ".git", ".bundler-out"] {
        let root = dir.root().join(name).join("project");
        let package = dir.root().join(name).join("packages/package");
        write(
            &root,
            "main.mjs",
            "import {value} from './linked/value.mjs'; console.log(value);",
        );
        write(&package, "value.mjs", "export const value = 42;");
        let alias = root.join("linked");
        directory_link(&package, &alias);
        let built = fresh_parity(&mut BuildSession::new(&root).unwrap(), &linked_options());
        assert_eq!(observe(&built.product.artifacts), b"42\n");
        remove_directory_link(&alias);
    }
}

#[test]
fn linked_aliases_cannot_bypass_reserved_inventory_subdirectories() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(&root, "main.mjs", "");
    write(&package, "value.mjs", "");
    let alias = root.join("linked");
    directory_link(&package, &alias);
    for scope in [&root, &package] {
        for name in ["target", ".git", ".bundler-out"] {
            let target = scope.join(name).join("nested");
            fs::create_dir_all(&target).unwrap();
            let bypass = scope.join("bypass");
            directory_link(&target, &bypass);
            let error = format!("{:#}", inventory(&root).err().unwrap());
            assert!(error.contains("reserved/excluded"), "{error}");
            remove_directory_link(&bypass);
        }
    }
    remove_directory_link(&alias);
}

#[test]
fn cross_tree_aliases_cannot_bypass_reserved_descendants_in_either_order() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("target/packages/package");
    write(&root, "main.mjs", "");
    write(&package, "value.mjs", "");
    let alias = root.join("package");
    directory_link(&package, &alias);
    for alias_name in ["a-bypass", "z-bypass"] {
        for name in ["target", ".git", ".bundler-out"] {
            let target = package.join(name).join("nested");
            write(&target, "value.mjs", "");
            let bypass = root.join(alias_name);
            directory_link(&target, &bypass);
            let result = inventory(&root);
            remove_directory_link(&bypass);
            let error = format!("{:#}", result.unwrap_err());
            assert!(error.contains("reserved/excluded"), "{alias_name}: {error}");
        }
    }
    remove_directory_link(&alias);
}

#[test]
fn retarget_source_metadata_and_membership_edits_invalidate() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    for (name, value) in [("one", 1), ("two", 2)] {
        write(
            dir.root(),
            &format!("{name}/package.json"),
            r#"{"type":"module","exports":"./index.js"}"#,
        );
        write(
            dir.root(),
            &format!("{name}/index.js"),
            &format!("export default {value};"),
        );
        write(dir.root(), &format!("{name}/other.js"), "export default 3;");
    }
    write(
        &root,
        "main.mjs",
        "import value from 'pkg'; console.log(value);",
    );
    let alias = root.join("node_modules/pkg");
    directory_link(&dir.root().join("one"), &alias);
    let config = linked_options();
    let mut session = BuildSession::new(&root).unwrap();
    let first = fresh_parity(&mut session, &config);
    let before = inventory(&root).unwrap();
    remove_directory_link(&alias);
    directory_link(&dir.root().join("two"), &alias);
    assert!(BuildSession::recheck(&root, &session.sources, &before).is_err());
    let second = fresh_parity(&mut session, &config);
    assert_ne!(
        first.product.fingerprints.resolution,
        second.product.fingerprints.resolution
    );
    assert!(session
        .sources
        .contains_key(&dir.root().join("two/index.js")));
    assert!(!session
        .sources
        .contains_key(&dir.root().join("one/index.js")));
    assert_eq!(observe(&second.product.artifacts), b"2\n");
    write(dir.root(), "two/index.js", "export default 4;");
    let edited = fresh_parity(&mut session, &config);
    assert_eq!(observe(&edited.product.artifacts), b"4\n");
    write(
        dir.root(),
        "two/package.json",
        r#"{"type":"module","exports":"./other.js"}"#,
    );
    let metadata = fresh_parity(&mut session, &config);
    assert_eq!(observe(&metadata.product.artifacts), b"3\n");
    write(dir.root(), "two/new.mjs", "");
    let added = fresh_parity(&mut session, &config);
    assert_eq!(
        metadata.product.fingerprints.resolution,
        added.product.fingerprints.resolution
    );
    assert!(Rc::ptr_eq(&metadata.product, &added.product));
    fs::remove_file(dir.root().join("two/new.mjs")).unwrap();
    let removed = fresh_parity(&mut session, &config);
    assert_eq!(
        metadata.product.fingerprints.resolution,
        removed.product.fingerprints.resolution
    );
    fs::remove_file(dir.root().join("two/other.js")).unwrap();
    assert!(session.build(&config).is_err());
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &removed.product));
    remove_directory_link(&alias);
}

#[test]
fn external_ancestor_configs_are_tracked_but_conditions_remain_explicit() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("workspace/package");
    write(&root, "main.mjs", "export {default} from 'pkg';");
    write(
        &package,
        "package.json",
        r#"{"type":"module","exports":"./index.js"}"#,
    );
    write(&package, "index.js", "export {default} from 'dep';");
    write(
        &package,
        "node_modules/dep/package.json",
        r#"{"exports":{".":{"source":"./source.mjs","default":"./default.mjs"}}}"#,
    );
    write(&package, "node_modules/dep/source.mjs", "export default 1;");
    write(
        &package,
        "node_modules/dep/default.mjs",
        "export default 2;",
    );
    write(dir.root(), "workspace/unrelated/invalid.json", "unrelated");
    directory_link(&package, &root.join("node_modules/pkg"));
    let mut config = linked_options();
    let mut session = BuildSession::new(&root).unwrap();
    let default = fresh_parity(&mut session, &config);
    assert!(session
        .sources
        .contains_key(&package.join("node_modules/dep/default.mjs")));
    write(
        dir.root(),
        "workspace/tsconfig.json",
        r#"{"compilerOptions":{"customConditions":["source"]}}"#,
    );
    let compiler_only = fresh_parity(&mut session, &config);
    assert_eq!(
        default.product.fingerprints.resolution,
        compiler_only.product.fingerprints.resolution
    );
    assert!(Rc::ptr_eq(&default.product, &compiler_only.product));
    assert!(session
        .sources
        .contains_key(&package.join("node_modules/dep/default.mjs")));
    config.resolver.export_conditions.push("source".into());
    fresh_parity(&mut session, &config);
    assert!(session
        .sources
        .contains_key(&package.join("node_modules/dep/source.mjs")));
    let before = inventory(&root).unwrap();
    write(dir.root(), "workspace/unrelated/new.mjs", "");
    assert_eq!(before, inventory(&root).unwrap());
    write(
        dir.root(),
        "workspace/tsconfig.json",
        r#"{"compilerOptions":{}}"#,
    );
    assert!(BuildSession::recheck(&root, &session.sources, &before).is_err());
    config
        .resolver
        .export_conditions
        .retain(|condition| condition != "source");
    fresh_parity(&mut session, &config);
    assert!(session
        .sources
        .contains_key(&package.join("node_modules/dep/default.mjs")));
    remove_directory_link(&root.join("node_modules/pkg"));
}

#[test]
fn unenumerated_ancestor_package_shadowing_never_reuses_stale_success() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("workspace/packages/pkg");
    let dependency = dir.root().join("workspace/node_modules/dep");
    write(&root, "main.mjs", "export {default} from 'pkg';");
    write(&package, "package.json", r#"{"exports":"./index.mjs"}"#);
    write(&package, "index.mjs", "export {default} from 'dep';");
    write(&dependency, "package.json", r#"{"exports":"./index.mjs"}"#);
    write(&dependency, "index.mjs", "export default 1;");
    directory_link(&package, &root.join("node_modules/pkg"));
    directory_link(&dependency, &root.join("node_modules/dep"));
    let config = linked_options();
    let mut session = BuildSession::new(&root).unwrap();
    let first = fresh_parity(&mut session, &config);
    let checked = resolver::BuildResolver::new(
        &root,
        &config,
        resolver::Lockfile::read(&root, &mut Vec::new()).unwrap(),
        resolver::Memory::default(),
    );
    checked
        .resolve_dependency(&package.join("index.mjs"), "dep", ResolutionMode::Import)
        .unwrap();
    let before = inventory(&root).unwrap();
    let shadow = dir.root().join("workspace/packages/node_modules/dep");
    write(&shadow, "package.json", r#"{"exports":"./index.mjs"}"#);
    write(&shadow, "index.mjs", "export default 2;");
    // No broad sibling-tree inventory was introduced just to detect this probe.
    assert_eq!(before, inventory(&root).unwrap());
    assert!(checked.recheck().is_err());
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("outside reachable build inventory"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    remove_directory_link(&root.join("node_modules/pkg"));
    remove_directory_link(&root.join("node_modules/dep"));
}

#[test]
fn contexts_include_linked_directory_aliases_and_new_members() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(&root, "main.mjs", "export {load} from 'pkg';");
    write(&package, "package.json", r#"{"exports":"./index.mjs"}"#);
    write(
        &package,
        "index.mjs",
        "export const load = name => import(`./pages/${name}.mjs`);",
    );
    write(&package, "actual/one.mjs", "export default 1;");
    directory_link(&package.join("actual"), &package.join("pages"));
    directory_link(&package, &root.join("node_modules/pkg"));
    let mut session = BuildSession::new(&root).unwrap();
    let config = linked_options();
    fresh_parity(&mut session, &config);
    assert!(session
        .sources
        .contains_key(&package.join("actual/one.mjs")));
    write(&package, "actual/two.mjs", "export default 2;");
    fresh_parity(&mut session, &config);
    assert!(session
        .sources
        .contains_key(&package.join("actual/two.mjs")));
    fs::remove_file(package.join("actual/one.mjs")).unwrap();
    fresh_parity(&mut session, &config);
    assert!(!session
        .sources
        .contains_key(&package.join("actual/one.mjs")));
    remove_directory_link(&package.join("pages"));
    remove_directory_link(&root.join("node_modules/pkg"));
}

#[test]
fn diamond_inventory_scans_canonical_directories_once_and_expands_only_context_prefix() {
    use ast_segmenter::owned::Dependency;

    let dir = TmpDir::new();
    let directories: Vec<_> = (0..=20)
        .map(|index| dir.root().join(format!("level{index}")))
        .collect();
    for directory in &directories {
        fs::create_dir(directory).unwrap();
    }
    let root = &directories[0];
    write(root, "main.mjs", "");
    write(&directories[20], "one.mjs", "export default 1;");
    for pair in directories.windows(2) {
        for alias in ["a", "b"] {
            directory_link(&pair[1], &pair[0].join(alias));
        }
    }
    let inventory = inventory(root).unwrap();
    assert_eq!(inventory.directory_reads(), 21);
    let prefix = format!("./{}", "a/".repeat(19));
    let mut dependency: Dependency = serde_json::from_value(serde_json::json!({
        "id": 0, "kind": "DynamicImport", "span": [0, 0], "specifier": "",
        "pattern": [prefix.clone(), "/one.mjs"], "specifier_span": [0, 0],
        "type_only": false, "enclosing_function": null
    }))
    .unwrap();
    let importer = root.join("main.mjs");
    let (candidates, visits) = inventory.context_candidates(root, &importer, (&dependency).into());
    // Nineteen fixed edges and only the final diamond are expanded.
    assert_eq!(visits, 22);
    let config = linked_options();
    let resolver = resolver::BuildResolver::new(
        root,
        &config,
        resolver::Lockfile::read(root, &mut Vec::new()).unwrap(),
        resolver::Memory::default(),
    );
    let resolved = crate::context::resolve(
        root,
        &importer,
        (&dependency).into(),
        candidates.into_iter(),
        &config.resolver.extensions,
        &resolver,
    )
    .unwrap();
    assert_eq!(
        resolved,
        BTreeMap::from([
            (
                format!("{prefix}a/one.mjs"),
                directories[20].join("one.mjs")
            ),
            (
                format!("{prefix}b/one.mjs"),
                directories[20].join("one.mjs")
            ),
        ])
    );
    directory_link(root, &directories[20].join("cycle"));
    let cycled = inventory::inventory(root).unwrap();
    assert_ne!(inventory, cycled);
    assert_eq!(cycled.directory_reads(), 21);
    let mut value = serde_json::to_value(&dependency).unwrap();
    value["pattern"] = serde_json::json!(["./cycle/", ".mjs"]);
    dependency = serde_json::from_value(value).unwrap();
    let external_importer = directories[20].join("one.mjs");
    let resolved = crate::context::resolve(
        root,
        &external_importer,
        (&dependency).into(),
        cycled.candidates(root, &external_importer, (&dependency).into()),
        &config.resolver.extensions,
        &resolver,
    )
    .unwrap();
    assert!(
        resolved.is_empty(),
        "physical projections must not reopen root cycles"
    );
    remove_directory_link(&directories[20].join("cycle"));
    for pair in directories.windows(2) {
        for alias in ["a", "b"] {
            remove_directory_link(&pair[0].join(alias));
        }
    }
}

#[cfg(windows)]
#[test]
fn context_cross_volume_candidates_keep_same_volume_lexical_aliases() {
    use ast_segmenter::owned::Dependency;
    use import_resolver::structural::{DependencyResolver, ResolvedDependency};

    struct Resolver;
    impl DependencyResolver for Resolver {
        fn resolve_dependency(
            &self,
            _: &Path,
            request: &str,
            _: ResolutionMode,
        ) -> eyre::Result<ResolvedDependency> {
            assert_eq!(request, "./linked/one.mjs");
            Ok(ResolvedDependency::File(r"\\?\D:\package\one.mjs".into()))
        }
    }
    for (root, other) in [
        (r"C:\project", r"D:\package\one.mjs"),
        (r"\\?\C:\project", r"\\?\D:\package\one.mjs"),
        (r"\\server\share\project", r"\\server\other\one.mjs"),
    ] {
        let root = Path::new(root);
        let importer = root.join("main.mjs");
        for kind in [DependencyKind::DynamicImport, DependencyKind::Require] {
            let dependency: Dependency = serde_json::from_value(serde_json::json!({
                "id": 0, "kind": kind, "span": [0, 0], "specifier": "",
                "pattern": ["./linked/", ".mjs"], "specifier_span": [0, 0],
                "type_only": false, "enclosing_function": null
            }))
            .unwrap();
            let resolved = crate::context::resolve(
                root,
                &importer,
                (&dependency).into(),
                [PathBuf::from(other), root.join("linked").join("one.mjs")].into_iter(),
                &[],
                &Resolver,
            )
            .unwrap();
            assert_eq!(resolved.len(), 1);
            assert_eq!(
                resolved["./linked/one.mjs"],
                Path::new(r"\\?\D:\package\one.mjs")
            );
        }
    }
}

#[test]
fn missing_targets_and_unlinked_root_escape_are_explicit_errors() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    write(&root, "main.mjs", "import '../outside.mjs';");
    write(dir.root(), "outside.mjs", "");
    let mut session = BuildSession::new(&root).unwrap();
    let error = format!("{:#}", session.build(&linked_options()).err().unwrap());
    assert!(
        error.contains("outside reachable build inventory"),
        "{error}"
    );
    let target = dir.root().join("missing");
    fs::create_dir(&target).unwrap();
    let alias = root.join("broken");
    directory_link(&target, &alias);
    fs::remove_dir(&target).unwrap();
    let error = format!("{:#}", inventory(&root).err().unwrap());
    assert!(
        error.contains("inventory target") && error.contains("broken"),
        "{error}"
    );
    remove_directory_link(&alias);
}

#[test]
fn default_entry_outputs_follow_in_root_canonical_links() {
    let dir = TmpDir::new();
    write(dir.root(), "source/main.mjs", "console.log('canonical');");
    let alias = dir.root().join("alias");
    directory_link(&dir.root().join("source"), &alias);
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = alias.join("main.mjs");
    let built = fresh_parity(&mut BuildSession::new(dir.root()).unwrap(), &config);
    assert_eq!(
        built.product.artifacts.entries["main"].to_str(),
        Some("source/main.mjs")
    );
    assert_eq!(observe(&built.product.artifacts), b"canonical\n");
    remove_directory_link(&alias);
}

#[test]
fn linked_entry_requires_explicit_safe_output_name() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    write(dir.root(), "package/main.mjs", "console.log('linked');");
    fs::create_dir(&root).unwrap();
    let alias = root.join("linked");
    directory_link(&dir.root().join("package"), &alias);
    let mut config = linked_options();
    config.entries.get_mut("main").unwrap().path = "linked/main.mjs".into();
    let mut session = BuildSession::new(&root).unwrap();
    let built = session.build(&config).unwrap();
    assert_eq!(observe(&built.product.artifacts), b"linked\n");
    config.entries.get_mut("main").unwrap().output_name = None;
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("requires an explicit output_name"),
        "{error}"
    );
    config.entries.get_mut("main").unwrap().output_name = Some("../escape.mjs".into());
    assert!(session.build(&config).is_err());
    remove_directory_link(&alias);
}

#[test]
fn file_symlinks_share_identity_and_track_config_targets() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    write(&root, "main.mjs", "import {value as a} from './value.mjs'; import {value as b} from './alias.mjs'; console.log(a === b);");
    write(&root, "value.mjs", "export const value = {};");
    write(dir.root(), "config.json", r#"{"compilerOptions":{}}"#);
    let link = |target: &Path, alias: &Path| {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, alias).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, alias).unwrap();
    };
    link(&root.join("value.mjs"), &root.join("alias.mjs"));
    link(&dir.root().join("config.json"), &root.join("tsconfig.json"));
    let mut session = BuildSession::new(&root).unwrap();
    let first = fresh_parity(&mut session, &linked_options());
    assert_eq!(session.sources.len(), 2);
    assert_eq!(observe(&first.product.artifacts), b"true\n");
    let before = inventory(&root).unwrap();
    write(
        dir.root(),
        "config.json",
        r#"{"compilerOptions":{"customConditions":["source"]}}"#,
    );
    assert!(BuildSession::recheck(&root, &session.sources, &before).is_err());
    let second = fresh_parity(&mut session, &linked_options());
    assert_eq!(
        first.product.fingerprints.resolution,
        second.product.fingerprints.resolution
    );
    assert!(Rc::ptr_eq(&first.product, &second.product));
    write(dir.root(), "other-config.json", r#"{"compilerOptions":{}}"#);
    fs::remove_file(root.join("tsconfig.json")).unwrap();
    link(
        &dir.root().join("other-config.json"),
        &root.join("tsconfig.json"),
    );
    let retargeted = fresh_parity(&mut session, &linked_options());
    assert_eq!(
        second.product.fingerprints.resolution,
        retargeted.product.fingerprints.resolution
    );
    assert!(Rc::ptr_eq(&second.product, &retargeted.product));
    fs::remove_file(root.join("value.mjs")).unwrap();
    let error = format!("{:#}", inventory(&root).err().unwrap());
    assert!(error.contains("alias.mjs"), "{error}");
}

#[test]
fn external_package_type_changes_reparse_unchanged_javascript() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("workspace/package");
    write(&root, "main.mjs", "import './linked/value.js';");
    write(&package, "value.js", "console.log('value');");
    write(dir.root(), "workspace/package.json", r#"{"type":"module"}"#);
    directory_link(&package, &root.join("linked"));
    let config = linked_options();
    let mut session = BuildSession::new(&root).unwrap();
    fresh_parity(&mut session, &config);
    let esm = session.sources[&package.join("value.js")].record.config();
    write(
        dir.root(),
        "workspace/package.json",
        r#"{"type":"commonjs"}"#,
    );
    let cjs = fresh_parity(&mut session, &config);
    assert_ne!(
        esm,
        session.sources[&package.join("value.js")].record.config()
    );
    assert_eq!(cjs.counters.source_parses, 1);
    assert_eq!(observe(&cjs.product.artifacts), b"value\n");
    remove_directory_link(&root.join("linked"));
}

#[test]
fn linked_css_compositions_and_urls_use_bounded_canonical_inputs() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(
        &root,
        "main.mjs",
        "export {button} from 'pkg/button.module.css';",
    );
    write(&package, "package.json", r#"{"exports":"./index.mjs"}"#);
    write(
        &package,
        "button.module.css",
        ".button { composes: base from './base.module.css'; background: url('./pixel.bin'); }",
    );
    write(&package, "base.module.css", ".base { color: red; }");
    write(&package, "pixel.bin", "pixel");
    directory_link(&package, &root.join("node_modules/pkg"));
    let mut config = linked_options();
    config.css.css_modules = CssModules::Enabled;
    let mut session = BuildSession::new(&root).unwrap();
    let first = fresh_parity(&mut session, &config);
    assert!(first
        .product
        .artifacts
        .stylesheets
        .contains_key(&package.join("button.module.css")));
    assert!(session
        .sources
        .contains_key(&package.join("base.module.css")));
    assert!(first
        .product
        .artifacts
        .resources
        .values()
        .any(|bytes| bytes == b"pixel"));
    write(&package, "pixel.bin", "edited");
    let changed = fresh_parity(&mut session, &config);
    assert_ne!(
        first.product.fingerprints.output,
        changed.product.fingerprints.output
    );
    assert!(changed
        .product
        .artifacts
        .resources
        .values()
        .any(|bytes| bytes == b"edited"));
    remove_directory_link(&root.join("node_modules/pkg"));
}
