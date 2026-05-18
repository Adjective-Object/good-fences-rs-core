High level goal: drop `swc_common::SourceMap` from `logger_srcfile` and render
file-aware diagnostics with `oxc_diagnostics::NamedSource` instead.

Depends on phase 0.

## Replace SourceMap-backed logger

Current shape ([crates/logger_srcfile/src/lib.rs](crates/logger_srcfile/src/lib.rs)):

```rust
pub trait SrcFileLogger: Logger {
    fn src_warn(&self, location: &Span, message: impl Display);
    fn src_error(&self, location: &Span, message: impl Display);
}

pub struct WrapFileLogger<TSrcMap, TLogger> {
    source_map: TSrcMap, // Borrow<SourceMap>
    inner_logger: TLogger,
}
```

New shape: replace `source_map` with `NamedSource` plus a cached newline index
for fast line/col lookup. Span becomes `oxc_span::Span`.

```rust
use oxc_diagnostics::NamedSource;
use oxc_span::Span;

pub trait SrcFileLogger: Logger {
    fn src_warn(&self, location: Span, message: impl Display);
    fn src_error(&self, location: Span, message: impl Display);
}

pub struct WrapFileLogger<TLogger> {
    named_source: NamedSource<String>,
    inner_logger: TLogger,
    // precomputed newline byte offsets for line/col lookup
    line_starts: Vec<u32>,
}

impl<TLogger> WrapFileLogger<TLogger> {
    pub fn new(filename: impl Into<String>, source: String, inner_logger: TLogger) -> Self { ... }
    fn line_col(&self, byte_offset: u32) -> (usize, usize) { ... }
}
```

`SimpleSourceFileLogger` (which only has a path, no source text) is preserved
and updated to take `Span` instead of `&Span`.

- [ ] Change `SrcFileLogger` trait to take `Span` (oxc) by value instead of `&swc_common::Span`
- [ ] Replace `WrapFileLogger` fields: drop `source_map: TSrcMap`, add `named_source: NamedSource<String>` and `line_starts: Vec<u32>`
- [ ] Implement `WrapFileLogger::new(filename, source, inner_logger)` that precomputes `line_starts`
- [ ] Implement `line_col(byte_offset)` via `binary_search` on `line_starts`
- [ ] Re-implement `src_warn`/`src_error` to emit `"{filename}:{line}:{col} :: {message}"` (preserves existing log format)
- [ ] Update `SimpleSourceFileLogger` to use `Span` (oxc) — no source text, just emits `"{path}:byte={span.start}:: {message}"`
- [ ] Remove `swc_common` dependency from `logger_srcfile/Cargo.toml`; add `oxc_span`, `oxc_diagnostics`

## Update call sites

`WrapFileLogger::new(source_map, logger)` callers exist in:

- `ast_segmenter/src/visitor.rs` (test only)
- `ast_name_tracker/src/visitor.rs` (test only)
- `unused_finder/src/parse/exports_visitor_runner.rs` (production)

These still use the swc parser at this point, so they have a `SourceMap` but no
raw source string. Adapter for the transition: a small `from_swc_source_map`
helper that reads source from the swc `SourceFile` and constructs the new
`WrapFileLogger`.

- [ ] Add `WrapFileLogger::from_swc_source_file(sm: &SourceMap, fm: &SourceFile, logger)` adapter behind a `swc-compat` cargo feature
- [ ] Update each call site to construct `WrapFileLogger` via the adapter
- [ ] Update test call sites in `ast_segmenter` and `ast_name_tracker` similarly
- [ ] Add unit test: line/col lookup matches expected for multi-line source with `\r\n` and `\n` endings

## Verify

- [ ] `cargo test -p logger_srcfile` passes
- [ ] `cargo test --workspace` still passes (regressions caught)
