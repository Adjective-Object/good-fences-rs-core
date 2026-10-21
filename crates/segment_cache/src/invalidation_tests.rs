use std::{
    fs,
    path::{Path, PathBuf},
};

use ast_segmenter::{owned::DependencyKind, ModuleBody, ModuleFormat};
use import_resolver::{
    node_resolver::{caching::NodeModulesResolverOptions, MonorepoResolver},
    resolve::{PathResolver, TargetEnv},
};
use test_tmpdir::TmpDir;

use crate::*;

fn load(path: &Path, package: ModuleFormat) -> CacheOutcome {
    load_or_parse(
        &logger::StdioLogger::new(),
        path,
        package,
        CacheMode::ReadWrite,
    )
    .unwrap()
}

fn package_format(root: &Path) -> ModuleFormat {
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("package.json")).unwrap()).unwrap();
    if package["type"] == "module" {
        ModuleFormat::Esm
    } else {
        ModuleFormat::CommonJs
    }
}

fn resolver(root: &Path, extensions: &[&str]) -> MonorepoResolver {
    let mut options = NodeModulesResolverOptions::default_for_env(TargetEnv::Node);
    options.extensions = extensions.iter().map(|s| (*s).to_owned()).collect();
    MonorepoResolver::new_resolver(root.to_owned(), options)
}

fn write(root: &Path, relative: &str, source: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, source).unwrap();
    path
}

#[test]
fn cold_warm_and_disabled_match_every_owned_fact_and_segment() {
    let dir = TmpDir::new();
    for (name, source, format) in [
        ("facts.ts", "#!/bin/node\n'use strict'; import main, {value as local} from './dep'; import * as ns from './other'; export {local as '名'}; export * from './star'; export * as space from './space'; export const {a: [first], ...rest} = ns; const later = () => forward; var forward = main; function shadow(local: number) { return local + first; } export default shadow; import('./lazy'); type T = import('./types').T; const café = local; café;", ModuleFormat::Esm),
        ("facts.cjs", "#!/bin/node\n'use strict'; exports.value = 1; module.exports = { value: 2 }; exports = {}; module = {}; require('./dep'); function deferred(){ return require('./dep'); } function shadow(require){ return require('not-a-dependency'); } return;", ModuleFormat::CommonJs),
        ("async.mjs", "await 1; export const value = await Promise.resolve(2); for await (const x of [1]) { await x; }", ModuleFormat::Esm),
        ("deferred.mjs", "const target = 1; async function f(value = target, callback = async () => target) { target; (() => target)(); class C { field = target; static { target; } callback = async () => target; async [target]() { target; } } }", ModuleFormat::Esm),
        ("empty.js", "// no statements\n", ModuleFormat::Esm),
        ("declarations.d.ts", "export interface Value { name: string } export declare const value: Value; export type { Value as Renamed };", ModuleFormat::Esm),
    ] {
        let path = write(dir.root(), name, source);
        let parsed = ast_segmenter::parse_module(&logger::StdioLogger::new(), source, &path, format).unwrap();
        let cold = load(&path, format);
        let warm = load(&path, format);
        let disabled = load_or_parse(&logger::StdioLogger::new(), &path, format, CacheMode::Disabled).unwrap();
        assert_eq!(cold.write, CacheWrite::Stored);
        assert_eq!(warm.status, CacheStatus::Hit);
        for record in [&cold.record, &warm.record, &disabled.record] {
            let loaded = record.to_parsed(source).unwrap();
            assert_eq!(loaded.to_flatbuffer(source, OXC_REVISION).unwrap(), parsed.to_flatbuffer(source, OXC_REVISION).unwrap());
            assert!(loaded.owned_facts().is_err());
            assert!(loaded.owned_segments().is_none());
            assert_eq!(record.module().body(), parsed.body());
            assert_eq!(record.module().segments(), parsed.segments().iter().map(|s| s.span).collect::<Vec<_>>());
        }
        assert_eq!(cold.record, disabled.record);
        assert_eq!(cold.record, warm.record);
    }
}

#[test]
fn changed_bytes_with_identical_length_and_mtime_miss() {
    let dir = TmpDir::new();
    let path = write(dir.root(), "entry.js", "export const value = 1;");
    let original = load(&path, ModuleFormat::Esm);
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "export const value = 2;").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    let edited = load(&path, ModuleFormat::Esm);
    assert_eq!(edited.source.len(), original.source.len());
    assert_ne!(edited.record.source_hash(), original.record.source_hash());
    assert!(matches!(
        edited.status,
        CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
    assert_eq!(edited.write, CacheWrite::Stored);
    assert_eq!(load(&path, ModuleFormat::Esm).status, CacheStatus::Hit);
}

#[test]
fn package_type_changes_reparse_but_explicit_extensions_override_it() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    let path = write(dir.root(), "entry.js", "'use strict'; const value = 1;");
    let esm = load(&path, package_format(dir.root()));
    write(dir.root(), "package.json", r#"{"type":"commonjs"}"#);
    let cjs = load(&path, package_format(dir.root()));
    assert!(matches!(
        cjs.status,
        CacheStatus::Miss(MissReason::InvalidRecord(_))
    ));
    assert_eq!(esm.record.module().body(), ModuleBody::Esm);
    assert!(matches!(
        cjs.record.module().body(),
        ModuleBody::CommonJs { .. }
    ));
    assert_ne!(esm.record.config(), cjs.record.config());
    for (extension, expected) in [("mjs", false), ("mts", false), ("cjs", true), ("cts", true)] {
        let explicit = write(
            dir.root(),
            &format!("explicit.{extension}"),
            "const value = 1;",
        );
        let first = load(&explicit, ModuleFormat::Esm);
        let second = load(&explicit, ModuleFormat::CommonJs);
        assert_eq!(first.record.config().commonjs(), expected);
        assert_eq!(second.status, CacheStatus::Hit);
        assert_eq!(first.record, second.record);
    }
}

#[test]
fn every_effective_parser_mode_matches_the_pinned_parser() {
    let dir = TmpDir::new();
    for extension in [
        "js", "jsx", "ts", "tsx", "mjs", "mts", "cjs", "cts", "d.ts", "d.mts", "d.cts",
    ] {
        let path = write(
            dir.root(),
            &format!("entry.{extension}"),
            "const value = 1;",
        );
        for format in [ModuleFormat::Esm, ModuleFormat::CommonJs] {
            let config = ParserConfig::for_path(&path, format).unwrap();
            let parsed = ast_segmenter::parse_module(
                &logger::StdioLogger::new(),
                "const value = 1;",
                &path,
                format,
            )
            .unwrap();
            assert_eq!(
                config,
                CacheRecord::from_parsed("const value = 1;", &parsed)
                    .unwrap()
                    .config(),
                "{extension}"
            );
        }
    }
    assert!(ParserConfig::for_path(&dir.root_join("entry.json"), ModuleFormat::Esm).is_err());
}

#[test]
fn language_and_definition_changes_invalidate_copied_sidecars() {
    let dir = TmpDir::new();
    let source = "export const value = 1;";
    let js = write(dir.root(), "entry.js", source);
    let baseline = load(&js, ModuleFormat::Esm);
    for name in ["entry.ts", "entry.jsx", "entry.tsx", "entry.d.ts"] {
        let destination = write(dir.root(), name, source);
        fs::copy(sidecar_path(&js), sidecar_path(&destination)).unwrap();
        let result = load(&destination, ModuleFormat::Esm);
        assert!(
            matches!(
                result.status,
                CacheStatus::Miss(MissReason::InvalidRecord(_))
            ),
            "{name}"
        );
        assert_ne!(baseline.record.config(), result.record.config());
        assert_eq!(
            load(&destination, ModuleFormat::Esm).status,
            CacheStatus::Hit
        );
    }
}

#[test]
fn relocation_reuses_bytes_but_resolves_dependencies_under_new_root() {
    let dir = TmpDir::new();
    let a = dir.root_join("original");
    let b = dir.root_join("relocated");
    let source = "import {value} from './dep'; export {value}; import('./lazy');";
    let original = write(&a, "src/entry.js", source);
    write(&a, "src/dep.js", "export const value = 'original';");
    let cold = load(&original, ModuleFormat::Esm);
    let relocated = write(&b, "src/entry.js", source);
    write(&b, "src/dep.js", "export const value = 'relocated';");
    fs::copy(sidecar_path(&original), sidecar_path(&relocated)).unwrap();
    let warm = load(&relocated, ModuleFormat::Esm);
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(cold.record, warm.record);
    let specifier = warm
        .record
        .module()
        .facts()
        .dependencies()
        .get(0)
        .unwrap()
        .specifier();
    assert_eq!(specifier, "./dep");
    let old_target = resolver(&a, &["js"])
        .resolve(&original, specifier)
        .unwrap()
        .path;
    let new_target = resolver(&b, &["js"])
        .resolve(&relocated, specifier)
        .unwrap()
        .path;
    assert_eq!(old_target, a.join("src/dep.js"));
    assert_eq!(new_target, b.join("src/dep.js"));
    assert_ne!(old_target, new_target);
    let bytes = fs::read(sidecar_path(&relocated)).unwrap();
    for root in [&a, &b] {
        let text = root.to_str().unwrap().as_bytes();
        assert!(!bytes.windows(text.len()).any(|w| w == text));
    }
    assert_eq!(bytes, fs::read(sidecar_path(&original)).unwrap());
}

#[test]
fn resolver_and_effect_configuration_changes_leave_parse_cache_unchanged() {
    let dir = TmpDir::new();
    let path = write(
        dir.root(),
        "entry.js",
        "import {value} from './dep'; export {value};",
    );
    write(dir.root(), "dep.jsx", "export const value = 1;");
    write(dir.root(), "dep.ts", "export const value: number = 2;");
    write(
        dir.root(),
        "package.json",
        r#"{"type":"module","sideEffects":false}"#,
    );
    write(dir.root(), "effects.json", r#"{"knownEffectful":[]}"#);
    let cold = load(&path, package_format(dir.root()));
    let bytes = fs::read(sidecar_path(&path)).unwrap();
    let modified = fs::metadata(sidecar_path(&path))
        .unwrap()
        .modified()
        .unwrap();
    let dep = cold.record.module().facts().dependencies().get(0).unwrap();
    assert_eq!(dep.kind(), DependencyKind::Import);
    let js = resolver(dir.root(), &["jsx"])
        .resolve(&path, &dep.specifier())
        .unwrap();
    let ts = resolver(dir.root(), &["ts"])
        .resolve(&path, &dep.specifier())
        .unwrap();
    assert_eq!(js.path, dir.root_join("dep.jsx"));
    assert_eq!(ts.path, dir.root_join("dep.ts"));
    write(
        dir.root(),
        "package.json",
        r#"{"type":"module","sideEffects":true,"exports":"./changed.js"}"#,
    );
    write(
        dir.root(),
        "effects.json",
        r#"{"knownEffectful":["*.appendChild(...)"]}"#,
    );
    write(
        dir.root(),
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"alias":["dep.ts"]}}}"#,
    );
    let warm = load(&path, package_format(dir.root()));
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(warm.record, cold.record);
    assert_eq!(fs::read(sidecar_path(&path)).unwrap(), bytes);
    assert_eq!(
        fs::metadata(sidecar_path(&path))
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
}

#[test]
fn deleted_source_or_dependency_never_comes_from_an_orphan_cache() {
    let dir = TmpDir::new();
    let path = write(dir.root(), "entry.js", "import './dep';");
    let target = write(dir.root(), "dep.js", "export const value = 1;");
    load(&target, ModuleFormat::Esm);
    let entry = load(&path, ModuleFormat::Esm);
    assert!(resolver(dir.root(), &["js"])
        .resolve(&path, "./dep")
        .is_ok());
    fs::remove_file(&target).unwrap();
    assert!(sidecar_path(&target).exists());
    assert!(load_or_parse(
        &logger::StdioLogger::new(),
        &target,
        ModuleFormat::Esm,
        CacheMode::ReadWrite
    )
    .is_err());
    assert!(resolver(dir.root(), &["js"])
        .resolve(&path, "./dep")
        .is_err());
    let warm = load(&path, ModuleFormat::Esm);
    assert_eq!(warm.status, CacheStatus::Hit);
    assert_eq!(entry.record, warm.record);
    fs::remove_file(&path).unwrap();
    assert!(sidecar_path(&path).exists());
    assert!(load_or_parse(
        &logger::StdioLogger::new(),
        &path,
        ModuleFormat::Esm,
        CacheMode::ReadWrite
    )
    .is_err());
    fs::write(&path, "import './dep';").unwrap();
    assert_eq!(load(&path, ModuleFormat::Esm).status, CacheStatus::Hit);
}

#[test]
fn diagnostics_are_regenerated_for_current_source_policy_and_path() {
    let dir = TmpDir::new();
    let path = write(
        dir.root(),
        "broken.js",
        "// This file is auto-generated\nexport const broken = ;",
    );
    for mode in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let error =
            load_or_parse(&logger::StdioLogger::new(), &path, ModuleFormat::Esm, mode).unwrap_err();
        assert!(error.to_string().contains(path.to_str().unwrap()));
        assert!(error.to_string().contains("parser diagnostics"));
        assert!(!sidecar_path(&path).exists());
    }
    fs::write(&path, "export const value = 1;").unwrap();
    load(&path, ModuleFormat::Esm);
    let bytes = fs::read(sidecar_path(&path)).unwrap();
    let relocated = write(dir.root(), "new-path.js", "export const value = 1;");
    fs::write(sidecar_path(&relocated), &bytes).unwrap();
    let error = load_or_parse(
        &logger::StdioLogger::new(),
        &relocated,
        ModuleFormat::CommonJs,
        CacheMode::ReadWrite,
    )
    .unwrap_err();
    let diagnostic = format!("{error:#}");
    assert!(
        diagnostic.contains(relocated.to_str().unwrap()),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains(path.to_str().unwrap()), "{diagnostic}");
    assert_eq!(fs::read(sidecar_path(&relocated)).unwrap(), bytes);
    fs::write(&relocated, [0xff, 0xfe]).unwrap();
    assert!(load_or_parse(
        &logger::StdioLogger::new(),
        &relocated,
        ModuleFormat::Esm,
        CacheMode::ReadWrite
    )
    .unwrap_err()
    .to_string()
    .contains("source read failed"));
}

#[test]
fn version_identity_tracks_workspace_oxc_pin() {
    let manifest = include_str!("../../../Cargo.toml");
    let pins: Vec<_> = manifest.lines().filter(|l| l.starts_with("oxc_")).collect();
    assert!(!pins.is_empty());
    assert!(pins.iter().all(|line| line.contains(OXC_REVISION)));
}
