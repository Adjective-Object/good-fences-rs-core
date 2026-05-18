# Notes: swc-to-oxc-migration.02-logger-srcfile

## What changed

`WrapFileLogger` was restructured:
- Dropped `source_map: TSrcMap` type parameter — no longer a generic over `Borrow<SourceMap>`.
- Added `named_source: NamedSource<String>` (from miette via oxc_diagnostics) to hold the filename.
- Added `line_starts: OnceCell<Vec<u32>>` for lazy line-start computation (only populated on first warning/error).
- Constructor is now `WrapFileLogger::new(filename, source: String, inner_logger)`.

`SrcFileLogger::src_warn` / `src_error` now take `Span` (oxc) **by value** instead of `&swc_common::Span`.

`SimpleSourceFileLogger` updated similarly; the format changed from `"{path} :: {message}"` (ignoring span) to `"{path}:byte={span.start}:: {message}"`.

`SrcLogger` and `HasSourceMap` (unused outside the crate) were removed.

## Design decisions

### `once_cell` vs `std::sync::OnceLock`
`OnceCell` from the `once_cell` crate was chosen over `std::sync::OnceLock` (MSRV 1.70) because `once_cell` is already in the transitive dependency closure (21 occurrences in `Cargo.lock`). The workspace `edition = "2018"` also pre-dates `OnceLock`'s stabilisation.

### `NamedSource<String>` — why not `(String, String)`?
The plan explicitly required `NamedSource<String>` so the type carries the filename and source together in an `oxc_diagnostics`-native wrapper for possible future diagnostic rendering.

### swc BytePos offsets in `swc_span_to_oxc`
`swc_common::BytePos` values in a single-file source map are 1-indexed relative to position 0 of the whole map, so they are effectively equal to the raw byte index within that file's source string. The conversion `span.lo.0 / span.hi.0` is therefore correct for single-file maps (tests, visitor). For multi-file maps the offset is still source-map-global, making it slightly off for logging purposes, but this is acceptable during the transition — the upstream visitors will be migrated to oxc spans in later phases. `swc_common::Span::new` in 0.37.4 takes only 2 args (lo, hi) — no `SyntaxContext` third parameter.

### `NamedSource::new` takes `impl AsRef<str>`, not `impl Into<String>`
Discovered at compile time. The constructor signature was adjusted accordingly.

### Call sites: test vs production
- Tests that already had the source `&str` call `WrapFileLogger::new(name, src.to_string(), logger)` directly.
- `exports_visitor_runner.rs` (production) uses `from_swc_source_file(cm, &fm, logger)` because it only has a file path at the construction site — the source lives in the swc `SourceFile`.

## Gotchas

- `#[cfg(feature = "swc-compat")]` must appear on *both* the `mod` and the `pub use` re-export.
- The `swc-compat` test (`swc_span_to_oxc_round_trip`) must also be gated on the feature.
- `#[derive(Clone)]` was required on `WrapFileLogger` because `Logger: Clone` is a supertrait bound and `Logger::error/warn/log` call `self.clone()` internally in the blanket impl.
