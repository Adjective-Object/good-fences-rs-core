use super::*;

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(".bundler-type-facts-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap()
}

fn facts(storage: &Path, path: &str, source: &str, package: ModuleFormat) -> WireTypeOnlyFacts {
    let allocator = Allocator::default();
    let path = Path::new(path);
    let parsed = oxc_utils_parse::parse_bundler_file(&allocator, source, path, package).unwrap();
    let config = ast_segmenter::ParserConfig::for_path(path, package)
        .unwrap()
        .resolved(parsed.program.source_type);
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_enum_eval(true)
        .build(&parsed.program);
    assert!(
        semantic.diagnostics.is_empty(),
        "{:?}",
        semantic.diagnostics
    );
    extract(
        storage,
        &parsed.program,
        &semantic.semantic,
        Settings::from_config(config, package).unwrap(),
    )
    .unwrap()
}

#[test]
fn original_types_values_aliases_and_routes() {
    let storage = directory();
    let record = facts(
        storage.path(),
        "original.ts",
        r#"
import { Shape as S, type Named, Value } from "./dependency";
interface Private { width: number }
type Merged = string;
const Merged = 1;
declare class AmbientClass {}
declare function ambientFunction(): void;
declare enum Ordinary { A }
declare const enum Hidden { A = 3, B, Text = "text" }
export { Private as Public, Merged, Hidden as Renamed, AmbientClass, ambientFunction, Ordinary };
export type { Private as Explicit };
export { Remote as RenamedRemote } from "./remote";
export type { Typed } from "./types";
export * from "./star";
export * as namespace from "./namespace";
export default interface DefaultShape { field: Named }
const a: S = { width: Value };
"#,
        ModuleFormat::Esm,
    );
    let f = record.facts();
    let exports: Vec<_> = f
        .exports()
        .iter()
        .map(|e| {
            (
                e.name().value(),
                e.presence(),
                e.has_type(),
                e.explicit_type(),
            )
        })
        .collect();
    assert!(exports.contains(&("Public", wire::Presence::OnlyType, true, false)));
    assert!(exports.contains(&("Merged", wire::Presence::RuntimeValue, true, false)));
    assert!(exports.contains(&("Renamed", wire::Presence::ConstEnum, true, false)));
    assert!(exports.contains(&("AmbientClass", wire::Presence::RuntimeValue, true, false)));
    assert!(exports.contains(&(
        "ambientFunction",
        wire::Presence::RuntimeValue,
        false,
        false
    )));
    assert!(exports.contains(&("Ordinary", wire::Presence::RuntimeValue, true, false)));
    assert!(exports.contains(&("Explicit", wire::Presence::OnlyType, true, true)));
    assert!(exports.contains(&("RenamedRemote", wire::Presence::Unresolved, false, false)));
    assert!(exports.contains(&("Typed", wire::Presence::OnlyType, true, true)));
    assert!(exports.contains(&("default", wire::Presence::OnlyType, true, false)));
    assert_eq!(f.const_enums().get(0).name().value(), "Hidden");
    let imports: Vec<_> = f
        .occurrences()
        .iter()
        .filter(|o| o.kind() == wire::OccurrenceKind::Import)
        .collect();
    assert_eq!(imports.len(), 3);
    assert!(imports[0]
        .uses()
        .iter()
        .any(|u| u.type_use() && !u.value_use()));
    assert!(imports[1].explicit_type());
    assert!(imports[2].uses().iter().any(|u| u.value_use()));
    assert!(record.backing().is_mapped());
    assert!(
        !f.locals()
            .iter()
            .find(|l| l.name().value() == "S")
            .unwrap()
            .has_type(),
        "unresolved unmarked imports are routes, not evidence that the target declares a type"
    );
}

#[test]
fn enum_constants_and_diagnostic_evidence() {
    let storage = directory();
    let record = facts(
        storage.path(),
        "enums.d.ts",
        r#"
declare const enum Forward { A = Later.Seed + 1, B, Text = "x" + "y", Shift = 1 << 3 }
declare const enum Later { Seed = 5, S = "s", Next = S + "!" }
export { Forward as Public };
export declare const enum Cycle { A = B, B = A, Bad = call(), Invalid = 1 / 0 }
"#,
        ModuleFormat::Esm,
    );
    let f = record.facts();
    let forward = f.const_enums().get(0);
    assert_eq!(forward.members().get(0).number(), Some(6.0));
    assert_eq!(forward.members().get(1).number(), Some(7.0));
    assert_eq!(forward.members().get(2).string(), Some("xy"));
    assert_eq!(forward.members().get(3).number(), Some(8.0));
    assert_eq!(f.const_enums().get(1).members().get(2).string(), Some("s!"));
    assert_eq!(f.diagnostics().len(), 4);
    assert!(f
        .diagnostics()
        .iter()
        .any(|d| d.kind() == wire::DiagnosticKind::UnsupportedConstant));
    assert!(f
        .diagnostics()
        .iter()
        .any(|d| d.kind() == wire::DiagnosticKind::CyclicConstant));
    assert!(f
        .const_enums()
        .get(2)
        .members()
        .iter()
        .all(|m| m.kind() == wire::ConstantKind::Unresolved));
}

#[test]
fn review_merged_forward_enum_members() {
    let dir = directory();
    let record = facts(
        dir.path(),
        "merged.d.ts",
        "declare const enum E { A=Later.Seed }\ndeclare const enum E { B=A+1 }\ndeclare const enum Later { Seed=5 }\nexport {E};",
        ModuleFormat::Esm,
    );
    assert_eq!(
        record
            .facts()
            .const_enums()
            .get(1)
            .members()
            .get(0)
            .number(),
        Some(6.0)
    );
    assert!(record.facts().diagnostics().is_empty());
}

#[test]
fn review_standalone_string_expansion_is_bounded() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    // The saved pre-fix repro used only 18 steps (256 KiB). Forty steps would
    // expand to 1 TiB without guarding the standalone semantic builder itself.
    let mut unbounded = Vec::new();
    for steps in [18, 40] {
        for forward in [true, false] {
            let mut source = String::new();
            for i in 0..=steps {
                let dependency = if forward { i + 1 } else { i - 1 };
                if (forward && i == steps) || (!forward && i == 0) {
                    source.push_str(&format!("declare const enum E{i} {{ V='x' }}\n"));
                } else {
                    source.push_str(&format!(
                        "declare const enum E{i} {{ V=E{dependency}.V+E{dependency}.V }}\n"
                    ));
                }
            }
            fs::write(root.join("growth.d.ts"), source).unwrap();
            for mode in [
                CacheMode::ReadWrite,
                CacheMode::ReadOnly,
                CacheMode::Disabled,
            ] {
                let (observation, _) =
                    load(&mut TypeOnlyCache::default(), &root, "growth.js", mode);
                if !observation
                    .record()
                    .unwrap()
                    .facts()
                    .diagnostics()
                    .iter()
                    .any(|d| {
                        d.kind() == wire::DiagnosticKind::UnsupportedConstant
                            && d.message().contains("string")
                    })
                {
                    unbounded.push((steps, forward, mode));
                }
                let evidence = observation.record().unwrap().facts();
                assert!(evidence
                    .const_enums()
                    .iter()
                    .flat_map(|e| e.members())
                    .filter_map(|m| m.string())
                    .all(|s| s.len() <= cache_flatbuffers::TYPE_ONLY_STRING_VALUE_BYTES));
                assert_eq!(
                    evidence
                        .const_enums()
                        .iter()
                        .flat_map(|e| e.members())
                        .filter(|m| m.string().is_some())
                        .count(),
                    17
                );
            }
        }
    }
    assert!(
        unbounded.is_empty(),
        "oversized constants without unsupported evidence (forward): {unbounded:?}"
    );
}

fn enum_evidence(source: &str, seed: enums::Seed, limits: enums::StringLimits) -> wire::FactsT {
    let allocator = Allocator::default();
    let parsed = oxc_utils_parse::parse_bundler_file(
        &allocator,
        source,
        Path::new("evidence.d.ts"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_enum_eval(seed == enums::Seed::Semantic)
        .build(&parsed.program);
    assert!(
        semantic.diagnostics.is_empty(),
        "{:?}",
        semantic.diagnostics
    );
    let mut facts = wire::FactsT::default();
    enums::extract_with_limits(
        &parsed.program,
        &semantic.semantic,
        &mut facts,
        seed,
        limits,
    )
    .unwrap();
    facts
}

#[test]
fn expanded_string_limits_cover_seeds_literals_references_templates_and_storage() {
    for seed in [enums::Seed::Semantic, enums::Seed::Bounded] {
        let facts = enum_evidence(
            r#"declare const enum E {
                A='1234', Alias=A, Joined=A+A, Huge=Joined+'x',
                Template=`${Joined}x`, Number='v'+1e21, Static=E['A']
            }"#,
            seed,
            enums::StringLimits {
                value: 8,
                total: 128,
            },
        );
        let members = &facts.const_enums[0].members;
        assert_eq!(members[0].string.as_deref(), Some("1234"));
        assert_eq!(members[1].string.as_deref(), Some("1234"));
        assert_eq!(members[2].string.as_deref(), Some("12341234"));
        assert_eq!(members[5].string.as_deref(), Some("v1e+21"));
        assert_eq!(members[6].string.as_deref(), Some("1234"));
        assert!(members[3..5]
            .iter()
            .all(|m| m.kind == wire::ConstantKind::Unresolved && m.string.is_none()));
        assert_eq!(facts.diagnostics.len(), 2);
        assert!(facts
            .diagnostics
            .iter()
            .all(|d| d.kind == wire::DiagnosticKind::UnsupportedConstant
                && d.message.contains("per-value")));

        let facts = enum_evidence(
            "declare const enum E { A='12345678', B='abcdefgh', C='xyz', D=1 }",
            seed,
            enums::StringLimits {
                value: 8,
                total: 18,
            },
        );
        assert!(facts.const_enums[0].members[..3]
            .iter()
            .all(|m| m.kind == wire::ConstantKind::Unresolved && m.string.is_none()));
        assert_eq!(facts.const_enums[0].members[3].number, Some(1.0));
        assert_eq!(facts.diagnostics.len(), 3);
        assert!(facts
            .diagnostics
            .iter()
            .all(|d| d.kind == wire::DiagnosticKind::UnsupportedConstant
                && d.message.contains("aggregate")));
    }
    let shared = enum_evidence(
        "declare const enum E { A='1234', B=A, C=E.A }",
        enums::Seed::Bounded,
        enums::StringLimits {
            value: 4,
            total: 16,
        },
    );
    assert!(shared.diagnostics.is_empty());
    assert!(shared.const_enums[0]
        .members
        .iter()
        .all(|m| m.string.as_deref() == Some("1234")));
}

#[test]
fn standalone_bounded_evaluation_preserves_normal_pinned_constants() {
    let source = r#"declare enum Ordinary { Base=8 }
        declare const enum E {
            Zero=-0, Empty="", Nul="\0", Base=Ordinary.Base,
            Whole=`hé\n`, Template=`${Base}:${"a"}:${1e21}`,
            Join="n"+1e-7, Numeric=(2**3 + 4*2 - 1) / 3 % 4,
            Shifts=(1<<3)|(16>>>2), Mask=~1 & 7, Plus=+3,
            Forward=Later.Seed, ForwardText=`${Later.Seed}:${Later.Text}`,
            Globals=`${Infinity}:${NaN}`, NonfiniteString="" + (1/0)
        }
        declare const enum E { Next=Base+1 }
        declare const enum Later { Seed=5, Text="z" }
        export {E};"#;
    let pinned = enum_evidence(
        source,
        enums::Seed::Semantic,
        enums::StringLimits::default(),
    );
    let bounded = enum_evidence(source, enums::Seed::Bounded, enums::StringLimits::default());
    assert!(pinned.diagnostics.is_empty());
    assert!(bounded.diagnostics.is_empty());
    assert_eq!(pinned.const_enums, bounded.const_enums);
    assert_eq!(
        bounded.const_enums[0].members[0].number.unwrap().to_bits(),
        (-0.0f64).to_bits()
    );
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("normal.d.ts"), source).unwrap();
    let (observed, _) = load(
        &mut TypeOnlyCache::default(),
        &root,
        "normal.js",
        CacheMode::Disabled,
    );
    assert!(observed.record().unwrap().facts().diagnostics().is_empty());
    assert_eq!(
        observed
            .record()
            .unwrap()
            .facts()
            .const_enums()
            .get(0)
            .members()
            .get(5)
            .string(),
        Some("8:a:1e+21")
    );
}

#[test]
fn merged_sibling_cycles_external_misses_and_lexical_shadowing() {
    let source = r#"
        declare const A: number;
        declare const enum E { A=Later.Seed, Cycle=C, External=Missing.Value }
        declare const enum E { B=A+1, C=Cycle, Forward=Future+1 }
        declare const enum E { Future=Later.Seed+2 }
        declare const enum Later { Seed=5 }
        declare const enum Shadow { Seed=100 }
        declare const enum F { A=Later.Seed }
        declare const enum F { Shadow=2, B=Shadow.Seed, C=Shadow+1 }
        declare const enum Reference { Value=Missing.Value }
        declare const enum Receiver { Reference=A }
        declare const enum Receiver { A=Reference.Value, B=Reference["Value"] }
        export {E,F};
    "#;
    for seed in [enums::Seed::Semantic, enums::Seed::Bounded] {
        let facts = enum_evidence(source, seed, enums::StringLimits::default());
        assert_eq!(facts.const_enums[0].members[0].number, Some(5.0));
        assert_eq!(facts.const_enums[1].members[0].number, Some(6.0));
        assert_eq!(facts.const_enums[1].members[2].number, Some(8.0));
        assert_eq!(facts.const_enums[2].members[0].number, Some(7.0));
        assert_eq!(facts.const_enums[6].members[2].number, Some(3.0));
        assert_eq!(
            facts.const_enums[6].members[1].kind,
            wire::ConstantKind::Unresolved
        );
        let classification: Vec<_> = facts.diagnostics.iter().map(|d| d.kind).collect();
        assert_eq!(
            classification,
            [
                wire::DiagnosticKind::CyclicConstant,
                wire::DiagnosticKind::UnresolvedConstant,
                wire::DiagnosticKind::CyclicConstant,
                wire::DiagnosticKind::UnresolvedConstant,
                wire::DiagnosticKind::UnresolvedConstant,
                wire::DiagnosticKind::UnresolvedConstant,
                wire::DiagnosticKind::UnresolvedConstant,
                wire::DiagnosticKind::UnresolvedConstant,
            ]
        );
    }
}

fn load(
    cache: &mut TypeOnlyCache,
    root: &Path,
    javascript: &str,
    mode: CacheMode,
) -> (Observation, Option<String>) {
    let inventory = super::super::inventory::Inventory::new(root);
    cache
        .load_companion(
            &inventory,
            root,
            &root.join(javascript),
            ModuleFormat::Esm,
            mode,
        )
        .unwrap()
        .unwrap()
}

fn exported(observation: &Observation) -> Vec<String> {
    observation
        .record()
        .unwrap()
        .facts()
        .exports()
        .iter()
        .map(|e| e.name().value().to_owned())
        .collect()
}

#[test]
fn lazy_declarations_add_edit_delete_and_shared_extraction_in_every_mode() {
    for mode in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let dir = directory();
        let root = dir.path().canonicalize().unwrap();
        fs::write(
            root.join("other.d.ts"),
            "this is deliberately invalid metadata",
        )
        .unwrap();
        let path = root.join("value.d.ts");
        let mut cache = TypeOnlyCache::default();
        let (absent, warning) = load(&mut cache, &root, "value.js", mode);
        assert!(warning.is_none());
        assert!(absent.record().is_none());
        assert_eq!(cache.extractions(), 0);
        fs::write(&path, "export interface First {}").unwrap();
        assert!(absent.validate().is_err());
        let (first, warning) = load(&mut cache, &root, "value.js", mode);
        assert!(warning.is_none(), "{warning:?}");
        assert_eq!(exported(&first), ["First"]);
        assert_eq!(cache.extractions(), 1);
        let (shared, _) = load(&mut cache, &root, "value.js", mode);
        assert_eq!(cache.extractions(), 1);
        assert_eq!(
            shared.record().unwrap().backing().as_ptr(),
            first.record().unwrap().backing().as_ptr()
        );
        assert_eq!(sidecar(&path).exists(), mode == CacheMode::ReadWrite);
        let mut fresh = TypeOnlyCache::default();
        let (disk, _) = load(&mut fresh, &root, "value.js", mode);
        assert_eq!(exported(&disk), ["First"]);
        assert_eq!(
            fresh.extractions(),
            usize::from(mode != CacheMode::ReadWrite)
        );
        fs::write(&path, "export type Second = string;").unwrap();
        assert!(first.validate().is_err());
        let (second, _) = load(&mut cache, &root, "value.js", mode);
        assert_eq!(exported(&second), ["Second"]);
        assert_eq!(
            exported(&first),
            ["First"],
            "old mapped generations must remain immutable"
        );
        assert_eq!(cache.extractions(), 2);
        fs::remove_file(&path).unwrap();
        assert!(second.validate().is_err());
        let (missing, _) = load(&mut cache, &root, "value.js", mode);
        assert!(missing.record().is_none());
        assert_eq!(cache.extractions(), 2);
        fs::write(&path, "export declare const enum Again { A = 4 }").unwrap();
        assert!(missing.validate().is_err());
        let (added, _) = load(&mut cache, &root, "value.js", mode);
        assert_eq!(exported(&added), ["Again"]);
        assert_eq!(cache.extractions(), 3);
    }
}

#[test]
fn corruption_regenerates_but_readonly_and_disabled_never_publish() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("value.d.ts");
    fs::write(&path, "export type T = number;").unwrap();
    load(
        &mut TypeOnlyCache::default(),
        &root,
        "value.js",
        CacheMode::ReadWrite,
    );
    let sidecar = sidecar(&path);
    let valid = fs::read(&sidecar).unwrap();
    let mut readonly = TypeOnlyCache::default();
    load(&mut readonly, &root, "value.js", CacheMode::ReadOnly);
    assert_eq!(readonly.extractions(), 0);
    let mut disabled = TypeOnlyCache::default();
    load(&mut disabled, &root, "value.js", CacheMode::Disabled);
    assert_eq!(disabled.extractions(), 1);
    fs::write(&sidecar, b"corrupted typed metadata").unwrap();
    for mode in [CacheMode::ReadOnly, CacheMode::Disabled] {
        let mut cache = TypeOnlyCache::default();
        let (record, warning) = load(&mut cache, &root, "value.js", mode);
        assert!(warning.is_none());
        assert_eq!(exported(&record), ["T"]);
        assert_eq!(cache.extractions(), 1);
        assert_eq!(fs::read(&sidecar).unwrap(), b"corrupted typed metadata");
    }
    let mut cache = TypeOnlyCache::default();
    load(&mut cache, &root, "value.js", CacheMode::ReadWrite);
    assert_eq!(cache.extractions(), 1);
    assert_eq!(fs::read(&sidecar).unwrap(), valid);
}

#[test]
fn changed_or_invalid_metadata_never_returns_previous_success() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("value.d.ts");
    fs::write(&path, "export interface Good {}").unwrap();
    let mut cache = TypeOnlyCache::default();
    let (old, _) = load(&mut cache, &root, "value.js", CacheMode::ReadWrite);
    fs::write(&path, "export interface {").unwrap();
    let inventory = super::super::inventory::Inventory::new(&root);
    let error = cache
        .load_companion(
            &inventory,
            &root,
            &root.join("value.js"),
            ModuleFormat::Esm,
            CacheMode::ReadWrite,
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("parser diagnostics"));
    assert!(old.validate().is_err());
    fs::write(&path, "export interface Fixed {}").unwrap();
    let (fixed, _) = load(&mut cache, &root, "value.js", CacheMode::ReadWrite);
    assert_eq!(exported(&fixed), ["Fixed"]);
}

#[test]
fn exact_companions_package_modes_and_executable_declaration_rejection() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let mut cache = TypeOnlyCache::default();
    for (js, declaration, source) in [
        ("a.js", "a.d.ts", "export interface T {}"),
        (
            "b.mjs",
            "b.d.mts",
            "export declare const enum E { A = \"A\" }",
        ),
        (
            "c.cjs",
            "c.d.cts",
            "declare const enum E { A = 1 } export = E;",
        ),
    ] {
        fs::write(root.join(declaration), source).unwrap();
        let (facts, _) = load(&mut cache, &root, js, CacheMode::ReadWrite);
        assert!(facts.record().is_some());
        for loader in [crate::loaders::Loader::Js, crate::loaders::Loader::Ts] {
            let policy = crate::loaders::LoaderPolicy::default()
                .with_override(
                    &format!(
                        ".{}",
                        Path::new(declaration)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .split_once('.')
                            .unwrap()
                            .1
                    ),
                    Some(loader),
                )
                .unwrap();
            assert!(policy.select(&root.join(declaration)).is_err());
        }
    }
    let inventory = super::super::inventory::Inventory::new(&root);
    assert!(cache
        .load_companion(
            &inventory,
            &root,
            &root.join("a.ts"),
            ModuleFormat::Esm,
            CacheMode::ReadWrite
        )
        .unwrap()
        .is_none());
}

#[test]
fn checked_scopes_and_retargeted_declaration_links_are_fresh() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("inside")).unwrap();
    fs::write(root.join("first.d.ts"), "export interface First {}").unwrap();
    fs::write(root.join("second.d.ts"), "export interface Second {}").unwrap();
    let link = root.join("value.d.ts");
    fn symlink(target: &Path, link: &Path) {
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(target, link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }
    symlink(&root.join("first.d.ts"), &link);
    for mode in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        let mut cache = TypeOnlyCache::default();
        let (first, _) = load(&mut cache, &root, "value.js", mode);
        assert_eq!(exported(&first), ["First"]);
        fs::remove_file(&link).unwrap();
        symlink(&root.join("second.d.ts"), &link);
        assert!(first.validate().is_err());
        let (second, _) = load(&mut cache, &root, "value.js", mode);
        assert_eq!(exported(&second), ["Second"]);
        fs::remove_file(&link).unwrap();
        symlink(&root.join("first.d.ts"), &link);
    }
    let inside = root.join("inside");
    let inventory = super::super::inventory::Inventory::new(&inside);
    let error = TypeOnlyCache::default()
        .load_companion(
            &inventory,
            &root,
            &root.join("value.js"),
            ModuleFormat::Esm,
            CacheMode::Disabled,
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("outside reachable inventory"));
}

#[test]
fn original_import_byte_locations_type_queries_and_executable_uses() {
    let dir = directory();
    let source = r#"import Def, { type T as Renamed, Both, Exported } from "dep";
import * as ns from "space";
type A = Def;
type B = typeof Both;
const value: Renamed = ns.make(Both);
export { Exported };
export /* default is in a comment */   default interface Shape {}
"#;
    let record = facts(dir.path(), "uses.ts", source, ModuleFormat::Esm);
    let f = record.facts();
    for occurrence in f.occurrences() {
        for n in [
            occurrence.request(),
            occurrence.local(),
            occurrence.exported(),
        ]
        .into_iter()
        .flatten()
        {
            let text = &source[n.span().start() as usize..n.span().end() as usize];
            assert!(text.contains(n.value()), "{text:?} {:?}", n.value());
        }
        for usage in occurrence.uses() {
            let text = &source[usage.span().start() as usize..usage.span().end() as usize];
            assert_eq!(text, occurrence.local().unwrap().value());
        }
    }
    let both = f
        .occurrences()
        .iter()
        .find(|o| o.local().is_some_and(|n| n.value() == "Both"))
        .unwrap();
    assert!(both
        .uses()
        .iter()
        .any(|u| u.value_as_type() && !u.value_use()));
    assert!(both.uses().iter().any(|u| u.value_use()));
    let exported = f
        .occurrences()
        .iter()
        .find(|o| {
            o.kind() == wire::OccurrenceKind::Import && o.local().unwrap().value() == "Exported"
        })
        .unwrap();
    assert!(
        exported.uses().is_empty(),
        "export connectors are not executable uses"
    );
    let default = f
        .exports()
        .iter()
        .find(|e| e.name().value() == "default")
        .unwrap();
    assert_eq!(
        &source[default.name().span().start() as usize..default.name().span().end() as usize],
        "default"
    );
}

#[test]
fn already_parsed_capture_is_shared_and_settings_sensitive_without_transform_wiring() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("original.ts");
    let source = "export interface Erased {} export const value = 1;";
    fs::write(&path, source).unwrap();
    let input = ftree_cache::inputs::InputSnapshot::default()
        .read(&path)
        .unwrap()
        .0;
    let allocator = Allocator::default();
    let parsed =
        oxc_utils_parse::parse_bundler_file(&allocator, source, &path, ModuleFormat::Esm).unwrap();
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_enum_eval(true)
        .build(&parsed.program);
    let config = ast_segmenter::ParserConfig::for_path(&path, ModuleFormat::Esm)
        .unwrap()
        .resolved(parsed.program.source_type);
    let settings = Settings::from_config(config, ModuleFormat::Esm).unwrap();
    let mut cache = TypeOnlyCache::default();
    let (first, _) = cache
        .capture_original(
            &input,
            &root,
            &parsed.program,
            &semantic.semantic,
            settings,
            CacheMode::ReadWrite,
        )
        .unwrap();
    let (second, _) = cache
        .capture_original(
            &input,
            &root,
            &parsed.program,
            &semantic.semantic,
            settings,
            CacheMode::ReadWrite,
        )
        .unwrap();
    assert_eq!(cache.extractions(), 1);
    assert_eq!(
        first.record().unwrap().backing().as_ptr(),
        second.record().unwrap().backing().as_ptr()
    );
    let mut disk = TypeOnlyCache::default();
    disk.capture_original(
        &input,
        &root,
        &parsed.program,
        &semantic.semantic,
        settings,
        CacheMode::ReadOnly,
    )
    .unwrap();
    assert_eq!(disk.extractions(), 0);
    let changed = Settings {
        package: ModuleFormat::CommonJs,
        ..settings
    };
    disk.capture_original(
        &input,
        &root,
        &parsed.program,
        &semantic.semantic,
        changed,
        CacheMode::ReadOnly,
    )
    .unwrap();
    assert_eq!(disk.extractions(), 1);
    assert_eq!(
        parsed.program.body.len(),
        2,
        "extractor must not alter the original tree"
    );
    let javascript = oxc_utils_parse::parse_bundler_file(
        &allocator,
        source,
        Path::new("source.js"),
        ModuleFormat::Esm,
    );
    assert!(javascript.is_err());
    let js = oxc_utils_parse::parse_bundler_file(
        &allocator,
        "export const value = 1;",
        Path::new("source.js"),
        ModuleFormat::Esm,
    )
    .unwrap();
    let js_semantic = SemanticBuilder::new().build(&js.program);
    assert!(extract(&root, &js.program, &js_semantic.semantic, settings).is_err());
}

#[test]
fn type_import_routes_explicit_type_value_misuse_and_shadowing() {
    let dir = directory();
    let source = r#"
import type { Only } from "dep";
import { Shadowed, Untouched } from "other";
type Remote = import("remote").Group.Nested;
Only();
function f(Shadowed: number) { return Shadowed; }
const executable = Untouched;
"#;
    let record = facts(dir.path(), "uses.ts", source, ModuleFormat::Esm);
    let occurrences: Vec<_> = record.facts().occurrences().iter().collect();
    let imported = occurrences
        .iter()
        .find(|o| o.kind() == wire::OccurrenceKind::Import && o.local().unwrap().value() == "Only")
        .unwrap();
    assert!(imported.explicit_type());
    assert!(
        imported.uses().iter().any(|u| u.value_use()),
        "illegal executable uses must remain evidence, never get erased as a decision"
    );
    let shadowed = occurrences
        .iter()
        .find(|o| o.local().is_some_and(|n| n.value() == "Shadowed"))
        .unwrap();
    assert!(shadowed.uses().is_empty());
    let type_import = occurrences
        .iter()
        .find(|o| o.kind() == wire::OccurrenceKind::TypeImport)
        .unwrap();
    assert_eq!(type_import.request().unwrap().value(), "remote");
    assert_eq!(type_import.imported().unwrap().value(), "Group");
    assert!(type_import.explicit_type());
}

#[test]
fn numeric_string_constant_details_and_merged_declarations() {
    let dir = directory();
    let record = facts(
        dir.path(),
        "constants.d.ts",
        r#"
declare enum Ordinary { Base = 8 }
declare const enum E { Zero = -0, Empty = "", Nul = "\0", Base = Ordinary.Base }
declare const enum E { Next = Base + 1, Hex = 0xff, Mask = Hex & 3 }
declare const enum F { Forward = G["First"] << 1, Next, Signed = -Forward }
declare const enum G { First = 4 }
export { E, F };
"#,
        ModuleFormat::Esm,
    );
    let f = record.facts();
    assert!(f.diagnostics().is_empty());
    assert_eq!(
        f.const_enums()
            .get(0)
            .members()
            .get(0)
            .number()
            .unwrap()
            .to_bits(),
        (-0.0f64).to_bits()
    );
    assert_eq!(f.const_enums().get(0).members().get(1).string(), Some(""));
    assert_eq!(f.const_enums().get(0).members().get(2).string(), Some("\0"));
    assert_eq!(f.const_enums().get(1).members().get(0).number(), Some(9.0));
    assert_eq!(f.const_enums().get(1).members().get(2).number(), Some(3.0));
    assert_eq!(f.const_enums().get(2).members().get(0).number(), Some(8.0));
    assert_eq!(f.const_enums().get(2).members().get(1).number(), Some(9.0));
    assert_eq!(f.const_enums().get(2).members().get(2).number(), Some(-8.0));
}

#[test]
fn commonjs_default_declaration_detection_and_disk_reuse_are_sealed() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("module.d.ts"), "export interface T {}").unwrap();
    let inventory = super::super::inventory::Inventory::new(&root);
    let mut cache = TypeOnlyCache::default();
    cache
        .load_companion(
            &inventory,
            &root,
            &root.join("module.js"),
            ModuleFormat::CommonJs,
            CacheMode::ReadWrite,
        )
        .unwrap();
    assert_eq!(cache.extractions(), 1);
    let mut disk = TypeOnlyCache::default();
    disk.load_companion(
        &inventory,
        &root,
        &root.join("module.js"),
        ModuleFormat::CommonJs,
        CacheMode::ReadOnly,
    )
    .unwrap();
    assert_eq!(disk.extractions(), 0);
    disk.load_companion(
        &inventory,
        &root,
        &root.join("module.js"),
        ModuleFormat::Esm,
        CacheMode::ReadOnly,
    )
    .unwrap();
    assert_eq!(
        disk.extractions(),
        1,
        "package policy changes are an independent cache input"
    );
}

#[test]
fn final_boundary_and_live_epoch_observations_are_separate() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("value.d.ts");
    fs::write(&path, "export type A = string;").unwrap();
    let inventory = super::super::inventory::Inventory::new(&root);
    let mut cache = TypeOnlyCache::default();
    let (observed, _) = cache
        .load_companion(
            &inventory,
            &root,
            &root.join("value.js"),
            ModuleFormat::Esm,
            CacheMode::Disabled,
        )
        .unwrap()
        .unwrap();
    for _ in 0..10 {
        let (shared, _) = cache
            .load_companion(
                &inventory,
                &root,
                &root.join("value.js"),
                ModuleFormat::Esm,
                CacheMode::Disabled,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            shared.record().unwrap().backing().as_ptr(),
            observed.record().unwrap().backing().as_ptr()
        );
    }
    assert_eq!(cache.extractions(), 1);
    fs::write(&path, "export type B = string;").unwrap();
    assert!(observed.validate().is_err());
    assert!(inventory.recheck().is_err());
    let (fresh, _) = load(&mut cache, &root, "value.js", CacheMode::Disabled);
    assert_eq!(exported(&fresh), ["B"]);
}

#[test]
fn dependency_retries_stop_at_a_bounded_frontier_and_require_enum_semantics() {
    let dir = directory();
    let mut source = String::new();
    for index in 0..70 {
        source.push_str(&format!(
            "declare const enum E{index} {{ V = E{}.V + 1 }}\n",
            index + 1
        ));
    }
    source.push_str("declare const enum E70 { V = 0 }\nexport { E0 };");
    let record = facts(dir.path(), "bounded.d.ts", &source, ModuleFormat::Esm);
    assert_eq!(
        record.facts().const_enums().get(0).members().get(0).kind(),
        wire::ConstantKind::Unresolved
    );
    assert!(record
        .facts()
        .diagnostics()
        .iter()
        .all(|d| d.kind() == wire::DiagnosticKind::UnresolvedConstant));
    assert_eq!(
        record
            .facts()
            .const_enums()
            .get(70)
            .members()
            .get(0)
            .number(),
        Some(0.0)
    );
    let allocator = Allocator::default();
    let path = Path::new("one.d.ts");
    let parsed = oxc_utils_parse::parse_bundler_file(
        &allocator,
        "declare const enum E { A = 1 }",
        path,
        ModuleFormat::Esm,
    )
    .unwrap();
    let semantic = SemanticBuilder::new().build(&parsed.program);
    let config = ast_segmenter::ParserConfig::for_path(path, ModuleFormat::Esm)
        .unwrap()
        .resolved(parsed.program.source_type);
    let error = extract(
        dir.path(),
        &parsed.program,
        &semantic.semantic,
        Settings::from_config(config, ModuleFormat::Esm).unwrap(),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("with_enum_eval(true)"));
}

#[test]
fn declaration_semantic_errors_retain_typed_causes_and_paths() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("value.d.ts");
    fs::write(
        &path,
        "declare const enum E { A = \"text\", Invalid } export { E };",
    )
    .unwrap();
    let inventory = super::super::inventory::Inventory::new(&root);
    let error = TypeOnlyCache::default()
        .load_companion(
            &inventory,
            &root,
            &root.join("value.js"),
            ModuleFormat::Esm,
            CacheMode::Disabled,
        )
        .unwrap_err();
    let chain = format!("{error:#}");
    assert!(chain.contains("analyzing declaration metadata") && chain.contains("value.d.ts"));
    assert!(chain.contains("Enum member must have initializer"));
}

#[test]
fn selected_typescript_syntax_is_independent_of_original_filename() {
    let dir = directory();
    let root = dir.path().canonicalize().unwrap();
    let original = root.join("original.custom");
    let source = "import Component from 'component'; export interface Shape {} export const view = <Component/>;";
    fs::write(&original, source).unwrap();
    let input = ftree_cache::inputs::InputSnapshot::default()
        .read(&original)
        .unwrap()
        .0;
    let allocator = Allocator::default();
    let parser_path = Path::new("selected.mtsx");
    let parsed = oxc_utils_parse::parse_bundler_file(
        &allocator,
        source,
        parser_path,
        ModuleFormat::CommonJs,
    )
    .unwrap();
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_enum_eval(true)
        .build(&parsed.program);
    assert!(semantic.diagnostics.is_empty());
    let config = ast_segmenter::ParserConfig::for_path(parser_path, ModuleFormat::CommonJs)
        .unwrap()
        .resolved(parsed.program.source_type);
    let settings = Settings::from_config(config, ModuleFormat::CommonJs).unwrap();
    assert!(!settings.commonjs && settings.typescript_jsx);
    let (observed, _) = TypeOnlyCache::default()
        .capture_original(
            &input,
            &root,
            &parsed.program,
            &semantic.semantic,
            settings,
            CacheMode::Disabled,
        )
        .unwrap();
    let facts = observed.record().unwrap().facts();
    assert_eq!(facts.exports().get(0).name().value(), "Shape");
    let component = facts
        .occurrences()
        .iter()
        .find(|o| o.local().is_some_and(|n| n.value() == "Component"))
        .unwrap();
    assert!(
        component.uses().iter().any(|u| u.value_use()),
        "JSX references remain executable evidence"
    );
    assert!(!sidecar(&original).exists());
}
