# ast_segmenter — agent notes

## Purpose

Parses a TypeScript/JavaScript source file and extracts a `Segment`: the set of
imports, exports, re-exports, and tagged symbols declared in that file.

## Extract-then-drop arena pattern

OXC allocates its AST inside an `oxc_allocator::Allocator` (bumpalo arena). All
node references and `Atom<'a>` strings borrow from that arena, so the arena
**must outlive every visitor that touches the AST**.

The pipeline follows an **extract-then-drop** sequence:

1. Borrow a thread-local `Allocator` (via `thread_local! { static ARENA }`).
2. Parse the file with `oxc_parser` + run `oxc_semantic::SemanticBuilder`.
3. Run `SegmentVisitor` (and other visitors) which copy every interesting value
   into owned types (`String`, `CompactStr`, `oxc_span::Span`, …).
4. Drop the parser output — **before** returning the `Segment**.
5. Call `Allocator::reset()` to reclaim arena memory cheaply.

**Never store `Atom<'a>`, `&'a str`, or any other arena-borrow in `Segment`,
`RawModuleDeps`, or any type that escapes a single parse call.** The next
`reset()` will dangle those references.

## Owned identifier type

`CompactStr` (from the `compact_str` crate, 24-byte SSO) is used wherever an
identifier name must be stored beyond the arena lifetime.  This replaces the
old `swc_atoms::Atom` (globally interned).

## Spans

`oxc_span::Span { start: u32, end: u32 }` is `Copy`.  It holds byte offsets
relative to the start of the file being parsed (not a global source map).

## Scope analysis

`oxc_semantic::SemanticBuilder` is used for scope analysis (replaces the
deleted `ast_name_tracker` crate).  Access the result via `Semantic::scoping()`.
