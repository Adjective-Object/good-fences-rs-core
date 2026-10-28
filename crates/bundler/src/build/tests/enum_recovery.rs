use super::*;

#[test]
fn const_enum_review_erased_original_input_edits_refresh_retained_overlays() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        let path = dir.root().join("api.js");
        write(
            dir.root(),
            "api.d.ts",
            "export declare const enum E { A='enum' }",
        );
        write(
            dir.root(),
            "main.mjs",
            "import {E,keep} from './api.js'; console.log(E.A,keep);",
        );
        let mut config = options();
        config.cache = cache;
        config.output.source_maps = false;
        config.loaders = config
            .loaders
            .with_override(".js", Some(crate::loaders::Loader::Ts))
            .unwrap();
        let mut session = BuildSession::new(dir.root()).unwrap();
        let mut prepared_hash = None;
        for interface in [
            "interface Erased { first: string }",
            "interface Erased { renamed: number; optional?: never }",
            "interface Erased { final: readonly boolean[] }",
        ] {
            let original = format!("{interface}\nexport const keep=1;");
            write(dir.root(), "api.js", &original);
            let result = session.build(&config).unwrap();
            assert_eq!(observe(&result.product.artifacts), b"enum 1\n");
            let source = &session.sources[&path];
            let original_facts = source
                .transpiled
                .as_ref()
                .unwrap()
                .original_facts
                .as_ref()
                .unwrap();
            assert_eq!(
                original_facts.input().content_seal(),
                Some(&cache_flatbuffers::hash128(original.as_bytes()))
            );
            let base_facts = session.enum_bases[&path]
                .transpiled
                .as_ref()
                .unwrap()
                .original_facts
                .as_ref()
                .unwrap();
            assert_eq!(
                original_facts.record().unwrap().backing().as_ptr(),
                base_facts.record().unwrap().backing().as_ptr()
            );
            let local = original_facts
                .record()
                .unwrap()
                .facts()
                .locals()
                .iter()
                .find(|local| local.name().value() == "Erased")
                .unwrap();
            assert_eq!(
                &original[local.span().start() as usize..local.span().end() as usize],
                interface
            );
            assert_eq!(
                local.presence(),
                cache_flatbuffers::type_only_wire::Presence::OnlyType
            );
            assert!(!original_facts
                .record()
                .unwrap()
                .facts()
                .exports()
                .iter()
                .any(|e| e.name().value() == "E"));
            assert_eq!(source.text.read_input(&path).unwrap(), original.as_bytes());
            assert_eq!(
                session.enum_bases[&path].text.read_input(&path).unwrap(),
                original.as_bytes()
            );
            let text = source.text.text(&path).unwrap();
            source.record.to_parsed_reporting_policies(&text).unwrap();
            if let Some(previous) = prepared_hash {
                assert_eq!(source.record.source_hash(), previous);
            }
            prepared_hash = Some(source.record.source_hash());
            let noop = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&result.product, &noop.product));
            assert_eq!(noop.counters.source_parses, 0);
            assert_eq!(noop.warnings, result.warnings);
        }
        let mut disk_session = BuildSession::new(dir.root()).unwrap();
        let disk = disk_session.build(&config).unwrap();
        assert_eq!(observe(&disk.product.artifacts), b"enum 1\n");
        let typed = disk_session.sources[&path]
            .transpiled
            .as_ref()
            .unwrap()
            .original_facts
            .as_ref()
            .unwrap();
        assert_eq!(
            typed.input().content_seal(),
            Some(&cache_flatbuffers::hash128(
                fs::read(&path).unwrap().as_slice()
            ))
        );
        assert!(!typed
            .record()
            .unwrap()
            .facts()
            .exports()
            .iter()
            .any(|e| e.name().value() == "E"));
    }
}

#[test]
fn const_enum_review_consumed_cjs_replacements_never_read_companions() {
    for javascript in [
        "const alias = module.exports = {}; alias.E = {A:'real'};",
        "const save=x=>x; const alias=save(module.exports = {}); alias.E={A:'real'};",
        "function replace(){return module.exports = {}} const alias=replace(); alias.E={A:'real'};",
        "const replace=()=>module.exports = {}; const alias=replace(); alias.E={A:'real'};",
        "const target={}; target.alias=module.exports = {}; target.alias.E={A:'real'};",
    ] {
        for unreadable in [false, true] {
            let dir = TmpDir::new();
            write(dir.root(), "api.cjs", javascript);
            write(dir.root(), "main.mjs", "import {E} from './api.cjs'; import * as kit from './api.cjs'; console.log(E.A,kit['E'].A);");
            if unreadable {
                fs::create_dir(dir.root().join("api.d.cts")).unwrap();
            } else {
                write(dir.root(), "api.d.cts", "invalid unneeded declaration {{{");
            }
            let mut session = BuildSession::new(dir.root()).unwrap();
            let result = session.build(&options()).unwrap();
            assert_eq!(observe(&result.product.artifacts), b"real real\n");
            assert_eq!(session.type_only_cache.extractions(), 0);
            assert!(result.warnings.is_empty());
            assert!(Rc::ptr_eq(
                &result.product,
                &session.build(&options()).unwrap().product
            ));
            let mut disk = BuildSession::new(dir.root()).unwrap();
            assert_eq!(
                disk.build(&options()).unwrap().product.artifacts,
                result.product.artifacts
            );
            assert_eq!(disk.type_only_cache.extractions(), 0);
        }
    }
}

#[test]
fn const_enum_review_ambiguous_origins_cannot_be_hidden_by_direct_recovery() {
    for namespace in [false, true] {
        for direct_first in [false, true] {
            let dir = TmpDir::new();
            let (first, second) = if direct_first { ("z", "a") } else { ("a", "z") };
            for name in [first, second] {
                write(dir.root(), &format!("{name}.mjs"), "export const keep=1;");
                write(
                    dir.root(),
                    &format!("{name}.d.mts"),
                    &format!("export declare const enum E {{ A='{name}' }}"),
                );
            }
            let (left, right) = if namespace {
                (second, first)
            } else {
                (first, second)
            };
            write(
                dir.root(),
                "barrel.mjs",
                &format!("export * from './{left}.mjs'; export * from './{right}.mjs';"),
            );
            let direct = format!("import {{E as direct}} from './{first}.mjs';");
            let barrel = if namespace {
                "import * as kit from './barrel.mjs';"
            } else {
                "import {E as fromBarrel} from './barrel.mjs';"
            };
            let consumer = if namespace {
                "kit['E'].A"
            } else {
                "fromBarrel.A"
            };
            let imports = if direct_first {
                format!("{direct} {barrel}")
            } else {
                format!("{barrel} {direct}")
            };
            write(
                dir.root(),
                "main.mjs",
                &format!("{imports} console.log(direct.A,{consumer});"),
            );
            let mut session = BuildSession::new(dir.root()).unwrap();
            for _ in 0..2 {
                let error = session
                    .build(&options())
                    .err()
                    .expect("competing declaration origins must stay ambiguous");
                assert!(format!("{error:#}").contains("ambiguous"), "{error:#}");
                assert!(session.last.is_none());
            }
            fs::remove_file(dir.root().join(format!("{second}.d.mts"))).unwrap();
            let repaired = session.build(&options()).unwrap();
            assert_eq!(
                observe(&repaired.product.artifacts),
                format!("{first} {first}\n").as_bytes()
            );
            assert!(Rc::ptr_eq(
                &repaired.product,
                &session.build(&options()).unwrap().product
            ));
            write(
                dir.root(),
                &format!("{second}.d.mts"),
                "export declare const enum E { A='competing' }",
            );
            assert!(session.build(&options()).is_err());
            assert!(Rc::ptr_eq(
                session.last.as_ref().unwrap(),
                &repaired.product
            ));
            write(
                dir.root(),
                &format!("{first}.mjs"),
                "export const E={A:'real'};",
            );
            for name in [first, second] {
                write(
                    dir.root(),
                    &format!("{name}.d.mts"),
                    "invalid unused declaration {{{",
                );
            }
            let real = session.build(&options()).unwrap();
            assert_eq!(observe(&real.product.artifacts), b"real real\n");
            assert!(real.warnings.is_empty());
            assert!(Rc::ptr_eq(
                &real.product,
                &session.build(&options()).unwrap().product
            ));
            let mut disk = BuildSession::new(dir.root()).unwrap();
            assert_eq!(
                disk.build(&options()).unwrap().product.artifacts,
                real.product.artifacts
            );
            assert_eq!(disk.type_only_cache.extractions(), 0);
        }
    }
}

#[test]
fn const_enum_recovery_shared_alias_namespace_numeric_merged_and_mutation() {
    let dir = TmpDir::new();
    write(dir.root(), "api.mjs", "const E = 'private'; export let live = 3; export const change = () => ++live; export default 'default';");
    write(
        dir.root(),
        "api.d.mts",
        r#"
        declare const enum Private { A = 4, B, Text = "text", "quoted-key" = "quoted", "__proto__" = "own" }
        declare const enum Private { C = 8, D }
        export { Private as E, Private as Alias };
    "#,
    );
    write(
        dir.root(),
        "one.mjs",
        "import {E} from './api.mjs'; E.changed = 42; export {E};",
    );
    write(
        dir.root(),
        "main.mjs",
        r#"
        import {E as first} from './one.mjs';
        import value, {E, Alias, live, change} from './api.mjs';
        import * as kit from './api.mjs';
        change();
        console.log(JSON.stringify([E===first,E===Alias,E===kit.E,E.changed,E.A,E.B,E[4],E[5],E.Text,E.C,E.D,E[9],E["quoted-key"],E["__proto__"],live,value]));
    "#,
    );
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&options()).unwrap();
    assert_eq!(
        observe(&cold.product.artifacts),
        br#"[true,true,true,42,4,5,"A","B","text",8,9,"D","quoted","own",4,"default"]
"#
    );
    assert_eq!(session.type_only_cache.extractions(), 1);
    let source = session.sources[&dir.root().join("api.mjs")]
        .text
        .text(&dir.root().join("api.mjs"))
        .unwrap();
    assert_eq!(source.matches("const __gf_const_enum_").count(), 1);
    assert_eq!(cold.warnings.len(), 5);
    assert_eq!(cold.warnings, cold.product.compatibility_warnings);
    let noop = session.build(&options()).unwrap();
    assert!(Rc::ptr_eq(&cold.product, &noop.product));
    assert_eq!(noop.counters.source_parses, 0);
    assert_eq!(noop.warnings, cold.warnings);
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(cold.product.artifacts, disk.product.artifacts);
    assert_eq!(cold.warnings, disk.warnings);
}

#[test]
fn const_enum_recovery_static_namespace_and_literal_brackets_are_lazy() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import * as kit from './api.mjs'; console.log(kit.E.A, kit.F.B, kit['F'].B);",
    );
    write(dir.root(), "api.mjs", "export const real = 1;");
    write(dir.root(), "api.d.mts", "export declare const enum E { A='a' } export declare const enum F { B=2 } export declare const enum Unused { Bad=call() }");
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&result.product.artifacts), b"a 2 2\n");
    assert_eq!(result.warnings.len(), 3);
    assert!(result.warnings.iter().all(|w| w.contains("main.mjs:1:")));
    assert!(!result
        .product
        .artifacts
        .files
        .values()
        .any(|s| s.contains("Unused")));
}

#[test]
fn const_enum_recovery_real_export_wins_without_companion_access() {
    for (javascript, declaration) in [
        (
            "export const E = { A: 'real' };",
            "not valid TypeScript {{{",
        ),
        (
            "export const E = { A: 'real' };",
            "export declare const enum E { A='wrong' }",
        ),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", "import {E} from './api.mjs'; import * as kit from './api.mjs'; console.log(E.A, kit.E.A);");
        write(dir.root(), "api.mjs", javascript);
        write(dir.root(), "api.d.mts", declaration);
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&options()).unwrap();
        assert_eq!(observe(&result.product.artifacts), b"real real\n");
        assert!(result.warnings.is_empty());
        assert_eq!(session.type_only_cache.extractions(), 0);
        assert!(!type_only_facts::sidecar(&dir.root().join("api.d.mts")).exists());
    }
}

#[test]
fn const_enum_recovery_declaration_only_add_edit_delete_and_failed_attempts() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import {E} from './api.mjs'; console.log(E.A);",
        );
        write(dir.root(), "api.mjs", "export const unrelated = 1;");
        let mut config = options();
        config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
        config.cache = cache;
        let mut session = BuildSession::new(dir.root()).unwrap();
        assert!(session.build(&config).is_err());
        write(
            dir.root(),
            "api.d.mts",
            "export declare const enum E { A='first' }",
        );
        let first = session.build(&config).unwrap();
        assert_eq!(observe(&first.product.artifacts), b"first\n");
        assert!(Rc::ptr_eq(
            &first.product,
            &session.build(&config).unwrap().product
        ));
        write(
            dir.root(),
            "api.d.mts",
            "export declare const enum E { A='second' }",
        );
        let second = session.build(&config).unwrap();
        assert_eq!(observe(&second.product.artifacts), b"second\n");
        assert!(second.invalidated.contains(&dir.root().join("api.mjs")));
        assert!(second.invalidated.contains(&dir.root().join("main.mjs")));
        fs::remove_file(dir.root().join("api.d.mts")).unwrap();
        let error = session.build(&config).err().unwrap();
        assert!(format!("{error:#}").contains("missing Value export"));
        assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &second.product));
        assert!(session.build(&config).is_err());
        write(
            dir.root(),
            "api.d.mts",
            "export declare const enum E { A='third' }",
        );
        let restored = session.build(&config).unwrap();
        assert_eq!(observe(&restored.product.artifacts), b"third\n");
        assert_eq!(restored.warnings.len(), 1);
        let disk = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(restored.product.artifacts, disk.product.artifacts);
    }
}

#[test]
fn const_enum_recovery_barrels_renames_cycles_and_ambiguous_stars() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {Renamed} from './a.mjs'; console.log(Renamed.A);",
    );
    write(dir.root(), "a.mjs", "export * from './b.mjs';");
    write(
        dir.root(),
        "b.mjs",
        "export * from './a.mjs'; export {E as Renamed} from './api.mjs';",
    );
    write(dir.root(), "api.mjs", "export const other = 1;");
    write(
        dir.root(),
        "api.d.mts",
        "export declare const enum E { A='barrel' }",
    );
    let build = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&build.product.artifacts), b"barrel\n");
    assert!(build
        .warnings
        .iter()
        .any(|w| w.contains("main.mjs") && w.contains("Renamed")));
    write(
        dir.root(),
        "a.mjs",
        "export * from './api.mjs'; export * from './other.mjs';",
    );
    write(dir.root(), "other.mjs", "export const other = 2;");
    write(
        dir.root(),
        "other.d.mts",
        "export declare const enum E { A='other' }",
    );
    write(
        dir.root(),
        "main.mjs",
        "import {E} from './a.mjs'; console.log(E.A);",
    );
    assert!(BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .is_err());
    write(dir.root(), "api.mjs", "export const E=1;");
    write(dir.root(), "other.mjs", "export const E=2;");
    let error = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("ambiguous"));
}

#[test]
fn const_enum_recovery_never_suppresses_nonconst_unknown_unsupported_or_cycles() {
    for declaration in [
        "export interface E {}",
        "export declare enum E { A=1 }",
        "export declare const enum Different { A=1 }",
        "export declare const enum E { A=fn() }",
        "export declare const enum E { A=B, B=A }",
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import {E} from './api.mjs'; console.log(E.A);",
        );
        write(dir.root(), "api.mjs", "export const real=1;");
        write(dir.root(), "api.d.mts", declaration);
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&options())
            .err()
            .unwrap();
        assert!(
            format!("{error:#}").contains(if declaration.starts_with("export interface") {
                "used as an executable value"
            } else {
                "missing Value export"
            }),
            "{error:#}"
        );
    }
}

#[test]
fn const_enum_recovery_hygiene_and_alias_only_default() {
    let dir = TmpDir::new();
    write(dir.root(), "api.mjs", "const E = 'private'; const __gf_const_enum_0 = 'occupied'; export const get = () => __gf_const_enum_0_ + E;");
    write(
        dir.root(),
        "api.d.mts",
        "declare const enum E { A='enum' } export {E as default};",
    );
    write(dir.root(), "main.mjs", "globalThis.__gf_const_enum_0_ = 'free-'; import E, {get} from './api.mjs'; console.log(E.A, get());");
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&result.product.artifacts), b"enum free-private\n");
}

#[test]
fn const_enum_recovery_only_selected_script_loaders_and_never_declaration_remaps() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {E} from './api.js'; console.log(E.A);",
    );
    write(dir.root(), "api.js", "{}");
    write(
        dir.root(),
        "api.d.ts",
        "export declare const enum E { A=1 }",
    );
    for loader in [crate::loaders::Loader::Json, crate::loaders::Loader::File] {
        let mut config = options();
        config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
        config.loaders = config.loaders.with_override(".js", Some(loader)).unwrap();
        let mut session = BuildSession::new(dir.root()).unwrap();
        assert!(session.build(&config).is_err());
        assert_eq!(session.type_only_cache.extractions(), 0);
    }
    let mut config = options();
    config.loaders = config
        .loaders
        .with_override(".d.ts", Some(crate::loaders::Loader::Js))
        .unwrap();
    config.entries.get_mut("main").unwrap().path = "api.d.ts".into();
    assert!(BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .is_err());
}

#[test]
fn const_enum_recovery_cjs_preserves_lazy_require_timing_node_and_browser() {
    for browser in [false, true] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "api.cjs",
            "globalThis.events.push('api'); exports.other='kept';",
        );
        write(
            dir.root(),
            "api.d.cts",
            "export declare const enum E { A='cjs' }",
        );
        write(dir.root(), "main.mjs", "globalThis.events=[]; console.log('before',events.length); const {E}=await import('./request.mjs'); const kit=require('./api.cjs'); console.log(E.A,kit.E===E,kit.other,events.join(','));");
        write(
            dir.root(),
            "request.mjs",
            "import {E} from './api.cjs'; export {E};",
        );
        let mut config = options();
        if browser {
            config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
        }
        let result = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .unwrap();
        assert_eq!(
            observe(&result.product.artifacts),
            b"before 0\ncjs true kept api\n"
        );
    }
}

#[test]
fn const_enum_recovery_mixed_cjs_surfaces_preserve_values_identity_and_activation() {
    for surface in [
        "exports.keep=1; exports.inc=()=>++exports.keep;",
        "module.exports.keep=1; module.exports.inc=()=>++module.exports.keep;",
        "module.exports={keep:1,inc:()=>++module.exports.keep};",
        "module.exports={keep:1,inc(){return ++module.exports.keep}};",
        "module['exports']={'keep':1,['inc']:()=>++module.exports['keep']};",
    ] {
        for browser in [false, true] {
            let dir = TmpDir::new();
            write(
                dir.root(),
                "api.cjs",
                &format!("globalThis.events.push('api'); const Shared='private'; {surface}"),
            );
            write(dir.root(), "api.d.cts", "declare const enum Shared { A='cjs', N=3, Next } export {Shared as E, Shared as Alias};");
            write(dir.root(), "request.mjs", "import value,{E,Alias,keep,inc} from './api.cjs'; import * as kit from './api.cjs'; E.changed=42; inc(); export {E}; export const result=[E===Alias,E===kit.E,E===kit['Alias'],E.changed,E.N,E.Next,E[4],keep,value.keep];");
            write(dir.root(), "main.mjs", "globalThis.events=[]; console.log('before',events.length); const {E,result}=await import('./request.mjs'); const kit=require('./api.cjs'); console.log(JSON.stringify([E.A,kit.E===E,kit.Alias===E,kit.keep,...result]),events.join(','));");
            let mut config = options();
            config.output.source_maps = true;
            if browser {
                config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
            }
            let mut session = BuildSession::new(dir.root()).unwrap();
            let cold = session.build(&config).unwrap();
            assert_eq!(
                observe(&cold.product.artifacts),
                b"before 0\n[\"cjs\",true,true,2,true,true,true,42,3,4,\"Next\",1,2] api\n",
                "{surface}"
            );
            assert_eq!(cold.warnings.len(), 4);
            assert_eq!(cold.warnings, cold.product.compatibility_warnings);
            assert!(cold.warnings.iter().all(|w| w.contains("request.mjs:1:")
                && w.contains("api.d.cts")
                && w.contains("api.cjs")));
            assert_eq!(session.type_only_cache.extractions(), 1);
            let text = session.sources[&dir.root().join("api.cjs")]
                .text
                .text(&dir.root().join("api.cjs"))
                .unwrap();
            assert_eq!(text.matches("const __gf_const_enum_").count(), 1);
            assert!(!text.contains("export {"));
            let noop = session.build(&config).unwrap();
            assert!(Rc::ptr_eq(&cold.product, &noop.product));
            assert_eq!(noop.counters.source_parses, 0);
            assert_eq!(noop.warnings, cold.warnings);
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(disk.product.artifacts, cold.product.artifacts);
            assert_eq!(disk.warnings, cold.warnings);
        }
    }
}

#[test]
fn const_enum_recovery_cjs_potential_names_never_read_unneeded_companions() {
    for (surface, name) in [
        ("exports.E={A:'real'};", "E"),
        ("module.exports.E={A:'real'};", "E"),
        ("module.exports={E:{A:'real'},keep:1};", "E"),
        ("module['exports']['E']={A:'real'};", "E"),
        ("exports.E=undefined;", "E"),
        ("if (false) exports.E={A:'not-executed'};", "E"),
        ("exports.keep=1;", "toString"),
    ] {
        for unreadable in [false, true] {
            let dir = TmpDir::new();
            write(dir.root(), "api.cjs", surface);
            let consumer = if name == "toString" {
                "import {toString as E} from './api.cjs'; console.log(typeof E==='function');"
                    .to_owned()
            } else {
                format!("import {{{name} as E}} from './api.cjs'; import * as kit from './api.cjs'; console.log(E===kit['{name}']);")
            };
            write(dir.root(), "main.mjs", &consumer);
            if unreadable {
                fs::create_dir(dir.root().join("api.d.cts")).unwrap();
            } else {
                write(dir.root(), "api.d.cts", "invalid unneeded declaration {{{");
            }
            let mut session = BuildSession::new(dir.root()).unwrap();
            let cold = session.build(&options()).unwrap();
            assert_eq!(observe(&cold.product.artifacts), b"true\n");
            assert!(cold.warnings.is_empty());
            assert_eq!(session.type_only_cache.extractions(), 0);
            let noop = session.build(&options()).unwrap();
            assert!(Rc::ptr_eq(&cold.product, &noop.product));
            let mut disk = BuildSession::new(dir.root()).unwrap();
            assert_eq!(
                disk.build(&options()).unwrap().product.artifacts,
                cold.product.artifacts
            );
            assert_eq!(disk.type_only_cache.extractions(), 0);
        }
    }
}

#[test]
fn const_enum_recovery_unknown_cjs_surfaces_remain_unchanged() {
    for surface in [
        "exports[globalThis.dynamicKey]={A:'dynamic'};",
        "globalThis.saved=exports; exports.keep=1;",
        "module.exports=JSON.parse('{\"keep\":1}');",
        "module.exports={...{keep:1}};",
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "api.cjs", surface);
        write(dir.root(), "api.d.cts", "invalid unneeded declaration {{{");
        write(dir.root(), "main.mjs", "import {E} from './api.cjs'; import * as kit from './api.cjs'; console.log(E===kit['E'],!!E);");
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&options()).unwrap();
        assert_eq!(observe(&result.product.artifacts), b"true false\n");
        assert!(result.warnings.is_empty());
        assert_eq!(session.type_only_cache.extractions(), 0);
    }
}

#[test]
fn const_enum_recovery_bracket_only_demand_and_cjs_declaration_freshness() {
    for (script, declaration, original) in [
        ("api.mjs", "api.d.mts", "export const keep=1;"),
        ("api.cjs", "api.d.cts", "exports.keep=1;"),
    ] {
        for cache in [
            CacheMode::ReadWrite,
            CacheMode::ReadOnly,
            CacheMode::Disabled,
        ] {
            let dir = TmpDir::new();
            write(dir.root(), script, original);
            write(
                dir.root(),
                "main.mjs",
                &format!("import * as kit from './{script}'; console.log(kit['E']?.A,kit.keep);"),
            );
            let mut config = options();
            config.cache = cache;
            let mut session = BuildSession::new(dir.root()).unwrap();
            let missing = session.build(&config).unwrap();
            assert_eq!(observe(&missing.product.artifacts), b"undefined 1\n");
            write(
                dir.root(),
                declaration,
                "export declare const enum E { A='first' }",
            );
            let first = session.build(&config).unwrap();
            assert_eq!(observe(&first.product.artifacts), b"first 1\n");
            assert_eq!(first.warnings.len(), 1);
            assert!(first.warnings[0].contains("main.mjs:1:"));
            assert!(Rc::ptr_eq(
                &first.product,
                &session.build(&config).unwrap().product
            ));
            write(
                dir.root(),
                declaration,
                "export declare const enum E { A='second' }",
            );
            let second = session.build(&config).unwrap();
            assert_eq!(observe(&second.product.artifacts), b"second 1\n");
            write(dir.root(), declaration, "invalid edited declaration {{{");
            assert!(session.build(&config).is_err());
            assert!(Rc::ptr_eq(session.last.as_ref().unwrap(), &second.product));
            fs::remove_file(dir.root().join(declaration)).unwrap();
            let deleted = session.build(&config).unwrap();
            assert_eq!(observe(&deleted.product.artifacts), b"undefined 1\n");
            assert!(deleted.warnings.is_empty());
            assert!(!session.sources[&dir.root().join(script)]
                .text
                .text(&dir.root().join(script))
                .unwrap()
                .contains("__gf_const_enum"));
            write(
                dir.root(),
                declaration,
                "export declare const enum E { A='restored' }",
            );
            let restored = session.build(&config).unwrap();
            assert_eq!(observe(&restored.product.artifacts), b"restored 1\n");
            let disk = BuildSession::new(dir.root())
                .unwrap()
                .build(&config)
                .unwrap();
            assert_eq!(restored.product.artifacts, disk.product.artifacts);
            assert_eq!(restored.warnings, disk.warnings);
        }
    }
}

#[test]
fn const_enum_recovery_warnings_use_existing_original_compiler_locations() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mts", "interface Gone { field: string }\nconst typed: number = 1;\nimport {E} from './api.mjs';\nconsole.log(E.A, typed);");
    write(dir.root(), "api.mjs", "export const real=1;");
    write(
        dir.root(),
        "api.d.mts",
        "export declare const enum E { A='value' }",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    config.output.source_maps = true;
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(result.warnings.len(), 1);
    assert!(
        result.warnings[0].contains("main.mts:3:"),
        "{}",
        result.warnings[0]
    );
    assert!(!result.warnings[0].contains("prepared JavaScript"));
}

#[test]
fn const_enum_recovery_opaque_cjs_real_exports_do_not_read_invalid_companions() {
    for (javascript, declaration, browser) in [
        ("exports.E = {A:'real'};", "invalid declaration {{{", false),
        (
            "module.exports = {E:{A:'real'}};",
            "invalid declaration {{{",
            true,
        ),
    ] {
        let dir = TmpDir::new();
        write(dir.root(), "main.mjs", "import {E} from './api.cjs'; import * as kit from './api.cjs'; console.log(E.A, kit.E.A);");
        write(dir.root(), "api.cjs", javascript);
        write(dir.root(), "api.d.cts", declaration);
        let mut config = options();
        if browser {
            config.resolver = NodeModulesResolverOptions::default_for_env(TargetEnv::Browser);
        }
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&config).unwrap();
        assert_eq!(observe(&result.product.artifacts), b"real real\n");
        assert!(result.warnings.is_empty());
        assert_eq!(session.type_only_cache.extractions(), 0);
    }
}

#[test]
fn const_enum_recovery_namespace_reexport_static_demand_and_unknown_surfaces() {
    let dir = TmpDir::new();
    write(dir.root(), "api.mjs", "export const real=1;");
    write(
        dir.root(),
        "barrel.mjs",
        "export * as kit from './api.mjs';",
    );
    write(
        dir.root(),
        "api.d.mts",
        "export declare const enum E { A='namespace' }",
    );
    write(
        dir.root(),
        "main.mjs",
        "import {kit} from './barrel.mjs'; console.log(kit.E.A);",
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&result.product.artifacts), b"namespace\n");
    for consumer in [
        "const name='E'; console.log(kit[name]);",
        "const pass=x=>x; console.log(pass(kit).E);",
    ] {
        write(
            dir.root(),
            "main.mjs",
            &format!("import * as kit from './api.mjs'; {consumer}"),
        );
        write(dir.root(), "api.d.mts", "invalid unneeded declaration {{{");
        let mut session = BuildSession::new(dir.root()).unwrap();
        let result = session.build(&options()).unwrap();
        assert_eq!(observe(&result.product.artifacts), b"undefined\n");
        assert_eq!(session.type_only_cache.extractions(), 0);
        assert!(result.warnings.is_empty());
    }
}

#[test]
fn const_enum_recovery_real_export_does_not_depend_on_unreadable_companion() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {E} from './api.mjs'; console.log(E.A);",
    );
    write(dir.root(), "api.mjs", "export const E={A:'real'};");
    fs::create_dir(dir.root().join("api.d.mts")).unwrap();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&options()).unwrap();
    assert_eq!(observe(&result.product.artifacts), b"real\n");
    assert_eq!(session.type_only_cache.extractions(), 0);
    assert!(result.warnings.is_empty());
}

#[test]
fn const_enum_recovery_star_barrel_can_own_an_erased_local_enum() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import {E,real} from './barrel.mjs'; console.log(E.A,real);",
    );
    write(dir.root(), "barrel.mjs", "export * from './leaf.mjs';");
    write(dir.root(), "leaf.mjs", "export const real='real';");
    write(
        dir.root(),
        "barrel.d.mts",
        "export * from './leaf.mjs'; export declare const enum E { A='barrel-local' }",
    );
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&options())
        .unwrap();
    assert_eq!(observe(&result.product.artifacts), b"barrel-local real\n");
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("barrel.d.mts"));
}

#[test]
fn const_enum_recovery_negative_namespace_observations_invalidate_successful_noops() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.mjs",
        "import * as kit from './api.mjs'; console.log(!!kit.E);",
    );
    write(dir.root(), "api.mjs", "export const real=1;");
    let mut session = BuildSession::new(dir.root()).unwrap();
    let missing = session.build(&options()).unwrap();
    assert_eq!(observe(&missing.product.artifacts), b"false\n");
    assert!(missing.warnings.is_empty());
    assert!(Rc::ptr_eq(
        &missing.product,
        &session.build(&options()).unwrap().product
    ));
    write(
        dir.root(),
        "api.d.mts",
        "export declare const enum E { A='new' }",
    );
    let added = session.build(&options()).unwrap();
    assert_eq!(observe(&added.product.artifacts), b"true\n");
    assert_eq!(added.warnings.len(), 1);
    fs::remove_file(dir.root().join("api.d.mts")).unwrap();
    let deleted = session.build(&options()).unwrap();
    assert_eq!(observe(&deleted.product.artifacts), b"false\n");
    assert!(deleted.warnings.is_empty());
    assert!(deleted.product.compatibility_warnings.is_empty());
}

#[test]
#[cfg(any(unix, windows))]
fn const_enum_recovery_declaration_link_retarget_rechecks_all_cache_modes() {
    fn symlink(target: &Path, link: &Path) {
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = TmpDir::new();
        write(
            dir.root(),
            "main.mjs",
            "import {E} from './api.mjs'; console.log(E.A);",
        );
        write(dir.root(), "api.mjs", "export const real=1;");
        write(
            dir.root(),
            "first.d.mts",
            "export declare const enum E { A='first' }",
        );
        write(
            dir.root(),
            "second.d.mts",
            "export declare const enum E { A='second' }",
        );
        let link = dir.root().join("api.d.mts");
        symlink(&dir.root().join("first.d.mts"), &link);
        let mut config = options();
        config.cache = cache;
        let mut session = BuildSession::new(dir.root()).unwrap();
        let first = session.build(&config).unwrap();
        assert_eq!(observe(&first.product.artifacts), b"first\n");
        fs::remove_file(&link).unwrap();
        symlink(&dir.root().join("second.d.mts"), &link);
        let second = session.build(&config).unwrap();
        assert_eq!(observe(&second.product.artifacts), b"second\n");
        assert!(!Rc::ptr_eq(&first.product, &second.product));
        assert!(Rc::ptr_eq(
            &second.product,
            &session.build(&config).unwrap().product
        ));
    }
}
