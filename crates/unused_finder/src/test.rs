use std::{collections::HashMap, path::PathBuf};

use path_slash::PathBufExt;
use test_tmpdir::{amap, test_tmpdir};

use crate::{
    cfg::package_match_rules::PackageMatchRules, report::SegmentReport, report::SymbolReport,
    tag::UsedTag, SymbolReportWithTags, UnusedFinder, UnusedFinderConfig, UnusedFinderReport,
};

fn symbol(id: &str) -> SymbolReport {
    SymbolReport {
        id: id.to_string(),
        start: 0,
        end: 0,
    }
}

fn segment(segment_idx: usize) -> SegmentReport {
    SegmentReport {
        segment_idx,
        start: 0,
        end: 0,
    }
}

fn tagged_symbol(id: &str, tags: UsedTag) -> SymbolReportWithTags {
    SymbolReportWithTags {
        symbol: symbol(id),
        tags: tags.into(),
    }
}

fn normalize_path(tmpdir: &test_tmpdir::TmpDir, path: &str) -> String {
    PathBuf::from(path.replace(tmpdir.root().to_str().unwrap(), "<root>"))
        .to_slash_lossy()
        .to_string()
}

fn normalize_test_report(
    tmpdir: &test_tmpdir::TmpDir,
    result: UnusedFinderReport,
) -> UnusedFinderReport {
    UnusedFinderReport {
        unused_files: result
            .unused_files
            .iter()
            .map(|x| normalize_path(tmpdir, x))
            .collect(),
        unused_symbols: result
            .unused_symbols
            .into_iter()
            .map(|(k, v)| {
                let mut s_v = v.clone();
                s_v.sort();
                (normalize_path(tmpdir, &k), s_v)
            })
            .collect(),
        extra_file_tags: result
            .extra_file_tags
            .into_iter()
            .map(|(k, v)| (normalize_path(tmpdir, &k), v))
            .collect(),
        extra_symbol_tags: result
            .extra_symbol_tags
            .into_iter()
            .map(|(k, v)| {
                let mut s_v = v.clone();
                s_v.sort();
                (normalize_path(tmpdir, &k), s_v)
            })
            .collect(),
        unused_segments: result
            .unused_segments
            .into_iter()
            .map(|(k, v)| {
                let mut s_v = v.clone();
                s_v.sort();
                (normalize_path(tmpdir, &k), s_v)
            })
            .collect(),
    }
}

fn run_unused_test(
    tmpdir: &test_tmpdir::TmpDir,
    mut config: UnusedFinderConfig,
    mut expected: UnusedFinderReport,
) {
    // patch config for convenience
    if config.repo_root.is_empty() {
        config.repo_root = tmpdir.root().to_string_lossy().to_string();
    }

    // run the test
    let logger = logger::StdioLogger::new();
    let mut finder = UnusedFinder::new_from_cfg(&logger, config).unwrap();
    let result = finder.find_unused(&logger).unwrap();

    let report = result.get_report();

    // build map of actual symbol locations
    let mut actual_symbol_ranges: HashMap<String, HashMap<String, (u32, u32)>> = HashMap::default();

    // check symbols in output are accurate
    for (file_path, items) in report.unused_symbols.iter() {
        let content_bytes = std::fs::read(file_path).unwrap();
        for item in items.iter() {
            // check that the bytes in the slice match the expected symbol
            let symbol_bytes = &content_bytes[(item.start - 1) as usize..item.end as usize];
            let symbol_str = String::from_utf8(symbol_bytes.to_vec()).unwrap();
            if !symbol_str.contains(&item.id) {
                panic!(
                    "Symbol {} with range {}:{} has incorrect offsets. Actual range contents: {:?}",
                    item.id, item.start, item.end, symbol_str
                );
            }

            // store the location of the symbol
            actual_symbol_ranges
                .entry(normalize_path(tmpdir, file_path))
                .or_default()
                .insert(item.id.clone(), (item.start, item.end));
        }
    }

    for (file_path, symbols) in report.extra_symbol_tags.iter() {
        let content_bytes = std::fs::read(file_path).unwrap();
        for tagged_symbol in symbols.iter() {
            let symbol = &tagged_symbol.symbol;
            // check that the bytes in the slice match the expected symbol
            let symbol_bytes = &content_bytes[(symbol.start - 1) as usize..symbol.end as usize];
            let symbol_str = String::from_utf8(symbol_bytes.to_vec()).unwrap();
            if !symbol_str.contains(&symbol.id) {
                panic!(
                    "Symbol {} with range {}:{} has incorrect offsets. Actual range contents: {:?}",
                    symbol.id, symbol.start, symbol.end, symbol_str
                );
            }

            // store the location of the symbol
            actual_symbol_ranges
                .entry(normalize_path(tmpdir, file_path))
                .or_default()
                .insert(symbol.id.clone(), (symbol.start, symbol.end));
        }
    }

    // for each of the symbols in the expected map, if the offsets are zero,
    // replace them with the actual offsets from the actual map
    //
    // This is so we can write the expected test data without caring about offsets in most cases
    for (file_path, items) in expected.unused_symbols.iter_mut() {
        for item in items.iter_mut() {
            if item.start == 0 && item.end == 0 {
                if let Some((start, end)) = actual_symbol_ranges
                    .get(file_path)
                    .and_then(|m| m.get(&item.id))
                {
                    item.start = *start;
                    item.end = *end;
                }
            }
        }
    }
    for (file_path, items) in expected.extra_symbol_tags.iter_mut() {
        for item in items.iter_mut() {
            if item.symbol.start == 0 && item.symbol.end == 0 {
                if let Some((start, end)) = actual_symbol_ranges
                    .get(file_path)
                    .and_then(|m| m.get(&item.symbol.id))
                {
                    item.symbol.start = *start;
                    item.symbol.end = *end;
                }
            }
        }
    }

    // For each segment in the expected map, if the offsets are zero,
    // replace them with the actual offsets from the report
    for (file_path, segs) in expected.unused_segments.iter_mut() {
        if let Some(actual_segs) = report.unused_segments.get(file_path.as_str()).or_else(|| {
            // Try to find by normalized path
            let norm = normalize_path(tmpdir, file_path);
            report
                .unused_segments
                .iter()
                .find(|(k, _)| normalize_path(tmpdir, k) == norm)
                .map(|(_, v)| v)
        }) {
            for seg in segs.iter_mut() {
                if seg.start == 0 && seg.end == 0 {
                    if let Some(actual_seg) = actual_segs
                        .iter()
                        .find(|s| s.segment_idx == seg.segment_idx)
                    {
                        seg.start = actual_seg.start;
                        seg.end = actual_seg.end;
                    }
                }
            }
        }
    }

    // check the report is as expected
    assert_eq!(expected, normalize_test_report(tmpdir, report));
}

#[test]
fn test_package_exports_walk_roots() {
    let tmpdir = test_tmpdir!(
        "packages/with-exports/package.json" => r#"{
            "name": "with-exports",
            "main": "./main.js",
            "module": "./esm/module.js",
            "exports": {
                ".": "./index.js",
                "./foo": {
                    "default": "./foo/foo-default.js",
                    "bar": "./foo/foo-bar.js"
                }
            }
        }"#,
        "packages/with-exports/main.js" => r#""#,
        "packages/with-exports/module.js" => r#""#,
        "packages/with-exports/not-exported.js" => r#""#,
        "packages/with-exports/index.js" => r#""#,
        "packages/with-exports/foo/foo-default.js" => r#""#,
        "packages/with-exports/foo/foo-bar.js" => r#""#,
        "packages/with-exports/foo/not-exported.js" => r#""#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
            entry_packages: vec!["with-exports"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            unused_files: [
                "<root>/packages/with-exports/foo/not-exported.js",
                "<root>/packages/with-exports/module.js",
                "<root>/packages/with-exports/not-exported.js",
            ]
            .iter()
            .map(|x| x.to_string())
            .collect(),
            ..Default::default()
        },
    );
}

#[test]
fn test_root_export_symbols_used() {
    let tmpdir = test_tmpdir!(
        "packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "packages/root/main.js" => r#"
            export const root_symbol = "root_symbol";
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            ..Default::default()
        },
    );
}

#[test]
fn test_transitive_re_export() {
    let tmpdir = test_tmpdir!(
        "packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "packages/root/main.js" => r#"
            export { transitiveReExport } from "./transitive-1.js"
        "#,
        "packages/root/transitive-1.js" => r#"
            export { transitive as transitiveReExport } from "./transitive-2.js"
        "#,
        "packages/root/transitive-2.js" => r#"
            export function transitive() {}
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            ..Default::default()
        },
    );
}

#[test]
fn test_partially_unused_file() {
    let tmpdir = test_tmpdir!(
        "packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "packages/root/main.js" => r#"
            import { a } from "./imported-1.js";
        "#,
        "packages/root/imported-1.js" => r#"
            export const a = 1;
            export const b = 2;
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            unused_files: vec![],
            unused_symbols: amap!(
                "<root>/packages/root/imported-1.js" => vec![
                    symbol("b"),
                ]
            ),
            unused_segments: amap!(
                "<root>/packages/root/imported-1.js" => vec![
                    segment(1),
                ]
            ),
            ..Default::default()
        },
    );
}

#[test]
fn test_unusedignore_file() {
    let tmpdir = test_tmpdir!(
        "packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "packages/root/main.js" => r#""#,
        "packages/root/ignored-unused.js" => r#"
            export const a = 1;
            export const b = 2;
        "#,
        "packages/root/ignored-exception.js" => r#"
            export const exception = 1;
        "#,
        "packages/root/unused.js" => r#"
            export const a = 1;
            export const b = 2;
        "#,
        "packages/root/.unusedignore" => r#"
ignored-*.js
!ignored-exception.js
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            unused_files: [
                "<root>/packages/root/ignored-exception.js",
                "<root>/packages/root/unused.js",
            ]
            .iter()
            .map(|x| x.to_string())
            .collect(),
            unused_symbols: amap!(
                "<root>/packages/root/unused.js" => vec![
                    symbol("a"),
                    symbol("b"),
                ],
                "<root>/packages/root/ignored-exception.js" => vec![
                    symbol("exception"),
                ]
            ),
            extra_file_tags: amap!(
                "<root>/packages/root/ignored-unused.js" => UsedTag::FROM_IGNORED.into()
            ),
            extra_symbol_tags: amap!(
                "<root>/packages/root/ignored-unused.js" => vec![
                    tagged_symbol("a", UsedTag::FROM_IGNORED),
                    tagged_symbol("b", UsedTag::FROM_IGNORED),
                ]
            ),
            ..Default::default()
        },
    );
}

#[test]
fn test_non_root_root_path() {
    // Tests that the package traversal works when there is a non-root "root" path.
    let tmpdir = test_tmpdir!(
        "search_root/packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "search_root/packages/root/main.js" => r#"
        export * from "./other";
        "#,
        "search_root/packages/root/other.js" => r#""#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            ..Default::default()
        },
    );
}

#[test]
fn test_recursive_indirect_empty_import() {
    // Tests that the package traversal works when there is a non-root "root" path.
    let tmpdir = test_tmpdir!(
        "search_root/packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "search_root/packages/root/main.js" => r#"
            export * from "./depth-1";
        "#,
        "search_root/packages/root/depth-1.js" => r#"
            export * from "./depth-2";
        "#,
        "search_root/packages/root/depth-2.js" => r#""#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            ..Default::default()
        },
        UnusedFinderReport {
            ..Default::default()
        },
    );
}

#[test]
fn test_test_pattern() {
    // Tests tagging "test" files
    let tmpdir = test_tmpdir!(
        "search_root/packages/utils/testUtils.js" => r#"
            export const testSymbol = "testSymbol";
        "#,
        "search_root/packages/__tests__/myTest.js" => r#"
            import * as utils from "../utils/testUtils";
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            test_files: vec!["**/__tests__/*Test.js".to_string()],
            ..Default::default()
        },
        UnusedFinderReport {
            extra_file_tags: amap!(
                "<root>/search_root/packages/__tests__/myTest.js" => UsedTag::FROM_TEST.into(),
                "<root>/search_root/packages/utils/testUtils.js" => UsedTag::FROM_TEST.into()
            ),
            extra_symbol_tags: amap!(
                "<root>/search_root/packages/utils/testUtils.js" => vec![
                    tagged_symbol("testSymbol", UsedTag::FROM_TEST),
                ]
            ),
            ..Default::default()
        },
    );
}

#[test]
fn test_relative_test_pattern() {
    // Tests tagging "test" files with relative patterns
    let tmpdir = test_tmpdir!(
        "search_root/packages/test-utils/testUtils.js" => r#"
            export const testSymbol = "testSymbol";
        "#,
        "search_root/tests/myTest.js" => r#"
            import * as utils from "../packages/test-utils/testUtils";
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            test_files: vec!["search_root/tests/**".to_string()],
            ..Default::default()
        },
        UnusedFinderReport {
            extra_file_tags: amap!(
                "<root>/search_root/tests/myTest.js" => UsedTag::FROM_TEST.into(),
                "<root>/search_root/packages/test-utils/testUtils.js" => UsedTag::FROM_TEST.into()
            ),
            extra_symbol_tags: amap!(
                "<root>/search_root/packages/test-utils/testUtils.js" => vec![
                    tagged_symbol("testSymbol", UsedTag::FROM_TEST),
                ]
            ),
            ..Default::default()
        },
    );
}

#[test]
fn test_testfiles_ignored() {
    // Tests that test files are ignored
    let tmpdir = test_tmpdir!(
        "search_root/packages/__tests__/myTests.test.js" => r#"
            import { myFunction } from "../test-helpers/test-helpers";
        "#,
        "search_root/packages/test-helpers/package.json" => r#"{
            "name": "test-helpers",
            "exports": {
                "." : {
                    "source": "./test-helpers.js"
                }
            }
        }"#,
        "search_root/packages/test-helpers/test-helpers.js" => r#"
            export function myFunction() {}
        "#,
        "search_root/packages/test-helpers/unused-helpers.js" => r#"
            export function myFunction() {}
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            entry_packages: PackageMatchRules::empty(),
            test_files: vec!["**/__tests__/**".to_string()],
            ..Default::default()
        },
        UnusedFinderReport {
            unused_files: vec!["<root>/search_root/packages/test-helpers/unused-helpers.js".into()],
            unused_symbols: amap![
                "<root>/search_root/packages/test-helpers/unused-helpers.js" => vec![symbol("myFunction")]
            ],
            extra_file_tags: amap![
                "<root>/search_root/packages/test-helpers/test-helpers.js" => UsedTag::FROM_TEST.into(),
                "<root>/search_root/packages/__tests__/myTests.test.js" => UsedTag::FROM_TEST.into()
            ],
            extra_symbol_tags: amap![
                "<root>/search_root/packages/test-helpers/test-helpers.js" => vec![tagged_symbol("myFunction", UsedTag::FROM_TEST)]
            ],
            ..Default::default()
        },
    );
}

#[test]
fn test_indirect_typeonly_export() {
    // Tests typeonly exports do not propogate as "used"
    let tmpdir = test_tmpdir!(
        "search_root/packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "search_root/packages/root/main.js" => r#"
            export type { ReExportedAsTypeOnly } from "./other";
        "#,
        "search_root/packages/root/other.js" => r#"
            export class ReExportedAsTypeOnly {}
            export class NotReExported {}
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            allow_unused_types: true,
            ..Default::default()
        },
        UnusedFinderReport {
            unused_files: vec!["<root>/search_root/packages/root/other.js".to_string()],
            unused_symbols: amap![
                "<root>/search_root/packages/root/other.js" => vec![
                    symbol("NotReExported"),
                    symbol("ReExportedAsTypeOnly"),
                ]
            ],
            extra_file_tags: amap![
                "<root>/search_root/packages/root/main.js" => (UsedTag::TYPE_ONLY | UsedTag::FROM_ENTRY).into()
            ],
            extra_symbol_tags: amap![
                "<root>/search_root/packages/root/main.js" => vec![
                    tagged_symbol("ReExportedAsTypeOnly", UsedTag::TYPE_ONLY | UsedTag::FROM_ENTRY),
                ]
            ],
            ..Default::default()
        },
    );
}

#[test]
fn test_typeonly_interface_allowed() {
    // Tests that interfaces are considered typeonly exports
    let tmpdir = test_tmpdir!(
        "search_root/packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "search_root/packages/root/main.js" => r#"
            export { ReExported } from "./other";
        "#,
        "search_root/packages/root/other.js" => r#"
            // This should be allowed because it is a typeonly export
            export interface MyInterface {
                getFoo: () => string;
            }

            export class ReExported<T extends MyInterface> {}
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            allow_unused_types: true,
            ..Default::default()
        },
        UnusedFinderReport {
            extra_symbol_tags: amap![
                "<root>/search_root/packages/root/other.js" => vec![tagged_symbol("MyInterface", UsedTag::TYPE_ONLY)]
            ],
            ..Default::default()
        },
    );
}

#[test]
fn test_typeonly_files_are_typeonly() {
    // Tests that interfaces are considered typeonly exports
    let tmpdir = test_tmpdir!(
        "search_root/packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "search_root/packages/root/main.js" => r#"
            export type TypeOnly = string
        "#
    );

    run_unused_test(
        &tmpdir,
        UnusedFinderConfig {
            repo_root: tmpdir.root().to_string_lossy().to_string(),
            root_paths: vec!["search_root".to_string()],
            entry_packages: vec!["entrypoint"].try_into().unwrap(),
            allow_unused_types: true,
            ..Default::default()
        },
        UnusedFinderReport {
            extra_symbol_tags: amap![
                "<root>/search_root/packages/root/main.js" => vec![tagged_symbol("TypeOnly", UsedTag::TYPE_ONLY| UsedTag::FROM_ENTRY)]
            ],
            extra_file_tags: amap![
                "<root>/search_root/packages/root/main.js" => (UsedTag::TYPE_ONLY | UsedTag::FROM_ENTRY).into()
            ],
            ..Default::default()
        },
    );
}

// ── Segment-level unused reporting tests ──────────────────────────────

#[test]
fn test_segment_one_used_one_unused_declaration() {
    // File with two exports: `a` is imported by the entry, `b` is not.
    // The segment containing `b` should appear in unused_segments.
    let tmpdir = test_tmpdir!(
        "packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "packages/root/main.js" => r#"
            import { a } from "./lib.js";
        "#,
        "packages/root/lib.js" => r#"
            export const a = 1;
            export const b = 2;
        "#
    );

    let logger = logger::StdioLogger::new();
    let mut config = UnusedFinderConfig {
        root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
        entry_packages: vec!["entrypoint"].try_into().unwrap(),
        ..Default::default()
    };
    config.repo_root = tmpdir.root().to_string_lossy().to_string();

    let mut finder = UnusedFinder::new_from_cfg(&logger, config).unwrap();
    let result = finder.find_unused(&logger).unwrap();
    let report = result.get_report();
    let normalized = normalize_test_report(&tmpdir, report);

    // `b` should be in unused_symbols (existing behavior).
    assert!(
        normalized
            .unused_symbols
            .get("<root>/packages/root/lib.js")
            .map_or(false, |syms| syms.iter().any(|s| s.id == "b")),
        "expected 'b' in unused_symbols for lib.js: {:#?}",
        normalized.unused_symbols
    );

    // The segment containing `b` should appear in unused_segments.
    let lib_unused_segs = normalized
        .unused_segments
        .get("<root>/packages/root/lib.js");
    assert!(
        lib_unused_segs.is_some(),
        "expected unused_segments entry for lib.js, got: {:#?}",
        normalized.unused_segments
    );
    let segs = lib_unused_segs.unwrap();
    assert!(
        !segs.is_empty(),
        "expected at least one unused segment in lib.js"
    );
    // Exactly one unused segment (the `export const b = 2;` statement).
    assert_eq!(
        segs.len(),
        1,
        "expected exactly 1 unused segment in lib.js, got: {:#?}",
        segs
    );
}

#[test]
fn test_segment_side_effect_keeps_deps_alive() {
    // The entry imports from helper.js. helper.js has a side-effect import
    // of polyfill.js. All segments in polyfill.js should be reachable
    // (not reported as unused).
    let tmpdir = test_tmpdir!(
        "packages/root/package.json" => r#"{
            "name": "entrypoint",
            "main": "./main.js",
            "exports": {}
        }"#,
        "packages/root/main.js" => r#"
            import { helper } from "./helper.js";
        "#,
        "packages/root/helper.js" => r#"
            import "./polyfill.js";
            export const helper = 1;
        "#,
        "packages/root/polyfill.js" => r#"
            const setup = "polyfill";
        "#
    );

    let logger = logger::StdioLogger::new();
    let mut config = UnusedFinderConfig {
        root_paths: vec![tmpdir.root().to_string_lossy().to_string()],
        entry_packages: vec!["entrypoint"].try_into().unwrap(),
        ..Default::default()
    };
    config.repo_root = tmpdir.root().to_string_lossy().to_string();

    let mut finder = UnusedFinder::new_from_cfg(&logger, config).unwrap();
    let result = finder.find_unused(&logger).unwrap();
    let report = result.get_report();
    let normalized = normalize_test_report(&tmpdir, report);

    // polyfill.js should NOT be in unused_files (it's imported for side effects).
    assert!(
        !normalized
            .unused_files
            .contains(&"<root>/packages/root/polyfill.js".to_string()),
        "polyfill.js should not be unused: {:#?}",
        normalized.unused_files
    );

    // polyfill.js should have no unused segments — the side-effect import
    // marks the entire file (and all its segments) as reachable.
    assert!(
        !normalized
            .unused_segments
            .contains_key("<root>/packages/root/polyfill.js"),
        "polyfill.js should have no unused segments: {:#?}",
        normalized.unused_segments
    );

    // helper.js should also have no unused segments.
    assert!(
        !normalized
            .unused_segments
            .contains_key("<root>/packages/root/helper.js"),
        "helper.js should have no unused segments: {:#?}",
        normalized.unused_segments
    );
}
