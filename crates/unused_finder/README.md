# unused_finder

Finds unused JavaScript/TypeScript files, exports, and statement segments.
Configuration selects search roots, entry packages, tests, ignored files/symbols,
and whether unused types are allowed. The library resolves dependencies,
propagates usage through `source_graph`/`tag_graph`, and produces reports.

Type-only references do not propagate entry, test, or ignored usage to their
dependencies. Type exports receive `TypeOnly` metadata independently. An
explicit entry root can therefore have both `Entry` and `TypeOnly`, while a
non-entry interface used only as a generic constraint retains just `TypeOnly`.

Symbol and segment report ranges are zero-based UTF-8 byte offsets with an
exclusive end: slice the original source bytes at `[start, end)`. Segment
dependencies use the resolver's raw-specifier-to-file mapping, including relative
imports, dynamic imports, re-exports, side-effect imports, and `require`.
Side-effect imports and opaque requires retain all target statements, even when
the target has no exports. This conservative report graph is not bundler effect
analysis or an execution scheduler.

Run `cargo test -p unused_finder` for configuration, parsing, and report tests.
