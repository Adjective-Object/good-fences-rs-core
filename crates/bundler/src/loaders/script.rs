//! Source-local precompilation; graph identity and resolution retain the input path.
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};

use ast_segmenter::{ModuleFormat, ParserConfig};
use cache_flatbuffers::{wire, MapData, MapSource, WireSourceMap};
use eyre::{ensure, WrapErr};
use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_semantic::SemanticBuilder;
use oxc_transformer::{Module, TransformOptions, Transformer};
use url::Url;

use super::{Loader, ScriptSyntax};

pub(crate) fn parser_path(path: &Path, syntax: ScriptSyntax) -> PathBuf {
    let original = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let prefix = match original {
        "mjs" | "mjsx" | "mts" | "mtsx" => "m",
        "cjs" | "cjsx" | "cts" | "ctsx" => "c",
        _ => "",
    };
    let suffix = match syntax {
        ScriptSyntax::Js => "js",
        ScriptSyntax::Jsx => "jsx",
        ScriptSyntax::Ts => "ts",
        ScriptSyntax::Tsx => "tsx",
    };
    path.with_extension(format!("{prefix}{suffix}"))
}

pub(crate) fn compile_selected(
    path: &Path,
    source: &str,
    package: ModuleFormat,
    source_maps: bool,
    loader: Loader,
) -> eyre::Result<Compiled> {
    let syntax = loader
        .script_syntax()
        .ok_or_else(|| eyre::eyre!("not a script loader"))?;
    compile_inner(
        path,
        &parser_path(path, syntax),
        source,
        package,
        source_maps,
    )
    .wrap_err_with(|| format!("{}: Oxc precompilation ({loader:?})", path.display()))
}

#[derive(Debug)]
pub(crate) struct Compiled {
    pub source: String,
    pub parser_path: PathBuf,
    pub source_map: Option<WireSourceMap>,
    pub source_map_hint: Option<oxc_span::Span>,
}

#[cfg(test)]
pub(crate) fn required(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ts" | "mts" | "cts" | "tsx" | "mtsx" | "ctsx" | "jsx" | "mjsx" | "cjsx")
    )
}

#[cfg(test)]
pub(crate) fn compile(
    path: &Path,
    source: &str,
    package: ModuleFormat,
    source_maps: bool,
) -> eyre::Result<Compiled> {
    compile_inner(path, path, source, package, source_maps)
        .wrap_err_with(|| format!("{}: Oxc precompilation", path.display()))
}

fn compile_inner(
    path: &Path,
    parser_path: &Path,
    source: &str,
    package: ModuleFormat,
    source_maps: bool,
) -> eyre::Result<Compiled> {
    let config = ParserConfig::for_path(parser_path, package)?;
    ensure!(
        !config.definition(),
        "TypeScript declaration files are not executable inputs"
    );
    let allocator = Allocator::default();
    let parsed = oxc_utils_parse::parse_bundler_file(&allocator, source, parser_path, package)?;
    let config = config.resolved(parsed.program.source_type);
    check_errors(
        "parse",
        parsed
            .diagnostics
            .into_iter()
            .map(|error| error.with_source_code(source.to_owned())),
    )?;
    let mut program = parsed.program;
    let source_map_hint = program.comments.iter().rev().find_map(|comment| {
        let text = &source[comment.span.start as usize..comment.span.end as usize];
        ast_segmenter::emission::source_mapping_url(text).map(|_| comment.span)
    });
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_enum_eval(true)
        .with_excess_capacity(2.0)
        .build(&program);
    check_errors(
        "semantic analysis",
        semantic
            .diagnostics
            .into_iter()
            .map(|error| error.with_source_code(source.to_owned())),
    )?;

    let mut options = TransformOptions::default();
    options.env.module = if config.commonjs() {
        Module::CommonJS
    } else {
        Module::Esm
    };
    // The pinned transformer implements legacy TypeScript decorators, not the
    // standard decorator proposal. Lower them rather than retaining runtime TS.
    options.decorator.legacy = true;
    let transformed = Transformer::new(&allocator, path, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    check_errors(
        "transform",
        transformed
            .diagnostics
            .into_iter()
            .map(|error| error.with_source_code(source.to_owned())),
    )?;
    if config.commonjs() {
        // TypeScriptAnnotations inserts a zero-span module marker after erasing
        // type-only imports/exports, even when the output format is CommonJS.
        program.body.retain(|statement| {
            !matches!(
                statement,
                Statement::ExportNamedDeclaration(declaration)
                    if declaration.span == oxc_span::SPAN
                        && declaration.specifiers.is_empty()
            )
        });
    }
    ensure!(
        !config.commonjs()
            || program
                .body
                .iter()
                .all(|statement| statement.as_module_declaration().is_none()),
        "the pinned Oxc transformer cannot lower ESM imports/exports to CommonJS"
    );
    let parser_path = path.with_extension(if config.commonjs() {
        "transpiled.cjs"
    } else {
        "transpiled.mjs"
    });
    let generated = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: source_maps.then(|| path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let source_map = generated
        .map
        .map(|map| checked_map(path, source, &generated.code, &map))
        .transpose()?;
    Ok(Compiled {
        source: generated.code,
        parser_path,
        source_map,
        source_map_hint,
    })
}

fn check_errors(phase: &str, errors: impl IntoIterator<Item = impl Debug>) -> eyre::Result<()> {
    let diagnostics = errors
        .into_iter()
        .map(|error| format!("{error:?}"))
        .collect::<Vec<_>>();
    ensure!(
        diagnostics.is_empty(),
        "{phase} failed:\n{}",
        diagnostics.join("\n")
    );
    Ok(())
}

fn checked_map(
    path: &Path,
    original: &str,
    generated: &str,
    map: &oxc_sourcemap::SourceMap<'_>,
) -> eyre::Result<WireSourceMap> {
    let absolute = std::path::absolute(path)?;
    let location = Url::from_file_path(&absolute)
        .map_err(|()| eyre::eyre!("invalid precompiler source path: {}", path.display()))?;
    ensure!(
        map.get_sources().count() == 1,
        "unexpected Oxc precompiler source map inputs"
    );
    let sources = [MapSource {
        original: location.as_str(),
        url: location.as_str(),
        content: Some(original),
    }];
    let names = map.get_names().map(AsRef::as_ref).collect::<Vec<_>>();
    let tokens = map
        .get_tokens()
        .map(|token| {
            wire::MapToken::new(
                token.get_dst_line(),
                token.get_dst_col(),
                token.get_src_line(),
                token.get_src_col(),
                token.get_source_id().map_or(-1, |id| id as i32),
                token.get_name_id().map_or(-1, |id| id as i32),
            )
        })
        .collect::<Vec<_>>();
    WireSourceMap::encode(
        generated,
        map.to_json_string().as_bytes(),
        MapData {
            location: location.as_str(),
            file: None,
            source_root: None,
            sources: &sources,
            names: &names,
            tokens: &tokens,
        },
    )?
    .snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_parser::Parser;

    fn compile_source(path: &str, source: &str, package: ModuleFormat) -> Compiled {
        let compiled = compile(Path::new(path), source, package, false).unwrap();
        let allocator = Allocator::default();
        let config = ParserConfig::for_path(&compiled.parser_path, package).unwrap();
        assert!(!config.typescript());
        assert!(!config.jsx());
        let parsed = Parser::new(&allocator, &compiled.source, config.source_type()).parse();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let semantic = SemanticBuilder::new()
            .with_check_syntax_error(true)
            .build(&parsed.program);
        assert!(
            semantic.diagnostics.is_empty(),
            "{:?}",
            semantic.diagnostics
        );
        compiled
    }

    #[test]
    fn transpile_typescript_runtime_and_type_imports() {
        let compiled = compile_source(
            "entry.mts",
            r#"import type { Shape } from "./types";
import { value } from "./value";
export enum Color { Red = 2, Blue }
export namespace Models { export const selected: Color = Color.Blue; }
export const result: number = value + Models.selected;"#,
            ModuleFormat::CommonJs,
        );
        assert_eq!(compiled.parser_path, Path::new("entry.transpiled.mjs"));
        assert!(!compiled.source.contains("./types"));
        assert!(compiled.source.contains("./value"));
        assert!(compiled.source.contains("Color"));
        assert!(compiled.source.contains("Models"));
        assert!(!compiled.source.contains(": number"));
        assert!(!compiled.source.contains("namespace"));
        assert!(!compiled.source.contains("enum "));
    }

    #[test]
    fn transpile_commonjs_import_equals_and_export_assignment() {
        let compiled = compile_source(
            "entry.cts",
            r#"import value = require("./value");
const result: number = value + 1;
export = result;"#,
            ModuleFormat::Esm,
        );
        assert_eq!(compiled.parser_path, Path::new("entry.transpiled.cjs"));
        assert!(compiled.source.contains("require(\"./value\")"));
        assert!(compiled.source.contains("module.exports = result"));
        assert!(!compiled.source.contains("import "));
    }

    #[test]
    fn transpile_commonjs_erases_synthesized_type_only_module_marker() {
        let compiled = compile_source(
            "entry.cts",
            "import type { T } from './missing-types'; const value: T = 42; export = value;",
            ModuleFormat::Esm,
        );
        assert_eq!(compiled.parser_path, Path::new("entry.transpiled.cjs"));
        assert!(compiled.source.contains("const value = 42;"));
        assert!(compiled.source.contains("module.exports = value;"));
        assert!(!compiled.source.contains("missing-types"));
        assert!(!compiled.source.contains("import "));
        assert!(!compiled.source.contains("export "));
    }

    #[test]
    fn transpile_jsx_automatic_runtime_preserves_module_format() {
        for path in ["view.jsx", "view.tsx", "view.mjsx", "view.mtsx"] {
            let esm = compile_source(
                path,
                "export const view = <><div title=\"hello\" /></>;",
                ModuleFormat::Esm,
            );
            assert!(esm.source.contains("react/jsx-runtime"));
            assert!(esm.source.contains("import "));
            assert!(!esm.source.contains("<div"));
            assert!(!esm.source.contains("React.createElement"));
            let commonjs =
                compile_source(path, "module.exports = <div />;", ModuleFormat::CommonJs);
            let config = ParserConfig::for_path(Path::new(path), ModuleFormat::CommonJs).unwrap();
            if config.commonjs() {
                assert!(commonjs.source.contains("require(\"react/jsx-runtime\")"));
                assert!(!commonjs.source.contains("import "));
            } else {
                assert!(commonjs.source.contains("import "));
            }
        }
        for path in ["view.cjsx", "view.ctsx"] {
            let commonjs = compile_source(path, "module.exports = <div />;", ModuleFormat::Esm);
            assert!(commonjs.source.contains("require(\"react/jsx-runtime\")"));
            assert!(!commonjs.source.contains("import "));
        }
    }

    #[test]
    fn transpile_decorators_are_lowered() {
        let compiled = compile_source(
            "decorated.ts",
            "function mark(value: any) { return value; }\n@mark\nclass Example { value: number = 1; }\nexport { Example };",
            ModuleFormat::Esm,
        );
        assert!(!compiled.source.contains("@mark"));
        assert!(compiled.source.contains("mark("));
    }

    #[test]
    fn transpile_rejects_declarations_and_reports_original_diagnostics() {
        for path in ["types.d.ts", "types.d.mts", "types.d.cts"] {
            assert!(required(Path::new(path)));
            let error = compile(
                Path::new(path),
                "declare const value: number;",
                ModuleFormat::Esm,
                false,
            )
            .unwrap_err();
            assert!(format!("{error:#}").contains("declaration files"));
        }
        for (source, phase) in [
            ("const value: = 1;", "parse"),
            ("let value: number; let value: number;", "semantic analysis"),
            ("export = 1;", "transform"),
        ] {
            let error =
                compile(Path::new("invalid.mts"), source, ModuleFormat::Esm, false).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("invalid.mts"), "{message}");
            assert!(message.contains(phase), "{message}");
        }
        for source in [
            "import { value } from './value'; module.exports = value;",
            "import './side-effects';",
            "export const value = 42;",
            "const value = 42; export { value };",
            "export { value } from './value';",
            "export * from './value';",
            "export {} from './side-effects';",
            "export {};",
        ] {
            for path in ["invalid.cts", "invalid.ctsx"] {
                let error =
                    compile(Path::new(path), source, ModuleFormat::CommonJs, false).unwrap_err();
                assert!(
                    format!("{error:#}").contains("cannot lower ESM imports/exports to CommonJS"),
                    "{source}: {error:#}"
                );
            }
        }
        assert!(!required(Path::new("plain.js")));
    }

    #[test]
    fn transpile_maps_preserve_original_identity_and_utf16_columns() {
        let path = std::env::current_dir().unwrap().join("original ü.ts");
        let source = "export const café: string = \"😀\"; console.log(café);\n\
                      export const \\u0061: number = 1;\n";
        let off = compile(&path, source, ModuleFormat::Esm, false).unwrap();
        let on = compile(&path, source, ModuleFormat::Esm, true).unwrap();
        assert_eq!(off.source, on.source);
        assert!(off.source_map.is_none());
        let map = on.source_map.unwrap();
        let url = Url::from_file_path(&path).unwrap();
        assert_eq!(map.root().location(), url.as_str());
        assert_eq!(map.root().sources().len(), 1);
        assert_eq!(map.root().sources().get(0).url(), url.as_str());
        assert_eq!(map.root().sources().get(0).content(), Some(source));
        let original_column = source[..source.rfind("café").unwrap()]
            .encode_utf16()
            .count() as u32;
        let generated_offset = on.source.rfind("café").unwrap();
        let prefix = &on.source[..generated_offset];
        let generated_line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
        let generated_column = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        let token = map.lookup(generated_line, generated_column).unwrap();
        assert_eq!(token.source(), 0);
        assert_eq!(token.original_line(), 0);
        assert_eq!(token.original_column(), original_column);
        assert!(map.root().names().iter().any(|name| name == "\\u0061"));
        assert!(map.root().tokens().iter().any(|token| token.name() >= 0));
    }

    #[test]
    fn transpile_original_map_directive_is_parser_identified_even_with_maps_off() {
        let source = "const misleading: string = '//# sourceMappingURL=not-a-map';\n\
                      //# sourceMappingURL=first.map\n\
                      export { misleading };\n\
                      /*# sourceMappingURL=last.map */";
        for enabled in [false, true] {
            let compiled =
                compile(Path::new("input.ts"), source, ModuleFormat::Esm, enabled).unwrap();
            let span = compiled.source_map_hint.unwrap();
            assert_eq!(
                ast_segmenter::emission::source_mapping_url(
                    &source[span.start as usize..span.end as usize]
                ),
                Some("last.map")
            );
        }
        let compiled = compile(
            Path::new("input.ts"),
            "const misleading: string = '//# sourceMappingURL=not-a-map';",
            ModuleFormat::Esm,
            false,
        )
        .unwrap();
        assert!(compiled.source_map_hint.is_none());
    }
}
