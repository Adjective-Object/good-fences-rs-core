# ast_segmenter

## Parser stack

Uses the oxc parser (`oxc_parser`) with `oxc_semantic` for scope analysis.

## Extract-then-drop arena pattern

oxc allocates its AST in a bumpalo arena (`oxc_allocator::Allocator`). All
`Atom<'a>`, `&'a Node`, and `oxc_allocator::Vec` types borrow from the arena
and cannot outlive it.

Parse pipeline:

1. Borrow (or create) an `Allocator`.
2. Parse → run `SemanticBuilder` → run visitors that copy owned data out.
3. Drop (or `reset()`) the `Allocator`.

Returned types (`Segment`, `RawModuleDeps`, `VariableScope`) hold only owned
data (`String`, `CompactString`, `oxc_span::Span`). Nothing borrows `'a`.

In threaded contexts, each Rayon worker holds a `thread_local! { Allocator }`
and calls `Allocator::reset()` between files to reuse memory.

## Scope analysis

`oxc_semantic` replaces the old hand-rolled `ast_name_tracker`. `SymbolFlags`
maps to `HoistingLevel` via `hoisting_for_flags()`.
Escaped references retain OXC's type/value distinction; a name is type-only
only if all its references in the segment are type-only. Do not discard runtime
references when merging names.

## Compatibility spans and graph

Statement spans are zero-based, end-exclusive UTF-8 byte offsets. The legacy
`SegmentGraph` treats execution-only imports and opaque requires as dependencies
on all target statements, not only exports; this is not evaluation scheduling.

## Bundler parse policy

`parse_module` uses explicit package format and extension-sensitive language
modes; validate parser and semantic diagnostics before extraction. It returns
owned records or an error, never a skipped/partial executable program. Keep the
legacy unused-tracker adapter separate. TS/JSX parsing is not transformation.
