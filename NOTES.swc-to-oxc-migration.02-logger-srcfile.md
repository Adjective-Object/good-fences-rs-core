# Notes: swc-to-oxc-migration.02-logger-srcfile

## Design decisions

- `SrcLogger` and `HasSourceMap` traits were removed — they had zero external callers.
- `WrapFileLogger` is no longer generic over `TSrcMap`. It holds `NamedSource<String>` directly.
- `Clone` is implemented manually because `NamedSource` doesn't derive `Clone`;
  we reconstruct it from name + source in the clone impl.
- `NamedSource::new` requires `impl AsRef<str>` for the name, not `impl Into<String>`,
  so `WrapFileLogger::new` takes `impl AsRef<str>`.
- `line_col` returns (1-based line, 0-based col) to match the old swc `lookup_char_pos` format.
- `\r\n` handling: we only track `\n` positions. The `\r` before `\n` inflates col by 1
  for the last column of a CRLF-terminated line, but byte offsets from the parser
  never land on `\r` so this is fine in practice.

## swc-compat feature

- `swc_common` is now an optional dep behind `swc-compat`.
- Two adapter constructors: `from_swc_source_map` (takes first file from SourceMap)
  and `from_swc_source_file` (explicit SourceFile). Both extract filename + source text
  and forward to the new `WrapFileLogger::new`.
- `swc_span_to_oxc` is a standalone pub fn in `swc_compat` module.
- All three dependent crates (`ast_segmenter`, `ast_name_tracker`, `unused_finder`)
  enable `swc-compat` and use the adapters.

## Call-site changes

- `src_warn`/`src_error` now take `Span` by value (Copy). Call sites that had `&span`
  now use `swc_compat::swc_span_to_oxc(span)`.
- `SimpleSourceFileLogger` format changed from `"{path} :: {msg}"` to
  `"{path}:byte={start} :: {msg}"` — intentional per the TODO spec.
