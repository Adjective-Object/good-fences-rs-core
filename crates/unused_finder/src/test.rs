use std::{collections::HashMap, path::PathBuf};

use path_slash::PathBufExt;
use test_tmpdir::{bmap, test_tmpdir};

use crate::{
    graph::UsedTag, logger, report::SymbolReport, UnusedFinder, UnusedFinderConfig,
    UnusedFinderReport,
};

fn symbol(id: &str, tags: UsedTag) -> SymbolReport {
    SymbolReport {
        id: id.to_string(),
        start: 0,
        end: 0,
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
                (k.replace(tmpdir.root().to_str().unwrap(), "<root>"), s_v)
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
            unused_symbols: bmap!(),
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
            unused_files: vec![],
            unused_symbols: bmap!(),
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
            unused_files: vec![],
            unused_symbols: bmap!(),
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
            unused_symbols: bmap!(
                "<root>/packages/root/imported-1.js" => vec![
                    symbol("b", UsedTag::default()),
                ]
            ),
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
            unused_symbols: bmap!(
                "<root>/packages/root/unused.js" => vec![
                    symbol("a", UsedTag::default()),
                    symbol("b", UsedTag::default()),
                ],
                "<root>/packages/root/ignored-exception.js" => vec![
                    symbol("exception", UsedTag::default()),
                ]
            ),
        },
    );
}
