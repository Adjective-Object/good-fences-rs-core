# Follow-up Notes

## delete dead code

### `import_require_expr.rs` debug `println!` removal

Removed `println!("arg prop: {:?}", prop);` at line 91 of
`crates/ast_segmenter/src/visitors/import_require_expr.rs`.

This was a leftover debug print inside a `filter_map` closure that processes
object destructuring patterns from `require()` calls. No other stray `println!`
calls remain in the `ast_segmenter` crate. All spot checks pass.
