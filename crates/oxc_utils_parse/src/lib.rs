mod leading_comments;

pub use leading_comments::leading_comments_at;

use eyre::{ensure, WrapErr};
use oxc_allocator::Allocator;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;

/// Package interpretation for extensions without an explicit module format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleFormat {
    Esm,
    CommonJs,
}

/// Infer language and explicit module modes, including prefixed JSX extensions.
pub fn bundler_source_type(path: &std::path::Path) -> eyre::Result<SourceType> {
    Ok(
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("mjsx") => SourceType::jsx().with_module(true),
            Some("mtsx") => SourceType::tsx().with_module(true),
            Some("cjsx") => SourceType::jsx().with_commonjs(true),
            Some("ctsx") => SourceType::tsx().with_commonjs(true),
            _ => SourceType::from_path(path)?,
        },
    )
}

/// Parse using extension-sensitive language modes and an explicit package policy.
///
/// Explicit module extensions override the package fallback. Static ESM syntax
/// disambiguates `.js`/`.jsx`/`.ts`/`.tsx` CommonJS-default inputs. This checks parser
/// diagnostics only; consumers must also check semantic-builder diagnostics.
pub fn parse_bundler_file<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &std::path::Path,
    package_format: ModuleFormat,
) -> eyre::Result<ParserReturn<'a>> {
    let mut source_type = bundler_source_type(path)
        .wrap_err_with(|| format!("{}: unsupported source extension", path.display()))?;
    let detect_esm = source_type.is_unambiguous() && package_format == ModuleFormat::CommonJs;
    if source_type.is_unambiguous() {
        source_type = match package_format {
            ModuleFormat::Esm => source_type.with_module(true),
            ModuleFormat::CommonJs => source_type.with_commonjs(true),
        };
    }
    let parsed = Parser::new(allocator, source, source_type).parse();
    if source_type.is_commonjs()
        && path
            .extension()
            .is_some_and(|extension| extension == "cjs" || extension == "cjsx")
        && parsed.module_record.has_module_syntax
    {
        use oxc_span::GetSpan;
        let span = parsed
            .program
            .body
            .iter()
            .find(|statement| statement.as_module_declaration().is_some())
            .map_or(parsed.program.span, GetSpan::span);
        eyre::bail!(
            "{}: mixed CommonJS/ESM syntax at bytes {}..{}: ESM syntax is not allowed in an explicit .cjs/.cjsx input; use an ESM or ambiguous extension",
            path.display(), span.start, span.end
        );
    }
    // Reparse detected modules in ESM mode to enforce strict/module grammar;
    // the CommonJS probe preserves wrapper returns for actual CommonJS inputs.
    let parsed = if detect_esm && parsed.module_record.has_module_syntax {
        Parser::new(allocator, source, source_type.with_module(true)).parse()
    } else {
        parsed
    };
    ensure!(
        !parsed.panicked && parsed.errors.is_empty(),
        "{}: parser diagnostics (panicked={}): {:?}",
        path.display(),
        parsed.panicked,
        parsed.errors
    );
    Ok(parsed)
}

/// Parse a file, always treating it as TypeScript (matching the swc parser behavior
/// which always used `Syntax::Typescript`). JSX/TSX is inferred from file extension.
pub fn parse_file<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &std::path::Path,
) -> ParserReturn<'a> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let source_type = match ext {
        "tsx" | "jsx" => SourceType::tsx(),
        // Always parse as TypeScript even for .js/.mjs/.cjs files,
        // since many codebases use TS syntax in .js files.
        _ => SourceType::ts(),
    };
    Parser::new(allocator, source, source_type).parse()
}

/// Parse TypeScript source (`.ts`).
pub fn parse_ts<'a>(allocator: &'a Allocator, source: &'a str) -> ParserReturn<'a> {
    Parser::new(allocator, source, SourceType::ts()).parse()
}

/// Parse TSX source (`.tsx`).
pub fn parse_tsx<'a>(allocator: &'a Allocator, source: &'a str) -> ParserReturn<'a> {
    Parser::new(allocator, source, SourceType::tsx()).parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_ts_fixture() {
        let source = "const x: number = 42;\nfunction foo(a: string): boolean { return true; }";
        let allocator = Allocator::default();
        let result = parse_ts(&allocator, source);
        assert!(
            result.errors.is_empty(),
            "parse errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn parse_tsx_fixture() {
        let source = "const App = () => <div className=\"test\">hello</div>;";
        let allocator = Allocator::default();
        let result = parse_tsx(&allocator, source);
        assert!(
            result.errors.is_empty(),
            "parse errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn parse_file_infers_ts() {
        let source = "const x: number = 1;";
        let allocator = Allocator::default();
        let result = parse_file(&allocator, source, Path::new("test.ts"));
        assert!(
            result.errors.is_empty(),
            "parse errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn parse_file_infers_tsx() {
        let source = "const App = () => <div />;";
        let allocator = Allocator::default();
        let result = parse_file(&allocator, source, Path::new("component.tsx"));
        assert!(
            result.errors.is_empty(),
            "parse errors: {:?}",
            result.errors
        );
    }

    #[test]
    fn bundler_language_and_module_policy() {
        for (path, source, typescript, jsx, explicit_format) in [
            ("file.js", "const value = 1;", false, false, None),
            ("file.jsx", "const value = <div />;", false, true, None),
            ("file.ts", "const value: number = 1;", true, false, None),
            (
                "file.tsx",
                "const value: unknown = <div />;",
                true,
                true,
                None,
            ),
            (
                "file.mjs",
                "export const value = 1;",
                false,
                false,
                Some(ModuleFormat::Esm),
            ),
            (
                "file.mts",
                "export const value: number = 1;",
                true,
                false,
                Some(ModuleFormat::Esm),
            ),
            (
                "file.mjsx",
                "const value = <div />;",
                false,
                true,
                Some(ModuleFormat::Esm),
            ),
            (
                "file.mtsx",
                "const value: unknown = <div />;",
                true,
                true,
                Some(ModuleFormat::Esm),
            ),
            (
                "file.cjsx",
                "module.exports = <div />; return;",
                false,
                true,
                Some(ModuleFormat::CommonJs),
            ),
            (
                "file.ctsx",
                "const value: unknown = <div />; return;",
                true,
                true,
                Some(ModuleFormat::CommonJs),
            ),
            (
                "file.cjs",
                "module.exports = 1; return;",
                false,
                false,
                Some(ModuleFormat::CommonJs),
            ),
            (
                "file.cts",
                "const value: number = 1; return;",
                true,
                false,
                Some(ModuleFormat::CommonJs),
            ),
        ] {
            for package_format in [ModuleFormat::Esm, ModuleFormat::CommonJs] {
                let allocator = Allocator::default();
                let parsed =
                    parse_bundler_file(&allocator, source, Path::new(path), package_format)
                        .unwrap();
                let source_type = parsed.program.source_type;
                assert_eq!(source_type.is_typescript(), typescript, "{path}");
                assert_eq!(source_type.is_jsx(), jsx, "{path}");
                assert_eq!(
                    source_type.is_commonjs(),
                    explicit_format.unwrap_or(package_format) == ModuleFormat::CommonJs,
                    "{path}"
                );
            }
        }
    }

    #[test]
    fn esm_syntax_overrides_the_commonjs_package_fallback() {
        for source in [
            "export const value = 1;",
            "export default 1;",
            "export * from './dep.js';",
            "import './dep.js';",
            "const url = import.meta.url;",
        ] {
            for extension in ["js", "jsx", "ts", "tsx"] {
                let allocator = Allocator::default();
                let parsed = parse_bundler_file(
                    &allocator,
                    source,
                    Path::new(&format!("compiled.{extension}")),
                    ModuleFormat::CommonJs,
                )
                .unwrap();
                assert!(
                    parsed.program.source_type.is_module(),
                    "{extension}: {source}"
                );
            }
        }
        for source in [
            "import('./dep.js');",
            "module.exports = 'export default 1'; return;",
            "/* import './dep.js' */ exports.value = 1;",
        ] {
            let allocator = Allocator::default();
            let parsed = parse_bundler_file(
                &allocator,
                source,
                Path::new("common.js"),
                ModuleFormat::CommonJs,
            )
            .unwrap();
            assert!(parsed.program.source_type.is_commonjs(), "{source}");
        }
        for source in ["export {}; return;", "export const await = 1;"] {
            let allocator = Allocator::default();
            assert!(
                parse_bundler_file(
                    &allocator,
                    source,
                    Path::new("invalid.js"),
                    ModuleFormat::CommonJs,
                )
                .is_err(),
                "{source}"
            );
        }
        for source in [
            "export const value = 1;",
            "import './dep.js';",
            "import.meta.url;",
        ] {
            for path in ["explicit.cjs", "explicit.cjsx"] {
                let allocator = Allocator::default();
                let error =
                    parse_bundler_file(&allocator, source, Path::new(path), ModuleFormat::Esm)
                        .err()
                        .unwrap();
                assert!(error.to_string().contains("explicit .cjs"), "{error}");
            }
        }
    }

    #[test]
    fn bundler_errors_do_not_return_recovered_programs() {
        for (path, source) in [
            ("broken.js", "export const broken = ;"),
            ("typed.js", "const value: number = 1;"),
            ("component.js", "const value = <div />;"),
            ("file.json", "{}"),
            ("file", "const value = 1;"),
        ] {
            let allocator = Allocator::default();
            let error = parse_bundler_file(&allocator, source, Path::new(path), ModuleFormat::Esm)
                .err()
                .expect("invalid input must not return an AST");
            assert!(error.to_string().contains(path), "{error}");
        }
    }

    #[test]
    fn tracker_policy_still_accepts_typescript_in_javascript_files() {
        let allocator = Allocator::default();
        for path in ["file.js", "file.cjs", "file.mjs", "file.unknown"] {
            for (source, is_module) in [
                ("const value: number = 1;", false),
                ("export const value: number = 1;", true),
            ] {
                let parsed = parse_file(&allocator, source, Path::new(path));
                assert!(parsed.errors.is_empty(), "{path}: {:?}", parsed.errors);
                assert!(parsed.program.source_type.is_typescript());
                assert_eq!(parsed.program.source_type.is_module(), is_module);
                assert!(!parsed.program.source_type.is_commonjs());
            }
        }
    }
}
