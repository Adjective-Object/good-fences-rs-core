use super::*;
// Exhaustive graph/security reference tests remain, alongside demand-build tests.
use super::super::reference_inventory as inventory;

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
fn early_linked_children_prepare_privately_with_checked_canonical_membership() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("linked-package");
    write(
        &root,
        "main.mjs",
        "export {value} from './node_modules/dependency/index.mjs';",
    );
    write(&package, "package.json", r#"{"type":"module"}"#);
    write(&package, "index.mjs", "export {value} from './value.mjs';");
    write(&package, "value.mjs", "export const value = 42;");
    let alias = root.join("node_modules/dependency");
    directory_link(&package, &alias);
    let (release, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    prepare::AFTER_DEPENDENCIES.with(|slot| {
        *slot.borrow_mut() = Some(std::sync::Arc::new(move |path| {
            if path.file_name().unwrap() == "main.mjs" {
                wait.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(30))
                    .expect("checked linked child must prepare while parent extraction is paused");
            }
        }));
    });
    let observer = observe_preparation(move |path| {
        if path.file_name().unwrap() == "index.mjs" {
            release.send(()).unwrap();
        }
    });
    let mut config = linked_options();
    config.cache = CacheMode::Disabled;
    config.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(4).unwrap());
    let built = BuildSession::new(&root).unwrap().build(&config).unwrap();
    drop(observer);
    assert_eq!(built.counters.source_parses, 3);
    assert_eq!(built.counters.speculative_parses, 0);
    assert!(built
        .product
        .graph
        .file_id(&package.join("value.mjs"))
        .is_some());
    remove_directory_link(&alias);
}

#[test]
fn literal_build_never_enumerates_large_unused_or_broken_node_modules() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    write(&root, "main.mjs", "export {value} from './value.mjs';");
    write(&root, "value.mjs", "export const value = 1;");
    for directory in 0..128 {
        for file in 0..32 {
            write(
                &root,
                &format!("node_modules/unused/{directory}/{file}.json"),
                "not json",
            );
        }
    }
    let missing = dir.root().join("missing");
    fs::create_dir(&missing).unwrap();
    let broken = root.join("node_modules/unused/broken");
    directory_link(&missing, &broken);
    fs::remove_dir(missing).unwrap();
    let mut session = BuildSession::new(&root).unwrap();
    let first = session.build(&linked_options()).unwrap();
    assert_eq!(first.counters.inventory_directory_reads, 0);
    let observer = observe_preparation(|path| {
        if path.file_name().is_some_and(|name| name == "main.mjs") {
            fs::write(
                path.parent().unwrap().join("node_modules/unused/0/0.json"),
                "changed",
            )
            .unwrap();
            fs::write(
                path.parent().unwrap().join("tsconfig.json"),
                "invalid unused config",
            )
            .unwrap();
        }
    });
    let noop = session.build(&linked_options()).unwrap();
    drop(observer);
    assert_eq!(noop.counters.inventory_directory_reads, 0);
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    let disk = BuildSession::new(&root)
        .unwrap()
        .build(&linked_options())
        .unwrap();
    assert_eq!(disk.counters.inventory_directory_reads, 0);
    assert_eq!(disk.counters.source_parses, 0);
    remove_directory_link(&broken);
}

#[test]
fn eager_local_walk_uses_selected_peer_and_runtime_links_without_walking_installations() {
    let dir = TmpDir::new();
    let root = dir.root();
    write(
        root,
        "package.json",
        r#"{
        "dependencies":{"dep":"*"},
        "peerDependencies":{"@scope/peer":"*","missing":"*"},
        "devDependencies":{"dev":"*"}
    }"#,
    );
    write(root, "main.mjs", "");
    for name in ["dep", "peer", "dev", "unselected"] {
        write(root, &format!("packages/{name}/package.json"), "{}");
        write(root, &format!("packages/{name}/source.mjs"), "");
    }
    write(root, "node_modules/registry/package.json", "{}");
    write(root, "node_modules/registry/never.mjs", "");
    fs::create_dir_all(root.join("node_modules/@scope")).unwrap();
    for (name, package) in [("dep", "dep"), ("@scope/peer", "peer"), ("dev", "dev")] {
        directory_link(
            &root.join("packages").join(package),
            &root.join("node_modules").join(name),
        );
    }
    let found = crate::build::discovery::candidates(
        root,
        std::iter::once(root.join("main.mjs")),
        &options(),
    )
    .unwrap();
    let expected: BTreeSet<_> = [
        "main.mjs",
        "packages/dep/source.mjs",
        "packages/peer/source.mjs",
    ]
    .into_iter()
    .map(|path| root.join(path).canonicalize().unwrap())
    .collect();
    assert_eq!(found, expected);
    for name in ["dep", "@scope/peer", "dev"] {
        remove_directory_link(&root.join("node_modules").join(name));
    }
}

#[test]
fn demand_admission_rechecks_links_and_discovered_reserved_scopes_in_either_order() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("target/packages/package");
    write(&root, "main.mjs", "");
    write(&package, "value.mjs", "");
    directory_link(&package, &root.join("package"));
    for name in ["target", ".git", ".bundler-out"] {
        let target = package.join(name).join("nested");
        write(&target, "value.mjs", "");
        directory_link(&target, &root.join("bypass"));
        for reverse in [false, true] {
            let inventory = Inventory::new(&root.canonicalize().unwrap());
            let paths = [
                root.join("package/value.mjs"),
                root.join("bypass/value.mjs"),
            ];
            let (first, second) = if reverse {
                (&paths[1], &paths[0])
            } else {
                (&paths[0], &paths[1])
            };
            inventory.admit(first).unwrap();
            let error = format!("{:#}", inventory.admit(second).unwrap_err());
            assert!(error.contains("reserved/excluded"), "{error}");
        }
        remove_directory_link(&root.join("bypass"));
    }
    let inventory = Inventory::new(&root.canonicalize().unwrap());
    inventory.admit(&root.join("package/value.mjs")).unwrap();
    inventory.recheck().unwrap();
    remove_directory_link(&root.join("package"));
    fs::create_dir(root.join("package")).unwrap();
    write(&root, "package/value.mjs", "");
    assert!(inventory.recheck().is_err());
}

#[test]
fn context_scans_only_matching_scopes_and_rechecks_additions_before_noop() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import './z.css'; export const load = n => import(`./pages/${n}.mjs`);",
    );
    write(dir.root(), "pages/one.mjs", "export default 1;");
    write(dir.root(), "z.css", "body {}");
    write(dir.root(), "unrelated/invalid.json", "not json");
    let missing = dir.root().join("missing");
    fs::create_dir(&missing).unwrap();
    directory_link(&missing, &dir.root().join("unrelated/broken"));
    fs::remove_dir(&missing).unwrap();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let config = linked_options();
    let first = session.build(&config).unwrap();
    assert_eq!(first.counters.inventory_directory_reads, 4); // root/pages, initial/final
    write(dir.root(), "unrelated/new.mjs", "export {};");
    write(dir.root(), "pages/nonmatching.txt", "ignored");
    let unchanged = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &unchanged.product));
    let new_file = dir.root().join("pages/two.mjs");
    let observer = observe_preparation(move |path| {
        if path.file_name().is_some_and(|name| name == "z.css") {
            fs::write(&new_file, "export default 2;").unwrap();
        }
    });
    let error = format!("{:#}", session.build(&config).err().unwrap());
    drop(observer);
    assert!(
        error.contains("context candidates changed during build"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    let added = session.build(&config).unwrap();
    assert!(!Rc::ptr_eq(&first.product, &added.product));
    assert!(
        added
            .product
            .graph
            .file_id(&dir.root().join("pages/two.mjs"))
            .is_some()
    );
    fs::remove_file(dir.root().join("pages/one.mjs")).unwrap();
    let removed = fresh_parity(&mut session, &config);
    assert!(
        removed
            .product
            .graph
            .file_id(&dir.root().join("pages/one.mjs"))
            .is_none()
    );
    remove_directory_link(&dir.root().join("unrelated/broken"));
}

#[test]
fn broad_dynamic_import_context_keeps_checked_aliases_finite_and_scoped() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let external = dir.root().join("external");
    write(&root, "main.mjs", "import './folder/loader.mjs';");
    write(
        &root,
        "folder/loader.mjs",
        "export const load = n => import(n); console.log((await load('./linked/deep/value.mjs')).value);",
    );
    write(&root, "outside.mjs", "invalid syntax!");
    write(&external, "deep/value.mjs", "export const value = 42;");
    write(&external, ".bundler-output/bad.mjs", "invalid syntax!");
    directory_link(&external, &root.join("folder/linked"));
    directory_link(&root, &root.join("folder/cycle"));
    let mut session = BuildSession::new(&root).unwrap();
    let broad_warnings = |built: &BuildResult| {
        built
            .warnings
            .iter()
            .filter(|warning| warning.contains("broad recursive capture"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let first = session.build(&linked_options()).unwrap();
    assert_eq!(broad_warnings(&first).len(), 1);
    assert_eq!(observe(&first.product.artifacts), b"42\n");
    assert_eq!(first.product.graph.file_count(), 3);
    let noop = session.build(&linked_options()).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    write(&external, "deep/added.mjs", "export const value = 43;");
    let added = session.build(&linked_options()).unwrap();
    assert_eq!(added.product.graph.file_count(), 4);
    assert_eq!(broad_warnings(&added), broad_warnings(&first));
    remove_directory_link(&root.join("folder/cycle"));
    remove_directory_link(&root.join("folder/linked"));
}

#[test]
fn contexts_settle_late_checked_link_scopes_before_graph_construction() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("workspace/package");
    let sibling = dir.root().join("workspace/sibling");
    write(
        &root,
        "main.mjs",
        "export {load} from './linked/index.mjs'; import './z.module.css';",
    );
    write(
        &package,
        "index.mjs",
        "export const load = n => import(`../sibling/${n}.mjs`);",
    );
    write(&sibling, "one.mjs", "export default 1;");
    write(&sibling, "base.module.css", ".base {}");
    write(
        &root,
        "z.module.css",
        ".local { composes: base from './styles/base.module.css'; }",
    );
    directory_link(&package, &root.join("linked"));
    directory_link(&sibling, &root.join("styles"));
    let mut config = linked_options();
    config.css.css_modules = CssModules::Enabled;
    let mut session = BuildSession::new(&root).unwrap();
    let first = fresh_parity(&mut session, &config);
    assert!(
        first
            .product
            .graph
            .file_id(&sibling.join("one.mjs"))
            .is_some()
    );
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    assert!(noop.invalidated.is_empty());
    write(
        &package,
        "index.mjs",
        "export const load = n => import(`../sibling/${n}.mjs`);\n//# sourceMappingURL=missing-index.js.map",
    );
    write(&sibling, "one.mjs", "eval('late context failure');");
    write(&sibling, "two.mjs", "export default 2;");
    config.output.source_maps = true;
    let mut failing = BuildSession::new(&root).unwrap();
    let error = failing.build(&config).err().unwrap();
    let failures = error.downcast_ref::<PreparationErrors>().unwrap();
    assert_eq!(failures.errors().len(), 2, "{error:#}");
    let summary = format!("{error:#}");
    assert!(summary.contains("missing-index.js.map"), "{summary}");
    assert!(summary.contains("policy uses-eval"), "{summary}");
    assert!(failing.sources.contains_key(&sibling.join("two.mjs")));
    assert!(failing.last.is_none());
    remove_directory_link(&root.join("linked"));
    remove_directory_link(&root.join("styles"));
}

#[test]
fn reviewed_css_wrappers_use_final_checked_aliases_in_either_discovery_order() {
    for package_name in ["a-package", "z-package"] {
        for load in [CssLoad::CallerLoaded, CssLoad::AutoMount] {
            let dir = TmpDir::new();
            let root = dir.root().join("project");
            let package = dir.root().join(package_name);
            write(
                &root,
                "main.mjs",
                "export {default} from './z/style.css'; import './late.css';",
            );
            write(&root, "late.css", "@import './a/style.css';");
            write(&package, "style.css", "body { color: red; }");
            directory_link(&package, &root.join("z"));
            directory_link(&package, &root.join("a"));
            let mut config = linked_options();
            config.css.load = load;
            config.output.base_url = "https://cdn.example.test/".into();
            let mut session = BuildSession::new(&root).unwrap();
            let built = fresh_parity(&mut session, &config);
            let artifacts = &built.product.artifacts;
            let stylesheet = &artifacts.stylesheets[&package.join("style.css")];
            assert!(artifacts.resources.contains_key(stylesheet));
            let late = &artifacts.stylesheets[&root.join("late.css")];
            let late_css = std::str::from_utf8(&artifacts.resources[late]).unwrap();
            assert!(
                late_css.contains(stylesheet.file_name().unwrap().to_str().unwrap()),
                "{late_css}"
            );
            let expected = format!(
                "https://cdn.example.test/{}",
                stylesheet.to_string_lossy().replace('\\', "/")
            );
            assert_eq!(
                observe_named(artifacts, &format!(
                    "{CSS_DOM_RECORDER}
                     const main = await import('./main.mjs');
                     assert.equal(main.default, {});
                     assert.equal(links.length, {});
                     for (const link of links) {{
                         assert.ok(existsSync(new URL('.' + new URL(link.href).pathname, import.meta.url)));
                     }}
                     if (links.length) assert.ok(links.some(link => link.href === main.default));",
                    serde_json::to_string(&expected).unwrap(),
                    if load == CssLoad::AutoMount { 2 } else { 0 },
                )),
                b""
            );
            let noop = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&built.product, &noop.product));
            assert!(noop.invalidated.is_empty());
            remove_directory_link(&root.join("z"));
            remove_directory_link(&root.join("a"));
        }
    }
}

#[test]
fn reviewed_upstream_maps_settle_linked_contexts_and_still_reject_real_additions() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("workspace/package");
    let sibling = dir.root().join("workspace/sibling");
    let extra = dir.root().join("workspace/extra");
    write(
        &root,
        "main.mjs",
        "export {load,extra} from './linked/index.mjs';\n//# sourceMappingURL=./maps/index.map",
    );
    write(
        &package,
        "index.mjs",
        "export const load = name => import(`../sibling/${name}.mjs`);
         export const extra = name => import(`../extra/${name}.mjs`);",
    );
    write(
        &sibling,
        "one.mjs",
        "export default 1;\n//# sourceMappingURL=next/index.map",
    );
    write(
        &extra,
        "two.mjs",
        "export default 2;\n//# sourceMappingURL=two.map",
    );
    for (directory, name) in [
        (&sibling, "index.map"),
        (&extra, "index.map"),
        (&extra, "two.map"),
    ] {
        write(
            directory,
            name,
            r#"{"version":3,"sources":[],"names":[],"mappings":""}"#,
        );
    }
    directory_link(&package, &root.join("linked"));
    directory_link(&sibling, &root.join("maps"));
    directory_link(&extra, &sibling.join("next"));
    let mut config = linked_options();
    config.output.source_maps = true;
    let mut session = BuildSession::new(&root).unwrap();
    let first = fresh_parity(&mut session, &config);
    assert_eq!(session.upstreams.len(), 3);
    assert_eq!(
        observe_named(
            &first.product.artifacts,
            "import assert from 'node:assert/strict'; import {load,extra} from './main.mjs'; assert.equal((await load('one')).default, 1); assert.equal((await extra('two')).default, 2);"
        ),
        b""
    );
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&first.product, &noop.product));
    assert!(noop.invalidated.is_empty());
    let new_file = sibling.join("two.mjs");
    let observer = observe_preparation(move |path| {
        if path.file_name().is_some_and(|name| name == "one.mjs") {
            fs::write(&new_file, "export default 2;").unwrap();
        }
    });
    let error = format!("{:#}", session.build(&config).err().unwrap());
    drop(observer);
    assert!(
        error.contains("context candidates changed during build"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    let added = fresh_parity(&mut session, &config);
    assert_eq!(
        observe_named(
            &added.product.artifacts,
            "import assert from 'node:assert/strict'; import {load} from './main.mjs'; assert.equal((await load('two')).default, 2);"
        ),
        b""
    );
    remove_directory_link(&sibling.join("next"));
    remove_directory_link(&root.join("linked"));
    remove_directory_link(&root.join("maps"));
}

#[test]
fn file_link_authorizes_only_its_target_not_external_siblings_or_maps() {
    let dir = TmpDir::new();
    let root = dir.root().join("project");
    let package = dir.root().join("package");
    write(&root, "main.mjs", "export {value} from './linked.mjs';");
    write(&package, "value.mjs", "export const value = 1;");
    write(&package, "sibling.mjs", "export const value = 2;");
    write(
        &package,
        "value.mjs.map",
        r#"{"version":3,"sources":[],"names":[],"mappings":""}"#,
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(package.join("value.mjs"), root.join("linked.mjs")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(package.join("value.mjs"), root.join("linked.mjs")).unwrap();
    let mut session = BuildSession::new(&root).unwrap();
    let mut config = linked_options();
    let first = session.build(&config).unwrap();
    write(
        &package,
        "value.mjs",
        "export {value} from './sibling.mjs';",
    );
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(
        error.contains("outside reachable build inventory"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    write(
        &package,
        "value.mjs",
        "export const value = 1;\n//# sourceMappingURL=value.mjs.map",
    );
    config.output.source_maps = true;
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(error.contains("source map outside build root"), "{error}");
}

#[test]
fn entry_package_format_observations_reject_final_manifest_mutations_without_requests() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(dir.root(), "main.js", "export const value = 1;");
    let mut config = linked_options();
    config.entries.get_mut("main").unwrap().path = "main.js".into();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    let observer = observe_preparation(|path| {
        fs::write(
            path.parent().unwrap().join("package.json"),
            r#"{"type":"commonjs"}"#,
        )
        .unwrap();
    });
    let error = format!("{:#}", session.build(&config).err().unwrap());
    drop(observer);
    assert!(
        error.contains("resolution input changed") && error.contains("package.json"),
        "{error}"
    );
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
}

#[cfg(windows)]
#[test]
fn absolute_entries_keep_case_insensitive_host_identity_without_external_authority() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mjs", "export const value = 1;");
    let mut config = linked_options();
    config.entries.get_mut("main").unwrap().path =
        PathBuf::from(dir.root().join("main.mjs").to_string_lossy().to_lowercase());
    let built = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert!(
        built
            .product
            .graph
            .file_id(&dir.root().join("main.mjs"))
            .is_some()
    );
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
    assert!(
        warm.warnings
            .iter()
            .any(|w| w.contains("uncacheable external path"))
    );
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
    write(
        dir.root(),
        "main.mjs",
        "import {value as a} from './real/value.mjs'; import {value as b} from './alias/value.mjs'; console.log(a === b);",
    );
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
    assert!(
        built
            .product
            .graph
            .file_id(&package.join("index.js"))
            .is_some()
    );
    assert!(
        built
            .product
            .graph
            .file_id(&root.join("node_modules/a/index.js"))
            .is_none()
    );
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
    assert!(before.recheck(&root).is_err());
    let second = fresh_parity(&mut session, &config);
    assert_ne!(
        first.product.fingerprints.resolution,
        second.product.fingerprints.resolution
    );
    assert!(
        session
            .sources
            .contains_key(&dir.root().join("two/index.js"))
    );
    assert!(
        !session
            .sources
            .contains_key(&dir.root().join("one/index.js"))
    );
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
    assert!(
        session
            .sources
            .contains_key(&package.join("node_modules/dep/default.mjs"))
    );
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
    assert!(
        session
            .sources
            .contains_key(&package.join("node_modules/dep/default.mjs"))
    );
    config.resolver.export_conditions.push("source".into());
    fresh_parity(&mut session, &config);
    assert!(
        session
            .sources
            .contains_key(&package.join("node_modules/dep/source.mjs"))
    );
    let before = inventory(&root).unwrap();
    write(dir.root(), "workspace/unrelated/new.mjs", "");
    assert_eq!(before, inventory(&root).unwrap());
    write(
        dir.root(),
        "workspace/tsconfig.json",
        r#"{"compilerOptions":{}}"#,
    );
    assert!(before.recheck(&root).is_err());
    config
        .resolver
        .export_conditions
        .retain(|condition| condition != "source");
    fresh_parity(&mut session, &config);
    assert!(
        session
            .sources
            .contains_key(&package.join("node_modules/dep/default.mjs"))
    );
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
    let herd = bumpalo_herd::Herd::new();
    let scratch = scratch::Scratch::new(&herd);
    let checked = resolver::BuildResolver::new(
        &root,
        &config,
        resolver::Lockfile::read(&root, &mut Vec::new(), config.execution).unwrap(),
        resolver::Memory::default(),
        &scratch,
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
    assert!(
        session
            .sources
            .contains_key(&package.join("actual/one.mjs"))
    );
    write(&package, "actual/two.mjs", "export default 2;");
    fresh_parity(&mut session, &config);
    assert!(
        session
            .sources
            .contains_key(&package.join("actual/two.mjs"))
    );
    fs::remove_file(package.join("actual/one.mjs")).unwrap();
    fresh_parity(&mut session, &config);
    assert!(
        !session
            .sources
            .contains_key(&package.join("actual/one.mjs"))
    );
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
    let herd = bumpalo_herd::Herd::new();
    let scratch = scratch::Scratch::new(&herd);
    let resolver = resolver::BuildResolver::new(
        root,
        &config,
        resolver::Lockfile::read(root, &mut Vec::new(), config.execution).unwrap(),
        resolver::Memory::default(),
        &scratch,
    );
    let resolved = crate::context::resolve(
        root,
        &importer,
        (&dependency).into(),
        candidates.into_iter(),
        &config.resolver.extensions,
        &resolver,
        true,
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
    write(
        root,
        "main.mjs",
        &format!("export const load = n => import(`{prefix}${{n}}/one.mjs`);"),
    );
    let demanded = BuildSession::new(root).unwrap().build(&config).unwrap();
    assert_eq!(demanded.counters.inventory_directory_reads, 42);
    assert!(
        demanded
            .product
            .graph
            .file_id(&directories[20].join("one.mjs"))
            .is_some()
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
        true,
    )
    .unwrap();
    assert!(
        resolved.is_empty(),
        "physical projections must not reopen root cycles"
    );
    write(
        &directories[20],
        "one.mjs",
        "export const load = n => import(`./cycle/${n}.mjs`);",
    );
    let demanded_cycle = BuildSession::new(root).unwrap().build(&config).unwrap();
    assert_eq!(demanded_cycle.product.graph.file_count(), 2);
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
                true,
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
    write(
        &root,
        "main.mjs",
        "import {value as a} from './value.mjs'; import {value as b} from './alias.mjs'; console.log(a === b);",
    );
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
    assert!(before.recheck(&root).is_err());
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
        "export {button} from 'pkg/button.module.css'; import './more.css';",
    );
    write(
        &root,
        "more.css",
        ".shared { background: url('../package/pixel.bin'); }",
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
    assert!(
        first
            .product
            .artifacts
            .stylesheets
            .contains_key(&package.join("button.module.css"))
    );
    assert!(
        session
            .sources
            .contains_key(&package.join("base.module.css"))
    );
    assert!(
        first
            .product
            .artifacts
            .resources
            .values()
            .any(|bytes| bytes == b"pixel")
    );
    write(&package, "pixel.bin", "edited");
    let changed = fresh_parity(&mut session, &config);
    assert_ne!(
        first.product.fingerprints.output,
        changed.product.fingerprints.output
    );
    assert!(
        changed
            .product
            .artifacts
            .resources
            .values()
            .any(|bytes| bytes == b"edited")
    );
    remove_directory_link(&root.join("node_modules/pkg"));
}
