use super::*;
use cache_flatbuffers::{type_only_wire as wire, WireTypeOnlyFacts};

fn original<'a>(session: &'a BuildSession, path: &Path) -> &'a WireTypeOnlyFacts {
    session.sources[path]
        .transpiled
        .as_ref()
        .unwrap()
        .original_facts
        .as_ref()
        .unwrap()
        .record()
        .unwrap()
}

#[test]
fn original_ts_facts_capture_erased_exports_routes_and_use_spans() {
    let dir = TmpDir::new();
    let source = r#"import { Shape as S, Query, Value, Unused, type Named } from "./dependency.mjs";
import type { Explicit } from "./missing-types";
interface Private { width: number }
type Mixed = string;
const Mixed = 1;
export { Private as Public, Mixed };
export type { Private as Marked };
export { Value as Routed } from "./dependency.mjs";
export type { Remote as TypedRoute } from "./missing-types";
export * from "./star.mjs";
export default interface DefaultShape { field: Explicit; named: Named }
type Model = S;
type QueryType = typeof Query;
type Both = typeof Value;
console.log(Value);
"#;
    write(dir.root(), "main.mts", source);
    write(dir.root(), "dependency.mjs", "export const Value=42;");
    write(dir.root(), "star.mjs", "export const star=1;");
    let path = dir.root().join("main.mts");
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&config).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"42\n");
    let record = original(&session, &path);
    assert!(record.backing().is_mapped());
    let facts = record.facts();
    for (name, presence, has_type) in [
        ("Public", wire::Presence::OnlyType, true),
        ("Mixed", wire::Presence::RuntimeValue, true),
        ("Marked", wire::Presence::OnlyType, true),
        ("default", wire::Presence::OnlyType, true),
        ("Routed", wire::Presence::Unresolved, false),
        ("TypedRoute", wire::Presence::OnlyType, true),
    ] {
        let export = facts
            .exports()
            .iter()
            .find(|e| e.name().value() == name)
            .unwrap();
        assert_eq!((export.presence(), export.has_type()), (presence, has_type));
    }
    let alias = facts
        .exports()
        .iter()
        .find(|e| e.name().value() == "Public")
        .unwrap();
    let local = alias.local().unwrap();
    assert_eq!(
        &source[local.span().start() as usize..local.span().end() as usize],
        "Private"
    );
    let route = facts
        .exports()
        .iter()
        .find(|e| e.name().value() == "Routed")
        .unwrap();
    assert_eq!(route.imported().unwrap().value(), "Value");
    assert_eq!(route.request().unwrap().value(), "./dependency.mjs");
    assert!(facts
        .exports()
        .iter()
        .any(|e| e.star() && e.request().unwrap().value() == "./star.mjs"));
    for (local, explicit, type_use, query, value_use) in [
        ("S", false, true, false, false),
        ("Query", false, true, true, false),
        ("Value", false, true, true, true),
        ("Unused", false, false, false, false),
        ("Explicit", true, true, false, false),
        ("Named", true, true, false, false),
    ] {
        let occurrence = facts
            .occurrences()
            .iter()
            .find(|o| {
                o.kind() == wire::OccurrenceKind::Import
                    && o.local().is_some_and(|n| n.value() == local)
            })
            .unwrap();
        assert_eq!(occurrence.explicit_type(), explicit);
        assert_eq!(occurrence.uses().iter().any(|u| u.type_use()), type_use);
        assert_eq!(occurrence.uses().iter().any(|u| u.value_as_type()), query);
        assert_eq!(occurrence.uses().iter().any(|u| u.value_use()), value_use);
        let name = occurrence.local().unwrap();
        assert_eq!(
            &source[name.span().start() as usize..name.span().end() as usize],
            local
        );
        for usage in occurrence.uses() {
            assert_eq!(
                &source[usage.span().start() as usize..usage.span().end() as usize],
                local
            );
        }
    }
    let generated = session.sources[&path].text.text(&path).unwrap();
    for erased in [
        "Private",
        "DefaultShape",
        "Query",
        "Unused",
        "missing-types",
    ] {
        assert!(!generated.contains(erased), "{}", &*generated);
    }
    assert!(!session.sources[&path].record.config().typescript());
}

#[test]
fn original_ts_facts_reuse_refresh_and_disk_cache_modes() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        let path = dir.root().join("main.mts");
        write(
            dir.root(),
            "main.mts",
            "export interface Before { x: string }\nconsole.log(42);",
        );
        let mut config = options();
        config.cache = cache;
        config.entries.get_mut("main").unwrap().path = "main.mts".into();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let cold = session.build(&config).unwrap();
        let first = original(&session, &path).clone();
        let js = session.sources[&path].record.source_hash();
        assert_eq!(session.type_only_cache.extractions(), 1);
        let noop = session.build(&config).unwrap();
        assert!(Rc::ptr_eq(&cold.product, &noop.product));
        assert_eq!(
            original(&session, &path).backing().as_ptr(),
            first.backing().as_ptr()
        );
        assert_eq!(session.type_only_cache.extractions(), 1);
        assert_eq!(noop.counters.source_parses, 0);
        let mut disk = BuildSession::new(dir.root()).unwrap();
        let loaded = disk.build(&config).unwrap();
        assert_eq!(loaded.product.artifacts, cold.product.artifacts);
        assert_eq!(
            disk.type_only_cache.extractions(),
            usize::from(cache != CacheMode::ReadWrite)
        );
        assert_eq!(
            type_only_facts::sidecar(&path).exists(),
            cache == CacheMode::ReadWrite
        );
        if cache == CacheMode::ReadWrite {
            let sidecar = fs::read(type_only_facts::sidecar(&path)).unwrap();
            for mode in [CacheMode::ReadOnly, CacheMode::Disabled] {
                let mut mode_config = config.clone();
                mode_config.cache = mode;
                let mut fresh = BuildSession::new(dir.root()).unwrap();
                fresh.build(&mode_config).unwrap();
                assert_eq!(
                    fresh.type_only_cache.extractions(),
                    usize::from(mode == CacheMode::Disabled)
                );
                assert_eq!(fs::read(type_only_facts::sidecar(&path)).unwrap(), sidecar);
            }
        }
        write(
            dir.root(),
            "main.mts",
            "export interface After { y: number }\nconsole.log(42);",
        );
        let changed = session.build(&config).unwrap();
        assert_eq!(session.sources[&path].record.source_hash(), js);
        assert_eq!(session.type_only_cache.extractions(), 2);
        assert_eq!(
            original(&session, &path)
                .facts()
                .exports()
                .get(0)
                .name()
                .value(),
            "After"
        );
        assert_eq!(first.facts().exports().get(0).name().value(), "Before");
        assert_eq!(observe(&changed.product.artifacts), b"42\n");
        let refreshed = original(&session, &path).clone();
        session.build(&config).unwrap();
        assert_eq!(
            original(&session, &path).backing().as_ptr(),
            refreshed.backing().as_ptr()
        );
        let mut latest = BuildSession::new(dir.root()).unwrap();
        latest.build(&config).unwrap();
        assert_eq!(
            original(&latest, &path)
                .facts()
                .exports()
                .get(0)
                .name()
                .value(),
            "After"
        );
        assert_eq!(
            latest.type_only_cache.extractions(),
            usize::from(cache != CacheMode::ReadWrite)
        );
        if cache == CacheMode::ReadWrite {
            fs::write(type_only_facts::sidecar(&path), b"corrupt").unwrap();
            let mut corrupted = BuildSession::new(dir.root()).unwrap();
            corrupted.build(&config).unwrap();
            assert_eq!(corrupted.type_only_cache.extractions(), 1);
            assert_eq!(
                original(&corrupted, &path)
                    .facts()
                    .exports()
                    .get(0)
                    .name()
                    .value(),
                "After"
            );
        }
    }
}

#[test]
fn original_ts_facts_selected_syntax_and_package_settings() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(
        dir.root(),
        "main.js",
        "export type Shape = string; console.log(42);",
    );
    let path = dir.root().join("main.js");
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.js".into();
    config.loaders = config
        .loaders
        .with_override(".js", Some(crate::loaders::Loader::Ts))
        .unwrap();
    let mut session = BuildSession::new(dir.root()).unwrap();
    session.build(&config).unwrap();
    assert_eq!(
        original(&session, &path)
            .facts()
            .exports()
            .get(0)
            .name()
            .value(),
        "Shape"
    );
    let first = original(&session, &path).clone();
    config.loaders = config
        .loaders
        .with_override(".js", Some(crate::loaders::Loader::Tsx))
        .unwrap();
    session.build(&config).unwrap();
    assert_eq!(session.type_only_cache.extractions(), 2);
    assert_ne!(
        original(&session, &path).backing().as_ptr(),
        first.backing().as_ptr()
    );
    write(dir.root(), "plain.ts", "console.log(7);");
    config.entries.get_mut("main").unwrap().path = "plain.ts".into();
    config.loaders = config
        .loaders
        .with_override(".ts", Some(crate::loaders::Loader::Js))
        .unwrap();
    session.build(&config).unwrap();
    assert!(session.sources[&dir.root().join("plain.ts")]
        .transpiled
        .is_none());
    assert!(!type_only_facts::sidecar(&dir.root().join("plain.ts")).exists());
    assert_eq!(session.type_only_cache.extractions(), 2);
    // Explicit module syntax stays identical when package format changes, but
    // the original metadata settings still include that package observation.
    write(
        dir.root(),
        "main.mts",
        "export interface Shape {}\nconsole.log(42);",
    );
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    session.build(&config).unwrap();
    let before = original(&session, &dir.root().join("main.mts")).clone();
    write(dir.root(), "package.json", r#"{"type":"commonjs"}"#);
    session.build(&config).unwrap();
    assert_eq!(session.type_only_cache.extractions(), 4);
    assert_ne!(
        original(&session, &dir.root().join("main.mts"))
            .backing()
            .as_ptr(),
        before.backing().as_ptr()
    );
}

#[test]
fn original_ts_facts_javascript_selection_drops_old_typed_owner() {
    for loader in [crate::loaders::Loader::Js, crate::loaders::Loader::Jsx] {
        let dir = TmpDir::new();
        write(dir.root(), "main.ts", "console.log(42);");
        let path = dir.root().join("main.ts");
        let mut config = options();
        config.entries.get_mut("main").unwrap().path = "main.ts".into();
        let mut session = BuildSession::new(dir.root()).unwrap();
        session.build(&config).unwrap();
        assert_eq!(session.type_only_cache.extractions(), 1);
        assert!(original(&session, &path).backing().is_mapped());
        config.loaders = config.loaders.with_override(".ts", Some(loader)).unwrap();
        let changed = session.build(&config).unwrap();
        assert_eq!(observe(&changed.product.artifacts), b"42\n");
        assert!(session.sources[&path]
            .transpiled
            .as_ref()
            .is_none_or(|t| t.original_facts.is_none()));
        assert_eq!(session.type_only_cache.extractions(), 1);
        let mut disk = BuildSession::new(dir.root()).unwrap();
        disk.build(&config).unwrap();
        assert_eq!(disk.type_only_cache.extractions(), 0);
        assert!(disk.sources[&path]
            .transpiled
            .as_ref()
            .is_none_or(|t| t.original_facts.is_none()));
    }
}

#[test]
fn original_ts_facts_survive_enum_overlay_restore_and_regeneration() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import {E} from './api.js'; console.log(E.A);",
        );
        write(
            dir.root(),
            "api.js",
            "export interface Shape {}\nexport const keep=1;",
        );
        write(
            dir.root(),
            "api.d.ts",
            "export declare const enum E { A='first' }",
        );
        let path = dir.root().join("api.js");
        let mut config = options();
        config.cache = cache;
        config.loaders = config
            .loaders
            .with_override(".js", Some(crate::loaders::Loader::Ts))
            .unwrap();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let first = session.build(&config).unwrap();
        let original = original(&session, &path).clone();
        assert_eq!(observe(&first.product.artifacts), b"first\n");
        write(
            dir.root(),
            "api.d.ts",
            "export declare const enum E { A='second' }",
        );
        let regenerated = session.build(&config).unwrap();
        assert_eq!(observe(&regenerated.product.artifacts), b"second\n");
        assert_eq!(
            self::original(&session, &path).backing().as_ptr(),
            original.backing().as_ptr()
        );
        assert_eq!(
            self::original(&session, &path)
                .facts()
                .exports()
                .get(0)
                .name()
                .value(),
            "Shape"
        );
        write(dir.root(), "api.d.ts", "export interface E {}");
        assert!(session.build(&config).is_err());
        assert_eq!(
            self::original(&session, &path).backing().as_ptr(),
            original.backing().as_ptr()
        );
        assert!(!session.sources[&path]
            .text
            .text(&path)
            .unwrap()
            .contains("second"));
        write(
            dir.root(),
            "api.d.ts",
            "export declare const enum E { A='third' }",
        );
        let restored = session.build(&config).unwrap();
        assert_eq!(observe(&restored.product.artifacts), b"third\n");
        assert_eq!(
            self::original(&session, &path).backing().as_ptr(),
            original.backing().as_ptr()
        );
    }
}

#[test]
fn original_ts_facts_noop_readmission_rejects_late_erased_source_edits() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mts",
        "export interface Before {}\nconsole.log(42);",
    );
    let path = dir.root().join("main.mts");
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    config.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(1).unwrap());
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    let changed = path.clone();
    prepare::AFTER_PREPARE.with(|hook| {
        *hook.borrow_mut() = Some(std::sync::Arc::new(move |path| {
            if path == changed {
                fs::write(path, "export interface Changed {}\nconsole.log(42);").unwrap();
            }
        }));
    });
    let result = session.build(&config);
    prepare::AFTER_PREPARE.with(|hook| *hook.borrow_mut() = None);
    assert!(result.is_err());
    assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &first.product));
    session.build(&config).unwrap();
    assert_eq!(
        original(&session, &path)
            .facts()
            .exports()
            .get(0)
            .name()
            .value(),
        "Changed"
    );
}
