High level goal: port `good_fences::get_imports` to the oxc parser + visitor
stack, preserving its existing `FileImports = HashMap<String,
Option<HashSet<String>>>` return shape and the `walk_dirs.rs` call site.

Depends on phases 0, 5. (Independent of phase 7.)

## Port `get_imports_map_from_file`

[crates/good_fences/src/get_imports/mod.rs](crates/good_fences/src/get_imports/mod.rs)

Same pattern as `unused_finder::get_file_segments`, including the
per-worker thread-local arena. `good_fences_runner.rs` runs
`get_imports_map_from_file` from a `par_iter`, so the arena reuse is
important for throughput.

```rust
use std::cell::RefCell;
use oxc_allocator::Allocator;

thread_local! {
    static PARSE_ARENA: RefCell<Allocator> = RefCell::new(Allocator::default());
}

pub fn get_imports_map_from_file<P: AsRef<str>>(file_path: &P) -> Result<FileImports, GetImportError> {
    let path = Path::new(file_path.as_ref());
    let source = std::fs::read_to_string(path).map_err(|e| GetImportError::FileDoesNotExist {
        filepath: path.display().to_string(),
        io_errors: vec![e],
    })?;

    PARSE_ARENA.with(|arena_cell| {
        let arena = arena_cell.borrow();
        let ret = oxc_utils_parse::parse_file(&arena, &source, path);
        if !ret.errors.is_empty() {
            let parser_errors = ret.errors.iter().map(|e| e.message.to_string()).collect();
            drop(arena);
            arena_cell.borrow_mut().reset();
            return Err(GetImportError::ParseTsFileError {
                filepath: path.display().to_string(),
                parser_errors,
            });
        }

        let semantic = SemanticBuilder::new().build(&ret.program).semantic;
        let mut visitor = ImportPathVisitor::new(&semantic);
        visitor.visit_program(&ret.program);
        let imports = get_imports_map_from_visitor(visitor);

        drop(semantic);
        drop(ret);
        drop(arena);
        arena_cell.borrow_mut().reset();

        Ok(imports)
    })
}
```

- [x] Rewrite `get_imports_map_from_file` against the thread-local arena pattern above
- [x] Delete the swc-specific `create_lexer` helper (was duplicated from `swc_utils_parse`)
- [x] Update tests at the bottom of `mod.rs` (they hit real files — should still pass unchanged)

## Port `ImportPathVisitor`

[crates/good_fences/src/get_imports/import_path_visitor.rs](crates/good_fences/src/get_imports/import_path_visitor.rs)

Same node-name translations as phase 3. The only semantic dependency is
distinguishing the user-bound `require` from the global, using the same
`is_global_require` recipe spelled out in phase 3:

```rust
fn is_global_require(semantic: &Semantic<'_>, ident: &IdentifierReference<'_>) -> bool {
    if ident.name != "require" { return false; }
    let Some(ref_id) = ident.reference_id.get() else { return false; };
    semantic.scoping().get_reference(ref_id).symbol_id().is_none()
}
```

- [x] Add `semantic: &'s Semantic<'a>` field to `ImportPathVisitor`
- [x] Replace `swc_ecma_visit::{Visit, VisitWith}` with `oxc_ast_visit::Visit<'a>`
- [x] Port `visit_named_export` → `visit_export_named_declaration`
- [x] Replace `visit_binding_ident` + `require_identifiers: HashSet<Id>` with the `is_global_require` helper, called from `visit_call_expression`
- [x] Port `visit_ts_import_equals_decl` → `visit_ts_import_equals_declaration`
- [x] Port `visit_call_expr` → `visit_call_expression`
- [x] Port `visit_import_decl` → `visit_import_declaration`
- [x] Update `append_imported_names` and `extract_argument_value` to take oxc types
- [x] Handle the third `ModuleExportName::IdentifierReference` variant
- [x] Drop the `require_identifiers: HashSet<Id>` field
- [x] Rewrite the in-crate tests to parse with `oxc_utils_parse::parse_file`

## Cargo.toml cleanup

- [x] Remove `swc_common`, `swc_ecma_ast`, `swc_ecma_parser`, `swc_ecma_visit`, `swc_ecma_transforms`, `swc_utils_parse` from `good_fences/Cargo.toml`
- [x] Add `oxc_allocator`, `oxc_ast`, `oxc_ast_visit`, `oxc_semantic`, `oxc_span`, `oxc_utils_parse`
- [x] `cargo test -p good_fences` passes
- [x] `cargo test -p good_fences_napi` passes
