mod leading_comments;

pub use leading_comments::leading_comments_at;

use oxc_allocator::Allocator;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;

/// Parse a file, always treating it as TypeScript (matching the swc parser behavior
/// which always used `Syntax::Typescript`). JSX/TSX is inferred from file extension.
pub fn parse_file<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &std::path::Path,
) -> ParserReturn<'a> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
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
        assert!(result.errors.is_empty(), "parse errors: {:?}", result.errors);
    }

    #[test]
    fn parse_tsx_fixture() {
        let source = "const App = () => <div className=\"test\">hello</div>;";
        let allocator = Allocator::default();
        let result = parse_tsx(&allocator, source);
        assert!(result.errors.is_empty(), "parse errors: {:?}", result.errors);
    }

    #[test]
    fn parse_file_infers_ts() {
        let source = "const x: number = 1;";
        let allocator = Allocator::default();
        let result = parse_file(&allocator, source, Path::new("test.ts"));
        assert!(result.errors.is_empty(), "parse errors: {:?}", result.errors);
    }

    #[test]
    fn parse_file_infers_tsx() {
        let source = "const App = () => <div />;";
        let allocator = Allocator::default();
        let result = parse_file(&allocator, source, Path::new("component.tsx"));
        assert!(result.errors.is_empty(), "parse errors: {:?}", result.errors);
    }
}
