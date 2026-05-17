# Segment-Aware Migration — Working Notes

## Phase 1: Make `ast_segmenter` compile

### Issues found and fixes applied

1. **`raw_module_deps.rs`: `TryInto` impl syntax** — Missing `type` keyword on associated type (`Error = ...` → `type Error = ...`). Also needed `use std::convert::TryInto` since crate uses edition 2018.

2. **`raw_module_deps.rs`: `ExportedLocal` undefined** — `RawModuleDeps.exports_locals` referenced `ExportedLocal` which was never defined. Created a type alias `type ExportedLocal = ExportedSymbol` since the existing `ExportedSymbol` enum (Named/Default) already models what an exported local needs.

3. **`raw_module_deps.rs`: `ReExportedSymbol` field mismatch in `TryInto`** — The `TryInto` impl tried to construct `ReExportedSymbol { name, exported_as }` but the struct (in `lib.rs`) has `imported_as: ImportTarget`. Fixed to use correct field names, wrapping the name in `ImportTarget::ExportedSymbol(ExportedSymbol::Named(name))`.

4. **`visitor.rs`: wrong import path `crate::import_require`** — Changed to `crate::visitors::import_require_expr`.

5. **`visitor.rs`: destructuring syntax error in `Stmt` arm** — Line 50 tried `imported_paths: lazy_imports.as_module_imports()` inside a destructuring pattern, which isn't valid Rust. Rewrote to call `find_imports_and_requires` properly and use the result fields.

6. **`visitor.rs`: `stmt` undefined in `ModuleDecl` arm** — The `ModuleDecl` arm referenced `stmt` but the binding is `module_decl`. Fixed reference. Also changed `Ok(names)` to return a proper `Option<Segment>` (was returning `Result`).

7. **`visitor.rs`: `roaring::RoaringBitmap`** — Added `roaring = "0.10"` to Cargo.toml as the commented-out dependency indicated.

8. **`visitor.rs`: `statement_to_segment` doesn't exist** — `segment_module` called `statement_to_segment` which was never defined. Changed to `module_item_to_segment` which is the function defined above in the same file.

9. **`import_export_statement.rs`: `From<ExportsVisitor<T>> for RawImportExportInfo`** — `RawImportExportInfo` doesn't exist in this crate. Changed the impl to `From<ExportsVisitor<T>> for RawModuleDeps`, returning `x.module_deps` directly since `ExportsVisitor` now stores a `RawModuleDeps` field.

10. **`import_export_statement.rs`: visitor methods reference old field names** — The `Visit` impl methods referenced `self.exported_ids`, `self.export_from_ids`, etc. (field names from `unused_finder`'s version). Rewrote to populate `self.module_deps.*` fields. Also added `require_identifiers: AHashSet<swc_ecma_ast::Id>` field to `ExportsVisitor`.

11. **`import_export_statement.rs`: `visit_named_export` calls `.try_as_re_export()`** — `ExportBinding` has no such method. Changed to use `.try_into()` (the `TryInto<ReExportedSymbol>` impl we fixed).

### Design decisions

- **`ExportedLocal = ExportedSymbol`**: The simplest approach. `exports_locals` maps from "what symbol is exported" to its metadata. `ExportedSymbol` already covers Named/Default variants which is exactly what we need.
- **`handle_export_named_specifiers`**: Kept as a method on `ExportsVisitor`, implementing it inline to handle `export { foo, bar }` (no source) by inserting into `module_deps.exports_locals`.
- **`Dependencies2D.add_dependency`**: Fixed — was calling `.contains()` (read) instead of `.insert()` (write).
- **`import_require_expr.rs`: `import()` inserted into wrong map**: Pre-existing bug — bare `import()` was inserting into `require_paths` instead of `imported_paths`. Fixed.
- **`import().then()` double-processing**: The visitor's `visit_children_with` traverses the inner `import()` CallExpr before `scan_call_expr` processes the outer `.then()` call. This caused a spurious `Namespace` entry. Fixed by removing the `Namespace` entry in the `.then()` arm after extracting named symbols.
- **`find_imports_and_requires` unused type parameter**: Removed unused `TLogger` generic from the function signature — it was never referenced in the body.
- **`ExportedSymbol::from(ident.sym)` move errors**: swc's `Atom` doesn't impl `Copy`. Switched to `.as_ref()` to borrow instead of move.
- **`get_export_bindings` lifetime issues**: Changed from `impl Comments` param + returning `impl Iterator` (lifetime conflict) to `&SingleThreadedComments` param + returning `Vec<ExportBinding>`.
- **`RawModuleDeps` fields**: Made all fields `pub` so `ExportsVisitor` (in a child module) can populate them directly.
- **`ImportTarget`, `ReExportedSymbol`**: Added `Debug, PartialEq, Eq, Hash, Clone` derives since these types are stored in `AHashSet`.

## Phase 2: Complete the segment orchestrator

### Design decisions

- **`ExportsVisitor` changed to borrow logger and comments**: Changed `ExportsVisitor<TLogger>` to `ExportsVisitor<'a, TLogger>` storing `&'a TLogger` and `&'a SingleThreadedComments` instead of owned values. This avoids cloning the logger/comments per segment and lets `segment_file` pass references through. `SingleThreadedComments` is Rc-backed so cloning was cheap, but borrowing is cleaner.
- **`Stmt` → `RawModuleDeps` mapping**: For statement segments, `find_imports_and_requires` returns `NameSet<String, Symbol>` for imported_paths and require_paths. These map directly onto `RawModuleDeps.dynamic_imports` (`AHashMap<String, AHashSet<Symbol>>`) and `RawModuleDeps.requires` (`AHashSet<String>`). Static imports/exports fields are left at default (empty) since `Stmt` nodes don't contain static import/export syntax.
- **`ModuleDecl` → `RawModuleDeps` mapping**: Reuses `ExportsVisitor` (the same visitor that handles `import`, `export`, `require` declarations) by visiting each `ModuleDecl` individually. This gives per-segment granularity.
- **`segment_file` as public API**: Returns `Vec<RawSegment>` (one per `ModuleItem`), filtering out unsupported constructs (with, return, break, continue) which emit diagnostics.
- **Tests are assertion-based, not insta-snapshots**: The repo uses `pretty_assertions` + manual assertions. No `insta` dependency existed, so tests follow the existing pattern: parse source → assert segment count and specific field contents.

### Gotchas

- `ExportsVisitor` fields `logger` and `comments` needed all `&self.comments` → `self.comments` fixups since the extra `&` would double-reference.
- `ast_name_tracker::find_names` takes `&TLogger` — with `self.logger: &TLogger`, must pass `self.logger` (not `&self.logger`) to avoid `&&TLogger`.
- The old `Segment` / `SegmentKind` types in `lib.rs` are now dead code — left in place as they're pre-existing and may be useful for future phases.

## Phase 3: Build the segment graph

### Design decisions

- **`SegmentGraph` as a flat-indexed graph**: Nodes are stored in a flat `Vec<SegmentNode>` with `SegmentId → index` lookup via `AHashMap`. Edges stored as `Vec<AHashSet<usize>>` (adjacency list). This mirrors the existing `Graph` in `unused_finder` but at segment granularity.
- **`TagSet` as u32 bitflags**: Simple bitflag propagation (REACHABLE, EFFECTFUL, etc.). Allows multiple independent tags to be propagated in separate BFS passes without interfering. Chosen over an enum because tags are composable.
- **Hoisting heuristic**: `segment_is_hoisted` returns true for segments with static imports, re-exports (`exports_from`), or side-effect imports (`executed_paths`). These correspond to JS/TS hoisted declarations. `exports_locals` alone (e.g. `export const x = 42`) is NOT hoisted — only `import` and `export ... from` are.
- **Intra-file effect edges**: Non-hoisted segments form a chain: each depends on the previous non-hoisted segment in file order. Hoisted segments are skipped entirely — they have no execution-order dependency on prior statements.
- **Inter-file name edges via `add_inter_file_edges`**: Decoupled from `build()` because import specifier resolution (path → file_id) is the caller's responsibility. This keeps the graph construction pure and testable without a filesystem.
- **Re-exports in file_export_map**: Both `exports_locals` and `exports_from` entries are registered in the per-file export map. For re-exports, the `exported_as` name (or original name if no rename) is used as the key. Wildcard re-exports (`export *`) are skipped since they can't be indexed by a single name.
- **BFS propagation direction**: Tags propagate forward along dependency edges — if A is seeded and A depends on B, then B also receives the tag. This matches the intuition that "if A is reachable and A imports B, then B is also reachable."

### Gotchas

- **Diamond test required re-export segments**: Initial test used separate export+import segments per file, but inter-file edges only resolve against the file_export_map. Two segments in the same file don't automatically have edges unless connected by effect ordering or shared names. Re-export segments (`exports_from`) are the correct way to model pass-through files.
- **`exports_locals` segments are NOT hoisted**: `export const x = 42` creates a segment with `exports_locals` but no static imports. This is intentionally non-hoisted because `const` declarations aren't hoisted in JS. Only `export function` would be hoisted, but we can't distinguish that from `exports_locals` alone without AST info — noted as a future refinement.
- **Wildcard re-exports**: `export * from '...'` can't be represented in the export map keyed by `ExportedSymbol`. Currently skipped during map construction. A future phase may need to resolve wildcards by expanding them against the source file's exports.

## Phase 4: Wire `ast_segmenter` into `unused_finder`

### Design decisions

- **`unused_finder::ExportedSymbol` kept as separate type**: `ast_segmenter::ExportedSymbol` has only Named/Default variants, while `unused_finder` needs Namespace and ExecutionOnly for graph traversal. Added `From` impls to convert between the two type systems.
- **`RawImportExportInfo` built by flattening `Vec<RawSegment>`**: Implemented `From<&[RawSegment]> for RawImportExportInfo` which merges all segment `RawModuleDeps` into a single flat structure. This preserves backward compatibility — all existing code continues using `RawImportExportInfo` and `ResolvedImportExportInfo` unchanged.
- **`get_file_import_export_info` preserved as convenience wrapper**: Now delegates to `get_file_segments` → `segment_file` → flatten. The original `ExportsVisitor` in `unused_finder` is kept but no longer called from the main pipeline (still used by tests).
- **`get_file_segments` added as new public API**: Returns `Vec<RawSegment>` directly, used by `walk.rs` to produce both segments and flattened `RawImportExportInfo` in a single parse pass.
- **Segments flow through the pipeline**: `WalkedSourceFile`, `ResolvedSourceFile`, and `GraphFile` all carry `Vec<RawSegment>` alongside the existing `import_export_info`. This enables future segment-level analysis without breaking file-level analysis.
- **`GraphFile::symbol_to_segment` index**: Built during `new_from_source_file` by iterating each segment's `exports_locals` and mapping exported symbol → segment index. Enables looking up which segment owns a given export.

### Span and tag propagation

- **`TaggedSymbol.span` added**: `ast_segmenter::TaggedSymbol` now carries a `swc_common::Span` so export locations survive the type conversion. `with_span()` constructor used where spans are available; `new()` defaults to `Span::default()`.
- **`ReExportedSymbol.tags` + `span` added**: Re-exports now carry `SymbolTags` (is_type_only, allow_unused) and `Span` from their source AST node. This is critical for type-only re-export propagation (`export type { X } from '...'`).
- **`ReExportedSymbol` custom Hash/Eq**: Identity is `imported_as + exported_as` only — `tags` and `span` are metadata excluded from equality/hashing. This prevents duplicate entries in `AHashSet<ReExportedSymbol>` when the same re-export has different metadata.

### Derives added to support Clone/Debug/Eq through the pipeline

- `ast_name_tracker::VarID`: added `Debug, PartialEq, Eq`
- `ast_name_tracker::HoistingLevel`: added `PartialEq, Eq`
- `ast_name_tracker::VariableScope`: added `Debug, Clone, PartialEq, Eq`
- `ast_segmenter::RawSegment`: added `Debug, Clone, PartialEq, Eq`

### Gotchas

- **Test `test_indirect_typeonly_export` required re-export span propagation**: Without spans on `ReExportedSymbol`, the test validation code attempted `(0 - 1)` on a u32 causing overflow. The fix was to carry spans all the way through.
- **`ast_segmenter::Symbol` → `ExportedSymbol` conversion needed**: The `Symbol::Namespace` variant maps to `ExportedSymbol::Namespace`. Added `From<&Symbol>` impl.
- **Re-export `is_type_only` needed propagation**: `export type { X } from '...'` must propagate `is_type_only` through to `ExportedSymbolMetadata` for the BFS type-only tracking to work correctly.
