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

New shape: `WrapFileLogger` holds **only** a `NamedSource<String>`. The
source text is retrieved via `NamedSource::inner()`. Line/col lookup uses a
lazily-computed `OnceCell<Vec<u32>>` of newline byte offsets. Span becomes
`oxc_span::Span`.

```rust
use once_cell::sync::OnceCell;
use oxc_diagnostics::NamedSource;
use oxc_span::Span;

pub trait SrcFileLogger: Logger {
    fn src_warn(&self, location: Span, message: impl Display);
    fn src_error(&self, location: Span, message: impl Display);
}

pub struct WrapFileLogger<TLogger> {
    named_source: NamedSource<String>,
    inner_logger: TLogger,
    line_starts: OnceCell<Vec<u32>>,
}

impl<TLogger> WrapFileLogger<TLogger> {
    pub fn new(filename: impl Into<String>, source: String, inner_logger: TLogger) -> Self { ... }
    fn line_col(&self, byte_offset: u32) -> (usize, usize) {
        let starts = self.line_starts.get_or_init(|| compute_line_starts(self.named_source.inner()));
        // binary_search on starts
    }
}
```

`SimpleSourceFileLogger` (which only has a path, no source text) is preserved
and updated to take `Span` instead of `&Span`.

- [ ] Change `SrcFileLogger` trait to take `Span` (oxc) by value instead of `&swc_common::Span`
- [ ] Replace `WrapFileLogger` fields: drop `source_map: TSrcMap`, add `named_source: NamedSource<String>` and `line_starts: OnceCell<Vec<u32>>` (no separate `source: String` — read via `NamedSource::inner()`)
- [ ] Implement `WrapFileLogger::new(filename, source, inner_logger)` constructing `NamedSource::new(filename, source)` and an empty `OnceCell`
- [ ] Implement `line_col(byte_offset)` that lazily populates `line_starts` via `OnceCell::get_or_init` and `binary_search`es into it
- [ ] Re-implement `src_warn`/`src_error` to emit `"{filename}:{line}:{col} :: {message}"` (preserves existing log format)
- [ ] Update `SimpleSourceFileLogger` to use `Span` (oxc) — no source text, just emits `"{path}:byte={span.start}:: {message}"` (intentional format change for the no-source variant)
- [ ] Remove `swc_common` dependency from `logger_srcfile/Cargo.toml`; add `oxc_span`, `oxc_diagnostics`, `once_cell`

## Update call sites

`WrapFileLogger::new(source_map, logger)` callers exist in:

- `ast_segmenter/src/visitor.rs` (test only)
- `ast_name_tracker/src/visitor.rs` (test only)
- `unused_finder/src/parse/exports_visitor_runner.rs` (production)

These still use the swc parser at this point, so they have a `SourceMap` but no
raw source string. Adapter for the transition: a small `from_swc_source_map`
helper that reads source from the swc `SourceFile` and constructs the new
`WrapFileLogger`. The callers also still hand around `swc_common::Span`
values; a stand-alone helper bridges those into the new `oxc_span::Span`
signature:

```rust
// In logger_srcfile (behind the `swc-compat` feature):
pub fn swc_span_to_oxc(span: swc_common::Span) -> oxc_span::Span {
    use swc_common::source_map::SmallPos;
    oxc_span::Span::new(span.lo.to_u32(), span.hi.to_u32())
}
```

- [ ] Add `WrapFileLogger::from_swc_source_file(sm: &SourceMap, fm: &SourceFile, logger)` adapter behind a `swc-compat` cargo feature
- [ ] Add `swc_span_to_oxc(swc_common::Span) -> oxc_span::Span` helper behind the same `swc-compat` feature
- [ ] Update each call site to construct `WrapFileLogger` via the adapter, and to wrap every `&swc_span` argument with `swc_span_to_oxc(*span)` until the upstream visitor itself is migrated
- [ ] Update test call sites in `ast_segmenter` and `ast_name_tracker` similarly
- [ ] Add unit test: line/col lookup matches expected for multi-line source with `\r\n` and `\n` endings
- [ ] Add unit test: `swc_span_to_oxc(Span::new(BytePos(3), BytePos(7), Default::default()))` round-trips to `Span::new(3, 7)`

## Verify

- [ ] `cargo test -p logger_srcfile` passes
- [ ] `cargo test --workspace` still passes (regressions caught)
