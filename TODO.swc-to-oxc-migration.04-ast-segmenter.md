High level goal: port `ast_segmenter` from swc AST to oxc AST. Drop the
`box_patterns` nightly feature. Use the leading-comment adapter from phase 0.

Depends on phase 0. Phase 1 may or may not be done — `ast_segmenter` uses
`logger_srcfile` and can use whichever shape it currently has.

## Port `Segment` and the data types

[crates/ast_segmenter/src/segment_info.rs](crates/ast_segmenter/src/segment_info.rs)
holds `Segment { span: swc_common::Span, ... }`. Switch to `oxc_span::Span`.

[crates/ast_segmenter/src/raw_module_deps.rs](crates/ast_segmenter/src/raw_module_deps.rs)
holds:

- `TaggedSymbol.span: swc_common::Span`
- `ReExportedSymbol.span: swc_common::Span`
- `SymbolTags::from_comments(comments: impl Comments, lo: BytePos)`
- `impl From<&swc_atoms::Atom> for Name` (used by callers that hand off swc atoms)
- `Symbol::from_module_export_name(&ModuleExportName)`

- [ ] Replace `swc_common::Span` with `oxc_span::Span` in `Segment`, `TaggedSymbol`, `ReExportedSymbol`, `ExportBinding`
- [ ] Replace `BytePos` with `u32` in `SymbolTags::from_comments(_parent)` signatures
- [ ] Replace `impl Comments` parameter with `&LeadingComments<'_>`
- [ ] Replace `impl From<&swc_atoms::Atom> for Name` with `impl From<&oxc_span::Atom<'_>> for Name`
- [ ] Update `Symbol::from_module_export_name` to match on `oxc_ast::ast::ModuleExportName` (handle the third variant `IdentifierReference`)
- [ ] Update `ExportedSymbol::from_module_export_name` similarly

## Port `import_export_statement.rs` visitor

[crates/ast_segmenter/src/visitors/import_export_statement.rs](crates/ast_segmenter/src/visitors/import_export_statement.rs)

Rewrite against `oxc_ast_visit::Visit<'a>`. Node-name translation (validated
in phase 0.5; correct any rows that the spike disproves before starting this
phase):

| swc                          | oxc                              |
| ---------------------------- | -------------------------------- |
| `ImportDecl`                 | `ImportDeclaration`              |
| `ExportDecl`                 | `ExportNamedDeclaration` with `source: None` and `declaration: Some(_)` |
| `NamedExport`                | `ExportNamedDeclaration` with `source: Some(_)` |
| `ExportAll`                  | `ExportAllDeclaration` (covers both `export * from 'mod'` and `export * as ns from 'mod'`; the `as ns` form sets `exported: Some(ModuleExportName)`) |
| `ExportDefaultDecl`          | `ExportDefaultDeclaration` with `declaration: ExportDefaultDeclarationKind::{FunctionDeclaration, ClassDeclaration, TSInterfaceDeclaration}` |
| `ExportDefaultExpr`          | `ExportDefaultDeclaration` with `declaration: ExportDefaultDeclarationKind::Expression(_)` (oxc unifies the two swc nodes) |
| `ImportSpecifier::{Named,Default,Namespace}` | `ImportDeclarationSpecifier::{ImportSpecifier, ImportDefaultSpecifier, ImportNamespaceSpecifier}` |
| `ExportSpecifier::Named`     | `ExportSpecifier` (inside `ExportNamedDeclaration.specifiers`) |
| `ExportSpecifier::Namespace` | **no separate variant** — represented as `ExportAllDeclaration { exported: Some(ModuleExportName), .. }` |
| `ExportSpecifier::Default`   | **not parsed** at the pinned oxc rev (stage-1 `export v from 'mod'` proposal). Phase 0.5 confirms no fixture in the corpus uses this; if any do, document them as a known regression. |
| `CallExpr` / `Callee::Import` | `CallExpression` whose `callee` is `Expression::ImportExpression(_)` |
| `CallExpr` / `Callee::Expr(Ident)` | `CallExpression` whose `callee` is `Expression::Identifier(IdentifierReference)` |
| `Lit::Str(s)` → `s.value`    | `Expression::StringLiteral(s)` → `s.value` |
| `TsImportEqualsDecl`         | `TSImportEqualsDeclaration`      |
| `BindingIdent { sym, id }`   | `BindingIdentifier { name: Atom<'a> }` (no SyntaxContext) |
| `Ident { sym }`              | `IdentifierReference { name: Atom<'a>, reference_id: Cell<Option<ReferenceId>> }` |
| `IdentName { sym }`          | `IdentifierName { name: Atom<'a>}` |

The "is `require` a user binding?" check is rewritten using `oxc_semantic`.
Given the call expression `require("foo")`, the callee is
`Expression::Identifier(ident)` and `ident.reference_id` has been populated
by `SemanticBuilder`. The recipe:

```rust
fn is_global_require(semantic: &Semantic<'_>, ident: &IdentifierReference<'_>) -> bool {
    if ident.name != "require" { return false; }
    let Some(ref_id) = ident.reference_id.get() else { return false; };
    semantic.scoping().get_reference(ref_id).symbol_id().is_none()
}
```

No `require_identifiers: AHashSet<Id>` tracking is needed because
`SemanticBuilder` already resolved every `IdentifierReference` against its
declaring scope; a `symbol_id.is_none()` result means "unresolved → falls
through to the global object."

- [ ] Add `&Semantic<'a>` parameter to `ExportsVisitor::new`
- [ ] Add `&'a [Comment]` parameter to `ExportsVisitor::new` (replaces `&SingleThreadedComments`)
- [ ] Drop the `require_identifiers: AHashSet<Id>` field and `visit_binding_ident` override
- [ ] Add a private `is_global_require(&self, ident: &IdentifierReference) -> bool` helper that implements the recipe above
- [ ] In `visit_call_expression`, when the callee is an `IdentifierReference`, call `is_global_require` to decide whether to treat the call as `require("...")`
- [ ] Port `visit_import_decl` → `visit_import_declaration`
- [ ] Port `visit_named_export` / `visit_export_decl` / `visit_export_default_decl` / `visit_export_default_expr` / `visit_export_all`
- [ ] Port `visit_ts_import_equals_decl` → `visit_ts_import_equals_declaration`
- [ ] Update `get_export_bindings` to handle the new `ModuleExportName::IdentifierReference` variant
- [ ] `SymbolTags::from_comments(comments, lo)` calls become `SymbolTags::from_comments(&program.comments, lo)` (slice, not wrapper)
- [ ] All `.sym.to_string()` calls become `.name.to_string()` (oxc uses `name` for `Atom`)
- [ ] All `.value.to_string()` on `Str` become `.value.to_string()` on `StringLiteral` (unchanged)

## Port `import_require_expr.rs` visitor

[crates/ast_segmenter/src/visitors/import_require_expr.rs](crates/ast_segmenter/src/visitors/import_require_expr.rs)
uses `box_patterns` to destructure `Box<Expr>`. OXC uses `&'a Expression`.

The entry point currently takes `&Stmt` / `&ModuleDecl`. In oxc, the
equivalent is to walk a single `Statement` using the free function
`oxc_ast_visit::walk::walk_statement(&mut visitor, stmt)`. The signature
becomes:

```rust
pub fn find_imports_and_requires<'a>(
    semantic: &Semantic<'a>,
    stmt: &Statement<'a>,
) -> ImportsAndRequires;
```

No `WalkWith` trait bound. Top-level callers in `visitor.rs` already match on
`Statement` variants, so they have a `&Statement` to hand off directly.

- [ ] Rewrite the `scan_call_expr` match using `match &expr.callee { Expression::ImportExpression(_) => ..., Expression::Identifier(id) if is_global_require(semantic, id) => ..., Expression::StaticMemberExpression(m) => ... }`
- [ ] Rewrite the `.then(({ a, b, c }) => { ... })` pattern walker against oxc's `ArrowFunctionExpression` / `Function` and `BindingPattern`
- [ ] Drop the `box_patterns` destructures — `Box<Expression>` is `&Expression` via `Deref`
- [ ] Update `extract_generic_function_def_first_arg` to take `&'a Expression<'a>`
- [ ] Update `args_as_import` to take `&'a oxc_allocator::Vec<'a, Argument<'a>>`
- [ ] Replace `find_imports_and_requires<TNode>` with the concrete `fn find_imports_and_requires<'a>(semantic: &Semantic<'a>, stmt: &Statement<'a>) -> ImportsAndRequires` signature above; implementation calls `oxc_ast_visit::walk::walk_statement(&mut visitor, stmt)`

## Update `visitor.rs` (the top-level segmenter)

[crates/ast_segmenter/src/visitor.rs](crates/ast_segmenter/src/visitor.rs)

New signature:

```rust
pub fn segment_file<'a>(
    logger: &impl SrcFileLogger,
    program: &Program<'a>,
    semantic: &Semantic<'a>,
) -> Vec<Segment>;
```

`program.comments` is read directly from the `Program` (no wrapper).

- [ ] Change `module: &swc_ecma_ast::Module` parameter to `program: &Program<'a>`
- [ ] Add `&Semantic<'a>` parameter
- [ ] Replace match on `ModuleItem::{Stmt, ModuleDecl}` with oxc's `Statement::*` variants (oxc flattens these into `Statement`; module-decl statements are `Statement::ImportDeclaration`, `Statement::ExportNamedDeclaration`, etc.)
- [ ] Replace `module_item.span()` calls (from `Spanned` trait) with direct `.span` field access
- [ ] Replace `ast_name_tracker::visitor::find_names(...)` calls with a temporary shim that asks `semantic` for the bindings — full deletion happens in phase 4

## Update in-crate tests

The test helper in `visitor.rs` currently parses with swc:

```rust
fn segment(src: &str) -> Vec<Segment> {
    let cm = swc_common::sync::Lrc::<swc_common::SourceMap>::default();
    let comments = SingleThreadedComments::default();
    // ...
}
```

Rewrite to:

```rust
fn segment(src: &str) -> Vec<Segment> {
    let allocator = oxc_allocator::Allocator::default();
    let ret = oxc_utils_parse::parse_file(&allocator, src, std::path::Path::new("test.ts"));
    let semantic = oxc_semantic::SemanticBuilder::new().build(&ret.program).semantic;
    let logger = logger::StdioLogger::new();
    let file_logger = logger_srcfile::WrapFileLogger::new("test.ts", src.to_owned(), &logger);
    ast_segmenter::segment_file(&file_logger, &ret.program, &semantic)
}
```

- [ ] Rewrite the in-crate `segment(...)` helper
- [ ] Rewrite the `find_imports_and_requires` tests to parse with oxc
- [ ] All existing assertions (segment counts, contents of `module_deps`, `exports_locals`, etc.) must pass unchanged

## Drop swc and nightly

- [ ] Remove `#![feature(box_patterns)]` from `ast_segmenter/src/lib.rs`
- [ ] Remove `swc_common`, `swc_ecma_ast`, `swc_ecma_visit`, `swc_ecma_parser`, `swc_atoms`, `swc_utils_parse` from `ast_segmenter/Cargo.toml`
- [ ] Add `oxc_ast`, `oxc_ast_visit`, `oxc_span`, `oxc_semantic`, `oxc_utils_parse` to `ast_segmenter/Cargo.toml`
- [ ] `cargo test -p ast_segmenter` passes
- [ ] `cargo build --workspace` passes (downstream callers still use swc-parsed AST via the runner — that's phase 5)
