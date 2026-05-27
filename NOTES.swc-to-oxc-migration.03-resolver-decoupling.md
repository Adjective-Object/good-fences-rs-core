# Phase 03 – Resolver Decoupling: Working Notes

## Section 1: Introduce local `PathResolver` trait

- Created `import_resolver/src/resolve.rs` with `Resolution`, `PathResolver` trait, `TargetEnv`, `NODE_BUILTINS`.
- Used `String` for `Resolution.slug` instead of `oxc_span::CompactStr` — `CompactStr` is not
  re-exported from `oxc_span` at the pinned rev, and `oxc_str` is not a workspace dep. `String`
  is the fallback per PLAN.md. Can be revisited if `oxc_str` is added later.
- Added blanket impls for `&R`, `Box<R>`, `Arc<R>` to match swc's `Resolve` ergonomics.
- `TargetEnv` derives `Serialize`/`Deserialize` matching the swc version (serde already a dep).
- `NODE_BUILTINS` copied verbatim from `swc_ecma_loader-0.49.1`.
