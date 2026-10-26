use super::*;
use oxc_sourcemap::{SourceMap, SourceMapBuilder};

fn coordinates(text: &str, offset: usize) -> (u32, u32) {
    let mut line = 0;
    let mut column = 0;
    let mut cr = false;
    for ch in text[..offset].chars() {
        match ch {
            '\n' if cr => {}
            '\r' | '\n' | '\u{2028}' | '\u{2029}' => {
                line += 1;
                column = 0;
            }

            _ => column += ch.len_utf16() as u32,
        }
        cr = ch == '\r';
    }
    (line, column)
}

#[test]
fn explicit_jsx_module_extensions_preserve_format_through_resolution_and_cache_hits() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"commonjs"}"#);
    write(dir.root(), "main.mjs",
        "import typed from './typed.mjsx'; import plain from './plain.mjsx'; \
         import legacy from './legacy.cjsx'; import legacyTyped from './legacy-typed.cjsx'; \
         console.log(typed.props.children, plain.props.children, legacy.props.children, legacyTyped.props.children);");
    write(
        dir.root(),
        "typed.mtsx",
        "const view: unknown = <div>typed</div>; export default view;",
    );
    write(dir.root(), "plain.mjsx", "export default <div>plain</div>;");
    write(
        dir.root(),
        "legacy.cjsx",
        "module.exports = <div>legacy</div>;",
    );
    write(
        dir.root(),
        "legacy-typed.ctsx",
        "const view: unknown = <div>typed-legacy</div>; export = view;",
    );
    write(
        dir.root(),
        "node_modules/react/package.json",
        r#"{"exports":{"./jsx-runtime":{"import":"./runtime.mjs","require":"./runtime.cjs"}}}"#,
    );
    write(
        dir.root(),
        "node_modules/react/runtime.mjs",
        "export function jsx(type, props) { return {type, props}; } export {jsx as jsxs};",
    );
    write(
        dir.root(),
        "node_modules/react/runtime.cjs",
        "exports.jsx = exports.jsxs = (type, props) => ({type, props});",
    );
    let config = options();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let cold = session.build(&config).unwrap();
    assert_eq!(
        observe(&cold.product.artifacts),
        b"typed plain legacy typed-legacy\n"
    );
    assert_eq!(session.build(&config).unwrap().counters.source_parses, 0);
    let mut fresh = BuildSession::new(dir.root()).unwrap();
    let disk = fresh.build(&config).unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.counters.source_reparses, 0);
    assert_eq!(disk.product.artifacts, cold.product.artifacts);
    write(
        dir.root(),
        "legacy.cjsx",
        "export default <div>invalid</div>;",
    );
    let error = format!("{:#}", session.build(&config).err().unwrap());
    assert!(error.contains("explicit .cjs/.cjsx"), "{error}");
}

fn origin(artifacts: &Artifacts, needle: &str) -> (String, u32, u32, Option<String>, String) {
    for (path, text) in &artifacts.files {
        if !crate::sourcemap::executable(path) {
            continue;
        }
        let Some(offset) = text.find(needle) else {
            continue;
        };
        let map_path = crate::sourcemap::path(path);
        let map = SourceMap::from_json_string(&artifacts.files[&map_path]).unwrap();
        let table = map.generate_lookup_table();
        let (line, column) = coordinates(text, offset);
        let token = map.lookup_source_view_token(&table, line, column).unwrap();
        if let Some(source) = token.get_source() {
            return (
                source.to_string(),
                token.get_src_line(),
                token.get_src_col(),
                token.get_name().map(ToString::to_string),
                token.get_source_content().unwrap().to_string(),
            );
        }
    }
    panic!("no original source for {needle}");
}

#[test]
fn transpiled_typescript_tsx_jsx_and_cts_execute_before_native_linking() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    write(
        dir.root(),
        "main.ts",
        "import {answer} from './answer.js'; import View from './view.tsx'; import label from './label.jsx'; import legacy from './legacy.cts'; console.log(answer, View().props.children, label.props.children, legacy);",
    );
    write(
        dir.root(),
        "answer.ts",
        "import type {Missing} from './does-not-exist'; export enum Kind {Answer = 42} export const answer: Kind = Kind.Answer;",
    );
    write(
        dir.root(),
        "view.tsx",
        "export default function View(): JSX.Element { return <main>view</main>; }",
    );
    write(
        dir.root(),
        "label.jsx",
        "export default <label>label</label>;",
    );
    write(
        dir.root(),
        "legacy.cts",
        "const value: number = 7; export = value;",
    );
    write(
        dir.root(),
        "node_modules/react/package.json",
        r#"{"type":"module","exports":{"./jsx-runtime":"./jsx-runtime.mjs"}}"#,
    );
    write(
        dir.root(),
        "node_modules/react/jsx-runtime.mjs",
        "export function jsx(type, props) { return {type, props}; } export {jsx as jsxs};",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.ts".into();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let result = session.build(&config).unwrap();
    assert_eq!(
        result.product.artifacts.entries["main"],
        Path::new("main.mjs")
    );
    assert_eq!(observe(&result.product.artifacts), b"42 view label 7\n");
    for file in [
        "main.ts",
        "answer.ts",
        "view.tsx",
        "label.jsx",
        "legacy.cts",
    ] {
        let path = dir.root().join(file);
        let source = &session.sources[&path];
        assert!(source.text.generated_backing().unwrap().is_mapped());
        assert!(source
            .transpiled
            .as_ref()
            .unwrap()
            .source_map
            .as_ref()
            .unwrap()
            .backing()
            .is_mapped());
        assert!(!source.record.config().typescript());
        assert!(!source.record.config().jsx());
        let file = result.product.graph.file_id(&path).unwrap();
        assert!(result.product.graph.file_prepared_resource(file).is_some());
    }
    let mut parallel = config.clone();
    parallel.execution =
        crate::execution::ExecutionOptions::new(std::num::NonZeroUsize::new(4).unwrap());
    let parallel = BuildSession::new(dir.root())
        .unwrap()
        .build(&parallel)
        .unwrap();
    assert_eq!(parallel.product.artifacts, result.product.artifacts);
    assert_eq!(parallel.counters.source_parses, 0);
}

#[test]
fn transpiled_commonjs_entries_and_jsx_runtime_keep_commonjs_activation() {
    let dir = TmpDir::new();
    write(
        dir.root(),
        "main.cts",
        "import type {Answer} from './missing-types'; import view = require('./view.jsx'); const answer: Answer = 42; console.log(view.props.children, answer); export = answer;",
    );
    write(dir.root(), "view.jsx", "module.exports = <div>hello</div>;");
    write(
        dir.root(),
        "node_modules/react/package.json",
        r#"{"exports":{"./jsx-runtime":"./jsx-runtime.cjs"}}"#,
    );
    write(
        dir.root(),
        "node_modules/react/jsx-runtime.cjs",
        "exports.jsx = function(type, props) { return {type, props}; };",
    );
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.cts".into();
    config.output.source_maps = true;
    let result = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(
        result.product.artifacts.entries["main"],
        Path::new("main.cjs")
    );
    assert_eq!(observe(&result.product.artifacts), b"hello 42\n");
}

#[test]
fn transpiler_maps_toggle_warm_disk_and_mapping_only_edits_without_reextracting() {
    let dir = TmpDir::new();
    let source = "type Unused = number;\r\nexport const emoji = '😀'; export const answer: number = 42;\r\nconsole.log(answer);";
    write(dir.root(), "main.mts", source);
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    let mut session = BuildSession::new(dir.root()).unwrap();
    let off = session.build(&config).unwrap();
    assert!(session.sources[&dir.root().join("main.mts")]
        .transpiled
        .as_ref()
        .unwrap()
        .source_map
        .is_none());
    config.output.source_maps = true;
    let on = session.build(&config).unwrap();
    assert_eq!(on.counters.source_parses, 0);
    assert_eq!(on.counters.resolutions, 0);
    assert_eq!(
        off.product.fingerprints.parse,
        on.product.fingerprints.parse
    );
    let (url, line, column, _, content) = origin(&on.product.artifacts, "42");
    assert_eq!(
        url,
        url::Url::from_file_path(dir.root().join("main.mts"))
            .unwrap()
            .as_str()
    );
    assert_eq!(
        (line, column),
        coordinates(source, source.find("42").unwrap())
    );
    assert_eq!(content, source);
    let noop = session.build(&config).unwrap();
    assert!(Rc::ptr_eq(&on.product, &noop.product));
    let disk = BuildSession::new(dir.root())
        .unwrap()
        .build(&config)
        .unwrap();
    assert_eq!(disk.counters.source_parses, 0);
    assert_eq!(disk.product.artifacts, on.product.artifacts);

    let prepared = session.sources[&dir.root().join("main.mts")]
        .text
        .text(&dir.root().join("main.mts"))
        .unwrap();
    let edited_source = source.replace(
        "type Unused = number;",
        "type Unused = number;\r\ntype Other = string;",
    );
    write(dir.root(), "main.mts", &edited_source);
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.source_parses, 0);
    assert_eq!(
        session.sources[&dir.root().join("main.mts")]
            .text
            .text(&dir.root().join("main.mts"))
            .unwrap(),
        prepared
    );
    assert_eq!(
        on.product.fingerprints.parse,
        edited.product.fingerprints.parse
    );
    assert_ne!(
        on.product.fingerprints.output,
        edited.product.fingerprints.output
    );
    let (_, line, column, _, content) = origin(&edited.product.artifacts, "42");
    assert_eq!(
        (line, column),
        coordinates(&edited_source, edited_source.find("42").unwrap())
    );
    assert_eq!(content, edited_source);
    config.output.source_maps = false;
    let disabled = session.build(&config).unwrap();
    assert_eq!(disabled.counters.source_parses, 0);
    assert_eq!(disabled.product.artifacts, off.product.artifacts);
}

#[test]
fn transpiler_maps_compose_original_linked_maps_and_track_map_only_edits() {
    let dir = TmpDir::new();
    write(dir.root(), "package.json", r#"{"type":"module"}"#);
    let source = "export const answer: number = 42;\n//# sourceMappingURL=original.map";
    write(dir.root(), "main.ts", source);
    let map = |line| {
        let mut map = SourceMapBuilder::default();
        let id = map.add_source_and_content("original.input", "upstream source");
        let name = map.add_name("originalAnswer");
        map.add_token(0, 0, line, 3, Some(id), Some(name));
        map.into_sourcemap().to_json_string()
    };
    write(dir.root(), "original.map", &map(7));
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.ts".into();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    let (url, line, column, name, content) = origin(&first.product.artifacts, "42");
    assert_eq!(
        url,
        url::Url::from_file_path(dir.root().join("original.input"))
            .unwrap()
            .as_str()
    );
    assert_eq!((line, column), (7, 3));
    assert_eq!(name.as_deref(), Some("originalAnswer"));
    assert_eq!(content, "upstream source");
    write(dir.root(), "original.map", &map(11));
    let edited = session.build(&config).unwrap();
    assert_eq!(edited.counters.source_parses, 0);
    assert_eq!(origin(&edited.product.artifacts, "42").1, 11);
    assert_ne!(
        first.product.fingerprints.output,
        edited.product.fingerprints.output
    );

    let mut sparse = SourceMapBuilder::default();
    let id = sparse.add_source_and_content("original.input", "upstream source");
    sparse.add_token(0, 0, 7, 3, Some(id), None);
    sparse.add_token(0, source.find("42").unwrap() as u32, 0, 0, None, None);
    write(
        dir.root(),
        "original.map",
        &sparse.into_sourcemap().to_json_string(),
    );
    let sparse = session.build(&config).unwrap();
    let mut checked = false;
    for (path, text) in &sparse.product.artifacts.files {
        if !crate::sourcemap::executable(path) {
            continue;
        }
        let Some(offset) = text.find("42") else {
            continue;
        };
        let map = SourceMap::from_json_string(
            &sparse.product.artifacts.files[&crate::sourcemap::path(path)],
        )
        .unwrap();
        let table = map.generate_lookup_table();
        let (line, column) = coordinates(text, offset);
        assert!(map
            .lookup_source_view_token(&table, line, column)
            .unwrap()
            .get_source()
            .is_none());
        checked = true;
    }
    assert!(checked);
}

#[test]
fn transpilation_reports_original_syntax_and_declaration_input_errors() {
    let dir = TmpDir::new();
    let mut config = options();
    for (file, source) in [
        ("broken.ts", "export const value: = 1;"),
        ("broken.tsx", "export default <div>;"),
        ("types.d.ts", "declare const value: number; export {value};"),
    ] {
        write(dir.root(), file, source);
        config.entries.get_mut("main").unwrap().path = file.into();
        let error = BuildSession::new(dir.root())
            .unwrap()
            .build(&config)
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains(file), "{error:#}");
    }
}

#[test]
fn transpilation_failure_and_original_input_recheck_do_not_return_stale_success() {
    let dir = TmpDir::new();
    write(dir.root(), "main.mts", "export const answer: number = 42;");
    let mut config = options();
    config.entries.get_mut("main").unwrap().path = "main.mts".into();
    config.output.source_maps = true;
    let mut session = BuildSession::new(dir.root()).unwrap();
    let first = session.build(&config).unwrap();
    write(dir.root(), "main.mts", "export const answer: = 1;");
    assert!(session.build(&config).is_err());
    write(dir.root(), "main.mts", "export const answer: number = 43;");
    let recovered = session.build(&config).unwrap();
    assert_ne!(first.product.artifacts, recovered.product.artifacts);
    assert_eq!(origin(&recovered.product.artifacts, "43").1, 0);

    let root = dir.root().to_owned();
    let observer = observe_preparation(move |path| {
        if path.ends_with("main.mts") {
            write(&root, "main.mts", "export const answer: number = 44;");
        }
    });
    let error = session.build(&config).err().unwrap();
    assert!(format!("{error:#}").contains("changed"), "{error:#}");
    drop(observer);
    let rebuilt = session.build(&config).unwrap();
    assert_eq!(origin(&rebuilt.product.artifacts, "44").1, 0);
}
