pub mod leading_comments;
pub use leading_comments::leading_comments_at;

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;

/// Parse a TypeScript or TSX file, treating the source as TypeScript regardless
/// of extension. Uses TSX mode for `.jsx`/`.tsx` files; plain TS for all others
/// (including `.js`/`.mjs`). Falls back to TS for unrecognised extensions.
///
/// This allows `.js` files to contain TypeScript syntax, which is common in
/// projects that mix `.js` and `.ts` files.
pub fn parse_file<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &Path,
) -> ParserReturn<'a> {
    let source_type = match path.extension().and_then(|e| e.to_str()) {
        Some("jsx") | Some("tsx") => SourceType::tsx().with_module(true),
        _ => SourceType::ts().with_module(true),
    };
    Parser::new(allocator, source, source_type).parse()
}

/// Parse source text as TypeScript (`.ts`, module mode).
pub fn parse_ts<'a>(allocator: &'a Allocator, source: &'a str) -> ParserReturn<'a> {
    Parser::new(allocator, source, SourceType::ts().with_module(true)).parse()
}

/// Parse source text as TypeScript + JSX (`.tsx`, module mode).
pub fn parse_tsx<'a>(allocator: &'a Allocator, source: &'a str) -> ParserReturn<'a> {
    Parser::new(allocator, source, SourceType::tsx().with_module(true)).parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_ts_fixture_no_errors() {
        let source = r#"
            export interface Foo { bar: string; }
            export const x: Foo = { bar: "hello" };
        "#;
        let alloc = Allocator::default();
        let ret = parse_ts(&alloc, source);
        assert!(ret.errors.is_empty(), "unexpected errors: {:?}", ret.errors);
        assert!(!ret.panicked);
    }

    #[test]
    fn parse_tsx_fixture_no_errors() {
        let source = r#"
            import React from "react";
            export const Comp = () => <div className="x">hello</div>;
        "#;
        let alloc = Allocator::default();
        let ret = parse_tsx(&alloc, source);
        assert!(ret.errors.is_empty(), "unexpected errors: {:?}", ret.errors);
        assert!(!ret.panicked);
    }

    #[test]
    fn parse_file_ts_extension() {
        let source = "export const n = 42;";
        let alloc = Allocator::default();
        let ret = parse_file(&alloc, source, &PathBuf::from("index.ts"));
        assert!(ret.errors.is_empty(), "unexpected errors: {:?}", ret.errors);
    }

    #[test]
    fn parse_file_tsx_extension() {
        let source = "export const el = <span />;";
        let alloc = Allocator::default();
        let ret = parse_file(&alloc, source, &PathBuf::from("comp.tsx"));
        assert!(ret.errors.is_empty(), "unexpected errors: {:?}", ret.errors);
    }
}
