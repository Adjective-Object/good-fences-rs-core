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
