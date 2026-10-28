use super::*;

fn typed() -> BuildOptions {
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    config
}

#[test]
fn type_only_wiring_declaration_erased_imports_warn_and_recheck_without_runtime_inputs() {
    for extension in ["d.ts", "d.mts", "d.cts"] {
        for cache in [
            CacheMode::ReadWrite,
            CacheMode::ReadOnly,
            CacheMode::Disabled,
        ] {
            let dir = TmpDir::new();
            let provider = format!("types.{extension}");
            write(dir.root(), &provider, "export interface Shape {}");
            write(dir.root(), "main.mts", &format!(
                "// original\n\nimport {{Shape as Alias}} from './{provider}'; export type Model=Alias; console.log('ok');"
            ));
            let mut config = typed();
            config.cache = cache;
            let mut session = BuildSession::new(dir.root()).unwrap();
            let cold = session.build(&config).unwrap();
            assert_eq!(observe(&cold.product.artifacts), b"ok\n");
            assert_eq!(cold.product.compatibility_warnings.len(), 1);
            let warning = &cold.product.compatibility_warnings[0];
            assert!(
                warning.contains("main.mts:3:9")
                    && warning.contains("Alias")
                    && warning.contains(&provider)
                    && warning.contains("import type"),
                "{warning}"
            );
            assert!(!session.sources.contains_key(&dir.root().join(&provider)));
            assert!(cold
                .product
                .graph
                .file_id(&dir.root().join(&provider))
                .is_none());
            assert!(cold
                .product
                .artifacts
                .files
                .values()
                .all(|text| !text.contains(&provider)));
            let consumer_hash = session.sources[&dir.root().join("main.mts")]
                .record
                .source_hash();
            assert_eq!(session.type_only_cache.extractions(), 2);
            let noop = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&cold.product, &noop.product));
            assert_eq!(noop.warnings, cold.warnings);
            assert_eq!(session.type_only_cache.extractions(), 2);
            let mut disk = BuildSession::new(dir.root()).unwrap();
            assert_eq!(disk.build(&config).unwrap().warnings, cold.warnings);
            assert_eq!(
                disk.type_only_cache.extractions(),
                if cache == CacheMode::ReadWrite { 0 } else { 2 }
            );
            write(dir.root(), &provider, "export declare const Shape: number;");
            let changed = session.build(&config).unwrap();
            assert!(changed.product.compatibility_warnings.is_empty());
            assert!(!Rc::ptr_eq(&cold.product, &changed.product));
            assert_eq!(
                session.sources[&dir.root().join("main.mts")]
                    .record
                    .source_hash(),
                consumer_hash
            );
            fs::remove_file(dir.root().join(&provider)).unwrap();
            assert!(session
                .build(&config)
                .unwrap()
                .product
                .compatibility_warnings
                .is_empty());
            write(dir.root(), &provider, "export type Shape=number;");
            assert_eq!(
                session
                    .build(&config)
                    .unwrap()
                    .product
                    .compatibility_warnings
                    .len(),
                1
            );
        }
    }
}

#[test]
fn type_only_wiring_declaration_explicit_type_barrels_default_aliases_and_silence() {
    for extension in ["d.ts", "d.mts", "d.cts"] {
        let dir = TmpDir::new();
        let provider = format!("types.{extension}");
        let barrel = format!("barrel.{extension}");
        let local = format!("local.{extension}");
        write(
            dir.root(),
            &provider,
            "export interface Shape {} export default interface Default {}",
        );
        write(
            dir.root(),
            &barrel,
            &format!("export type {{Shape as Alias,default}} from './{provider}';"),
        );
        write(dir.root(), &local, &format!("import type {{Alias as Local,default as Default}} from './{barrel}'; export type {{Local as Alias,Default as default}};"));
        write(dir.root(), "main.mts", &format!("import Default,{{Alias as Renamed}} from './{local}'; export type Model=Renamed; export type Other=Default; console.log('ok');"));
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&typed()).unwrap();
        assert_eq!(observe(&result.product.artifacts), b"ok\n");
        assert_eq!(result.product.compatibility_warnings.len(), 2);
        assert_eq!(result.product.graph.module_ids().count(), 1);
        write(
            dir.root(),
            "barrel.mts",
            &format!("export type {{Shape as Alias,default}} from './{provider}';"),
        );
        write(
            dir.root(),
            "main.mts",
            "export {Alias,default} from './barrel.mts';",
        );
        let result = session.build(&typed()).unwrap();
        assert_eq!(result.product.compatibility_warnings.len(), 2);
        let entry = result.product.artifacts.entries["main"]
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(observe_named(&result.product.artifacts, &format!(
            "import assert from 'node:assert/strict'; const ns=await import('./{entry}'); assert.deepEqual(Object.keys(ns),[]);"
        )), b"");
        assert!(result
            .product
            .graph
            .file_id(&dir.root().join(&provider))
            .is_none());
        write(dir.root(), "main.mts", &format!("import type Default from './{provider}'; import type {{Shape}} from './{provider}'; export type Model=Shape; export type Other=Default; console.log('explicit');"));
        let explicit = session.build(&typed()).unwrap();
        assert_eq!(observe(&explicit.product.artifacts), b"explicit\n");
        assert!(explicit.product.compatibility_warnings.is_empty());
    }
}

#[test]
fn type_only_wiring_declaration_runtime_imports_and_entries_still_reject_execution() {
    for extension in ["d.ts", "d.mts", "d.cts"] {
        let dir = TmpDir::new();
        let provider = format!("types.{extension}");
        write(dir.root(), &provider, "export interface Shape {}");
        write(
            dir.root(),
            "main.mts",
            &format!("import {{Shape}} from './{provider}'; console.log(Shape);"),
        );
        let mut session = BuildSession::new(dir.root()).unwrap();
        let error = session.build(&typed()).err().unwrap();
        assert!(
            format!("{error:#}").contains("declaration files are not executable inputs"),
            "{error:#}"
        );
        let mut entry = typed();
        entry.entries.get_mut("main").unwrap().path = provider.clone().into();
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&entry)
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains("declaration files are not executable inputs"),
            "{error:#}"
        );
        write(
            dir.root(),
            "main.mts",
            &format!("import type {{Shape}} from './{provider}'; console.log(Shape);"),
        );
        let error = session.build(&typed()).err().unwrap();
        assert!(
            format!("{error:#}").contains("executable value"),
            "{error:#}"
        );
    }
}

#[test]
fn type_only_wiring_mixed_barrels_and_local_aliases_keep_values_and_explicit_effects() {
    let dir = TmpDir::new();
    write(dir.root(), "types.mts", "console.log('unwanted'); export interface Shape {} export type Color=string; export default interface Default {}");
    write(dir.root(), "register.mjs", "console.log('registered');");
    write(
        dir.root(),
        "values.mts",
        "export const keep=42; export interface Shape {}",
    );
    write(dir.root(), "barrel.mts", "export {Shape as PublicShape, Color, default as DefaultShape} from './types.mts'; export {keep, Shape as MixedShape} from './values.mts';");
    write(
        dir.root(),
        "local.mts",
        "import {Shape as Local} from './types.mts'; export {Local as Alias};",
    );
    write(dir.root(), "main.mts", "import './register.mjs'; export {PublicShape,Color,DefaultShape,keep,MixedShape} from './barrel.mts'; export {Alias} from './local.mts'; console.log('main');");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&typed()).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"registered\nmain\n");
    assert_eq!(result.product.compatibility_warnings.len(), 11);
    let text = result
        .product
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<String>();
    for erased in [
        "PublicShape",
        "DefaultShape",
        "MixedShape",
        "unwanted",
        "Alias",
    ] {
        assert!(!text.contains(erased), "{text}");
    }
    assert!(text.contains("keep"));
    let noop = session.build(&typed()).unwrap();
    assert!(Rc::ptr_eq(&result.product, &noop.product));
    assert_eq!(noop.warnings, result.warnings);
    assert_eq!(noop.counters.source_parses, 0);
}

#[test]
fn type_only_wiring_erased_imports_warn_at_original_utf16_specifier_and_explicit_types_are_silent()
{
    let dir = TmpDir::new();
    write(
        dir.root(),
        "types.mts",
        "export interface Shape {} export const Value=1;",
    );
    write(dir.root(), "main.mts", "// 😀 é\r\n\r\nimport { Shape as Alias, Value } from './types.mts';\nimport type {Shape} from './types.mts';\nexport type Model=Alias;\nexport type Other=Shape;\nconsole.log('ok');");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&typed()).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"ok\n");
    assert_eq!(result.product.compatibility_warnings.len(), 1);
    let warning = &result.product.compatibility_warnings[0];
    assert!(warning.contains("main.mts:3:10"), "{warning}");
    assert!(
        warning.contains("Alias") && warning.contains("import type"),
        "{warning}"
    );
    assert!(!result.product.graph.module_ids().any(|id| result
        .product
        .graph
        .file_path(id)
        .unwrap()
        .ends_with("types.mts")));
}

#[test]
fn type_only_wiring_actual_values_still_fail_without_erasing_expressions() {
    for body in [
        "import {Shape} from './types.mts'; console.log(Shape);",
        "import {Shape} from './types.mts'; export const value=()=>Shape;",
        "import * as types from './types.mts'; console.log(types.Shape);",
        "import * as types from './types.mts'; console.log(types['Shape']);",
        "import type {Shape} from './types.mts'; console.log(Shape);",
        "import type * as types from './types.mts'; console.log(types.Shape);",
        "import {Shape} from './types.mts'; export default Shape;",
        "import {Shape} from './types.mts'; export default console.log(Shape);",
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "types.mts", "export interface Shape {}");
        write(dir.root(), "main.mts", body);
        let mut session = BuildSession::new(dir.root()).unwrap();
        let error = session.build(&typed()).err().unwrap();
        assert!(
            format!("{error:#}").contains("executable value"),
            "{error:#}"
        );
        let source = session.sources[&dir.root().join("main.mts")]
            .text
            .text(&dir.root().join("main.mts"))
            .unwrap();
        assert!(source.contains("Shape"));
    }
}

#[test]
fn type_only_wiring_cycles_renames_default_and_star_ambiguity() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "types.mts",
        "export interface Shape {} export default interface D {}",
    );
    write(
        dir.root(),
        "a.mts",
        "export * from './b.mts'; export {default as D} from './types.mts';",
    );
    write(
        dir.root(),
        "b.mts",
        "export * from './a.mts'; export {Shape as Renamed} from './types.mts';",
    );
    write(
        dir.root(),
        "main.mts",
        "export {Renamed,D} from './a.mts'; console.log('ok');",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&typed()).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"ok\n");
    write(dir.root(), "other.mts", "export interface Shape {}");
    write(
        dir.root(),
        "b.mts",
        "export * from './types.mts'; export * from './other.mts';",
    );
    write(dir.root(), "main.mts", "export {Shape} from './b.mts';");
    assert!(session.build(&typed()).is_err());
    write(dir.root(), "b.mts", "export * from './types.mts'; export * from './other.mts'; export {Shape} from './types.mts';");
    assert!(session.build(&typed()).is_ok());
}

#[test]
fn type_only_wiring_explicit_type_diamonds_keep_canonical_origins_and_reject_typos() {
    for declaration in ["export interface Shape {}", "export class Shape {}"] {
        let dir = TmpDir::new();
        write(dir.root(), "types.mts", declaration);
        write(
            dir.root(),
            "a.mts",
            "export type {Shape as Shared} from './types.mts';",
        );
        write(
            dir.root(),
            "b.mts",
            "import type {Shape as Local} from './types.mts'; export {Local as Shared};",
        );
        write(
            dir.root(),
            "barrel.mts",
            "export * from './a.mts'; export * from './b.mts';",
        );
        write(
            dir.root(),
            "main.mts",
            "export {Shared} from './barrel.mts'; console.log('ok');",
        );
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&typed()).unwrap();
        assert_eq!(observe(&result.product.artifacts), b"ok\n");
        write(
            dir.root(),
            "a.mts",
            "export type {Typo as Shared} from './types.mts';",
        );
        write(dir.root(), "barrel.mts", "export * from './a.mts';");
        assert!(session.build(&typed()).is_err());
    }
}

#[test]
fn type_only_wiring_explicit_namespace_aliases_preserve_identity_without_runtime_effects() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "types.mts",
        "console.log('never'); export interface Shape {}",
    );
    write(
        dir.root(),
        "a.mts",
        "export type * as Shared from './types.mts';",
    );
    write(
        dir.root(),
        "b.mts",
        "import type * as Local from './types.mts'; export {Local as Shared};",
    );
    write(
        dir.root(),
        "c.mts",
        "import * as Local from './types.mts'; export type {Local as Shared};",
    );
    write(
        dir.root(),
        "barrel.mts",
        "export * from './a.mts'; export * from './b.mts'; export * from './c.mts';",
    );
    write(
        dir.root(),
        "main.mts",
        "export {Shared} from './barrel.mts';",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&typed()).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"");
    assert!(result
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("never")));
    write(
        dir.root(),
        "main.mts",
        "import {Shared} from './barrel.mts'; console.log(Shared);",
    );
    assert!(format!("{:#}", session.build(&typed()).err().unwrap()).contains("executable value"));
    write(dir.root(), "a.mts", "export type {Missing as Shared};");
    write(dir.root(), "barrel.mts", "export * from './a.mts';");
    write(
        dir.root(),
        "main.mts",
        "export {Shared} from './barrel.mts';",
    );
    assert!(session.build(&typed()).is_err());
}

#[test]
fn type_only_wiring_values_win_and_typos_and_unresolved_runtime_requests_still_fail() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "types.mts",
        "export interface Shape {} export const Shape=42;",
    );
    write(
        dir.root(),
        "main.mts",
        "import {Shape} from './types.mts'; console.log(Shape);",
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&typed()).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"42\n");
    assert!(result.product.compatibility_warnings.is_empty());
    write(dir.root(), "main.mts", "export {Typo} from './types.mts';");
    assert!(session.build(&typed()).is_err());
    write(
        dir.root(),
        "main.mts",
        "import {Missing} from './missing.mts'; console.log(Missing);",
    );
    assert!(session.build(&typed()).is_err());
    write(
        dir.root(),
        "main.mts",
        "import {Missing} from './missing.mts'; export type T=Missing;",
    );
    assert!(session
        .build(&typed())
        .unwrap()
        .product
        .compatibility_warnings
        .is_empty());
}

#[test]
fn type_only_wiring_erased_provider_lifecycle_and_cache_modes() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mts",
            "import {Shape} from './types.mts'; export type T=Shape; console.log('ok');",
        );
        write(dir.root(), "types.mts", "export interface Shape {}");
        let mut config = typed();
        config.cache = cache;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        assert_eq!(cold.product.compatibility_warnings.len(), 1);
        assert_eq!(session.type_only_cache.extractions(), 2);
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        assert_eq!(noop.warnings, cold.warnings);
        assert_eq!(session.type_only_cache.extractions(), 2);
        let mut disk = BuildSession::new(dir.root()).unwrap();
        assert_eq!(disk.build(&config).unwrap().warnings, cold.warnings);
        assert_eq!(
            disk.type_only_cache.extractions(),
            if cache == CacheMode::ReadWrite { 0 } else { 2 }
        );
        let hash = session.sources[&dir.root().join("main.mts")]
            .record
            .source_hash();
        write(dir.root(), "types.mts", "export const Shape=1;");
        let changed = session.build(&config).unwrap();
        assert_eq!(
            session.sources[&dir.root().join("main.mts")]
                .record
                .source_hash(),
            hash
        );
        assert!(changed.product.compatibility_warnings.is_empty());
        assert!(!Rc::ptr_eq(&cold.product, &changed.product));
        fs::remove_file(dir.root().join("types.mts")).unwrap();
        assert!(session
            .build(&config)
            .unwrap()
            .product
            .compatibility_warnings
            .is_empty());
        write(dir.root(), "types.mts", "export type Shape=number;");
        assert_eq!(
            session
                .build(&config)
                .unwrap()
                .product
                .compatibility_warnings
                .len(),
            1
        );
    }
}

#[test]
fn type_only_wiring_raw_external_seams_and_native_exports_remain_executable() {
    for barrel in [
            "export {Shape,default as Removed,keep,defaultValue as default} from './types.mts';",
            "import {Shape,default as Removed,keep,defaultValue} from './types.mts'; export {Shape,Removed,keep,defaultValue as default};",
        ] {
            let dir = TmpDir::new();
            write(dir.root(), "types.mts", "console.log('provider'); export interface Shape {} export default interface D {} export const keep=42; export const defaultValue=7;");
            write(dir.root(), "barrel.mts", &format!("console.log('barrel'); {barrel}"));
            write(dir.root(), "main.mts", "import 'node:fs'; export {Shape,Removed,keep,default} from './barrel.mts';");
            let mut config = typed();
            config.externals.insert("node:fs".into());
            let result = BuildSession::new(dir.root()).unwrap().build(&config).unwrap();
            let entry = result.product.artifacts.entries["main"].to_string_lossy().replace('\\', "/");
            assert_eq!(observe_named(&result.product.artifacts, &format!(
                "import assert from 'node:assert/strict'; const ns=await import('./{entry}'); assert.deepEqual(Object.keys(ns),['default','keep']); assert.equal(ns.default,7); assert.equal(ns.keep,42);"
            )), b"provider\nbarrel\n");
        }
}

#[test]
fn type_only_wiring_explicit_type_exports_and_pure_namespace_have_no_effect_import() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "values.mts",
        "console.log('never'); export const Real=42;",
    );
    write(
        dir.root(),
        "barrel.mts",
        "export type {Real as Shape} from './values.mts';",
    );
    write(
        dir.root(),
        "main.mts",
        "export {Shape} from './barrel.mts';",
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&typed())
        .unwrap();
    assert_eq!(result.product.compatibility_warnings.len(), 1);
    let entry = result.product.artifacts.entries["main"]
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(observe_named(&result.product.artifacts, &format!(
            "import assert from 'node:assert/strict'; const ns=await import('./{entry}'); assert.deepEqual(Object.keys(ns),[]);"
        )), b"");
    assert!(result
        .product
        .artifacts
        .files
        .values()
        .all(|text| !text.contains("never")));
}

#[test]
fn type_only_wiring_resolution_settings_and_read_only_disk_replay_are_current() {
    let dir = TmpDir::new();
    write(dir.root(), "one.mts", "export interface Shape {}");
    write(dir.root(), "two.mts", "export const Shape=1;");
    write(
        dir.root(),
        "main.mts",
        "/*😀é*/ import {Shape as Alias} from './api'; export type T=Alias;",
    );
    let mut config = typed();
    config
        .resolver
        .alias
        .insert("./api".into(), "./one.mts".into());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(cold.product.compatibility_warnings.len(), 1);
    assert!(
        cold.product.compatibility_warnings[0].contains("main.mts:1:17"),
        "{:?}",
        cold.warnings
    );
    config.cache = CacheMode::ReadOnly;
    let mut disk = BuildSession::new(dir.root()).unwrap();
    let replay = disk.build(&config).unwrap();
    assert_eq!(replay.warnings, cold.warnings);
    assert_eq!(disk.type_only_cache.extractions(), 0);
    config
        .resolver
        .alias
        .insert("./api".into(), "./two.mts".into());
    assert!(session
        .build(&config)
        .unwrap()
        .product
        .compatibility_warnings
        .is_empty());
    config
        .resolver
        .alias
        .insert("./api".into(), "./one.mts".into());
    assert_eq!(
        session
            .build(&config)
            .unwrap()
            .product
            .compatibility_warnings
            .len(),
        1
    );
    config.loaders = config
        .loaders
        .with_override(".mts", Some(crate::loaders::Loader::Js))
        .unwrap();
    assert!(session.build(&config).is_err());
    config = typed();
    config
        .resolver
        .alias
        .insert("./api".into(), "./types.meta".into());
    write(dir.root(), "types.meta", "export interface Shape {}");
    config.loaders = config
        .loaders
        .with_override(".meta", Some(crate::loaders::Loader::Ts))
        .unwrap();
    assert_eq!(
        session
            .build(&config)
            .unwrap()
            .product
            .compatibility_warnings
            .len(),
        1
    );
    config.loaders = config
        .loaders
        .with_override(".meta", Some(crate::loaders::Loader::Js))
        .unwrap();
    assert!(session
        .build(&config)
        .unwrap()
        .product
        .compatibility_warnings
        .is_empty());
}

#[test]
fn type_only_wiring_erased_unused_real_js_never_reads_an_unneeded_companion() {
    let dir = TmpDir::new();
    write(dir.root(), "api.mjs", "export const Real=42;");
    write(dir.root(), "api.d.mts", "invalid unneeded declaration {{{");
    write(
        dir.root(),
        "main.mts",
        "import {Real} from './api.mjs'; export const keep=1;",
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&typed())
        .unwrap();
    assert!(result.product.compatibility_warnings.is_empty());
}

#[test]
fn type_only_wiring_lazy_original_routes_do_not_discover_unrelated_metadata() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mts",
        "import {Shape} from './types.mts'; export type T=Shape;",
    );
    write(dir.root(), "types.mts", "import {Unused} from './bad.mts'; export {Other} from './bad.mts'; export interface Shape {}");
    write(dir.root(), "bad.mts", "invalid unneeded TypeScript {{{");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&typed()).unwrap();
    assert_eq!(result.product.compatibility_warnings.len(), 1);
    assert_eq!(session.type_only_cache.extractions(), 2);
    assert!(!type_only_facts::sidecar(&dir.root().join("bad.mts")).exists());
    assert_eq!(observe(&result.product.artifacts), b"");
}
