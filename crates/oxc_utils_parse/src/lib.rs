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

/// Parse using extension-sensitive language modes and an explicit package policy.
///
/// `.mjs`/`.mts` and `.cjs`/`.cts` override the package format. Other supported
/// extensions use it without content-based module detection. This checks parser
/// diagnostics only; consumers must also check semantic-builder diagnostics.
pub fn parse_bundler_file<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &std::path::Path,
    package_format: ModuleFormat,
) -> eyre::Result<ParserReturn<'a>> {
    let mut source_type = SourceType::from_path(path)
        .wrap_err_with(|| format!("{}: unsupported source extension", path.display()))?;
    if source_type.is_unambiguous() {
        source_type = match package_format {
            ModuleFormat::Esm => source_type.with_module(true),
            ModuleFormat::CommonJs => source_type.with_commonjs(true),
        };
    }
    let parsed = Parser::new(allocator, source, source_type).parse();
    if source_type.is_commonjs() && path.extension().is_some_and(|ext| ext == "js") {
        use oxc_ast::ast::Statement;
        use oxc_span::GetSpan;
        if let Some(declaration) = parsed.program.body.iter().find(|statement| {
            matches!(
                statement,
                Statement::ImportDeclaration(_)
                    | Statement::ExportNamedDeclaration(_)
                    | Statement::ExportDefaultDeclaration(_)
                    | Statement::ExportAllDeclaration(_)
            )
        }) {
            let span = declaration.span();
            eyre::bail!(
                "{}: mixed CommonJS/ESM syntax at bytes {}..{}: this ESM .js input's package is missing required \"type\":\"module\" in package.json (or explicitly selects CommonJS); add \"type\":\"module\" or use .mjs instead of .js",
                path.display(), span.start, span.end
            );
        }
    }
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
    fn esm_javascript_requires_explicit_package_format() {
        for source in [
            "export const value = 1;",
            "export default 1;",
            "export * from './dep.js';",
            "import './dep.js';",
        ] {
            let allocator = Allocator::default();
            let error = parse_bundler_file(
                &allocator,
                source,
                Path::new("compiled.js"),
                ModuleFormat::CommonJs,
            )
            .err()
            .unwrap()
            .to_string();
            assert!(
                error.contains("compiled.js") && error.contains("bytes "),
                "{error}"
            );
            assert!(
                error.contains("missing required \"type\":\"module\""),
                "{error}"
            );
            assert!(error.contains("use .mjs instead of .js"), "{error}");
            assert!(parse_bundler_file(
                &allocator,
                source,
                Path::new("compiled.js"),
                ModuleFormat::Esm,
            )
            .is_ok());
            assert!(parse_bundler_file(
                &allocator,
                source,
                Path::new("compiled.mjs"),
                ModuleFormat::CommonJs,
            )
            .is_ok());
        }
        let allocator = Allocator::default();
        assert!(parse_bundler_file(
            &allocator,
            "import('./dep.js');",
            Path::new("dynamic.js"),
            ModuleFormat::CommonJs,
        )
        .is_ok());
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
