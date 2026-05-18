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
leading-comment lookup helper that later phases consume.

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
use oxc_ast::Comment;

/// Return the contiguous slice of comments in `comments` whose `span.end`
/// falls immediately before `statement_start` with no intervening statement.
///
/// Assumes `comments` is sorted by `span.end` (oxc invariant on
/// `Program.comments`). Implementation: `partition_point` to find the first
/// comment whose `span.end > statement_start`, then walk backwards while the
/// comments are contiguous (gap is only whitespace).
pub fn leading_comments_at<'a>(
    comments: &'a [Comment],
    source: &str,
    statement_start: u32,
) -> &'a [Comment];
```

No wrapper struct. Callers pass `&program.comments` directly.

- [ ] Create the crate with `Cargo.toml` depending on `oxc_allocator`, `oxc_ast`, `oxc_parser`, `oxc_span`
- [ ] Implement `parse_file`, `parse_ts`, `parse_tsx` in `lib.rs`
- [ ] Implement `leading_comments_at(comments, source, statement_start)` using `partition_point` + a backward walk that stops once the gap between two adjacent comments contains a non-whitespace byte
- [ ] Add unit test: parse a `.ts` and a `.tsx` fixture; assert `errors.is_empty()`
- [ ] Add unit test: source `"// @ALLOW-UNUSED-EXPORT\nexport const x = 1;"` — parse, find the `Statement::ExportNamedDeclaration` in `program.body`, take its `.span.start`, assert `leading_comments_at(&program.comments, source, span.start)` returns one comment with `text` containing `"@ALLOW-UNUSED-EXPORT"`
- [ ] Add unit test: source with an interior comment between two statements — assert that comment is NOT returned as leading for either neighbor (whichever rule we pick: it leads the second one if there's no blank-line gap, else neither). Document the rule we picked in a doc comment on `leading_comments_at`.

## Verify

- [ ] `cargo check --workspace` succeeds
- [ ] `cargo test -p oxc_utils_parse` passes
