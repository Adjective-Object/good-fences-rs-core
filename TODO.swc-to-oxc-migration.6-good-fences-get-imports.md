High level goal: port `good_fences::get_imports` to the oxc parser + visitor
stack, preserving its existing `FileImports = HashMap<String,
Option<HashSet<String>>>` return shape and the `walk_dirs.rs` call site.

Depends on phases 0, 5. (Independent of phase 7.)

## Port `get_imports_map_from_file`

[crates/good_fences/src/get_imports/mod.rs](crates/good_fences/src/get_imports/mod.rs)

Same pattern as `unused_finder::get_file_segments`:

```rust
pub fn get_imports_map_from_file<P: AsRef<str>>(file_path: &P) -> Result<FileImports, GetImportError> {
    let path = Path::new(file_path.as_ref());
    let source = std::fs::read_to_string(path).map_err(|e| GetImportError::FileDoesNotExist {
        filepath: path.display().to_string(),
        io_errors: vec![e],
    })?;

    let allocator = Allocator::default();
    let ret = oxc_utils_parse::parse_file(&allocator, &source, path);
    if !ret.errors.is_empty() {
        return Err(GetImportError::ParseTsFileError {
            filepath: path.display().to_string(),
            parser_errors: ret.errors.iter().map(|e| e.message.to_string()).collect(),
        });
    }

    let semantic = SemanticBuilder::new().build(&ret.program).semantic;
    let mut visitor = ImportPathVisitor::new(&semantic);
    visitor.visit_program(&ret.program);

    Ok(get_imports_map_from_visitor(visitor))
}
```

- [ ] Rewrite `get_imports_map_from_file` against `oxc_utils_parse` + `oxc_semantic`
- [ ] Delete the swc-specific `create_lexer` helper (was duplicated from `swc_utils_parse`)
- [ ] Update tests at the bottom of `mod.rs` (they hit real files — should still pass unchanged)

## Port `ImportPathVisitor`

[crates/good_fences/src/get_imports/import_path_visitor.rs](crates/good_fences/src/get_imports/import_path_visitor.rs)

Same node-name translations as phase 3. The only semantic dependency is
distinguishing the user-bound `require` from the global.

- [ ] Add `semantic: &'s Semantic<'a>` field to `ImportPathVisitor`
- [ ] Replace `swc_ecma_visit::{Visit, VisitWith}` with `oxc_ast_visit::Visit<'a>`
- [ ] Port `visit_named_export` → `visit_export_named_declaration`
- [ ] Port `visit_binding_ident` — replace the `require_identifiers: HashSet<Id>` tracking with a semantic lookup in `visit_call_expression`
- [ ] Port `visit_ts_import_equals_decl` → `visit_ts_import_equals_declaration`
- [ ] Port `visit_call_expr` → `visit_call_expression`
- [ ] Port `visit_import_decl` → `visit_import_declaration`
- [ ] Update `append_imported_names` and `extract_argument_value` to take oxc types
- [ ] Handle the third `ModuleExportName::IdentifierReference` variant
- [ ] Drop the `require_identifiers: HashSet<Id>` field
- [ ] Rewrite the in-crate tests to parse with `oxc_utils_parse::parse_file`

## Cargo.toml cleanup

- [ ] Remove `swc_common`, `swc_ecma_ast`, `swc_ecma_parser`, `swc_ecma_visit`, `swc_ecma_transforms`, `swc_utils_parse` from `good_fences/Cargo.toml`
- [ ] Add `oxc_allocator`, `oxc_ast`, `oxc_ast_visit`, `oxc_semantic`, `oxc_span`, `oxc_utils_parse`
- [ ] `cargo test -p good_fences` passes
- [ ] `cargo test -p good_fences_napi` passes
