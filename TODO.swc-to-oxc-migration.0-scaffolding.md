High level goal: add OXC workspace dependencies and a new `oxc_utils_parse`
crate so subsequent phases can migrate one crate at a time while the swc stack
keeps working.

## Add OXC workspace dependencies

Pin to git rev `87f065ebf7cde21d1229322f4c7ee58baca5648e`. Add these to
`[workspace.dependencies]` in the root `Cargo.toml`:

- `oxc_allocator`
- `oxc_ast`
- `oxc_ast_visit`
- `oxc_parser`
- `oxc_semantic`
- `oxc_span`
- `oxc_diagnostics`

Do **not** remove any `swc_*` dependency in this phase — both stacks coexist
through phase 7.

- [ ] Add OXC git dependencies to root `Cargo.toml` `[workspace.dependencies]`
- [ ] Verify `cargo metadata` resolves the new graph (run `cargo check --workspace`)

## Create `oxc_utils_parse` crate

Sibling of `swc_utils_parse`. Provides the parse entry point and the
leading-comment adapter that later phases consume.

```
crates/oxc_utils_parse/
  Cargo.toml
  src/
    lib.rs
    leading_comments.rs
```

API surface:

```rust
use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;

pub fn parse_file<'a>(
    allocator: &'a Allocator,
    source: &'a str,
    path: &std::path::Path,
) -> ParserReturn<'a> {
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts());
    Parser::new(allocator, source, source_type).parse()
}

// also re-exported convenience for callers that don't have a Path on hand
pub fn parse_ts<'a>(allocator: &'a Allocator, source: &'a str) -> ParserReturn<'a>;
pub fn parse_tsx<'a>(allocator: &'a Allocator, source: &'a str) -> ParserReturn<'a>;
```

```rust
// leading_comments.rs
use ahashmap::AHashMap;
use oxc_ast::{ast::Program, Comment};

pub struct LeadingComments<'a> {
    source: &'a str,
    by_start: AHashMap<u32, &'a [Comment]>,
}

impl<'a> LeadingComments<'a> {
    pub fn for_program(source: &'a str, program: &'a Program<'a>) -> Self { /* ... */ }
    pub fn at(&self, statement_start: u32) -> &'a [Comment] { /* ... */ }
    pub fn source(&self) -> &'a str { self.source }
}
```

- [ ] Create the crate with `Cargo.toml` depending on `oxc_allocator`, `oxc_ast`, `oxc_parser`, `oxc_span`, `ahashmap`
- [ ] Implement `parse_file`, `parse_ts`, `parse_tsx` in `lib.rs`
- [ ] Implement `LeadingComments::for_program` by walking `program.comments` and `program.body` once, associating each comment range with the next statement's `span.start`
- [ ] Implement `LeadingComments::at(lo)` returning the precomputed slice (empty slice if none)
- [ ] Add unit test: parse a `.ts` and a `.tsx` fixture; assert `errors.is_empty()`
- [ ] Add unit test: source with `// @ALLOW-UNUSED-EXPORT\nexport const x = 1;` — assert `LeadingComments::at(span_of_export_x)` returns the comment

## Verify

- [ ] `cargo check --workspace` succeeds
- [ ] `cargo test -p oxc_utils_parse` passes
