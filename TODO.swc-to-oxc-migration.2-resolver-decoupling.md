High level goal: remove `swc_common::FileName` and `swc_ecma_loader::resolve`
from the custom resolver while keeping all resolution behavior, caching, and
the `MonorepoResolver` ouroboros wrapper intact.

Depends on phase 0. Independent of phases 1, 3+.

## Introduce a local `PathResolver` trait

New module `import_resolver/src/resolve.rs`:

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub path: PathBuf,
    pub slug: Option<String>,
}

pub trait PathResolver: Send + Sync {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution>;
}
```

`TargetEnv` and `NODE_BUILTINS` move into the same module (copied from
`swc_ecma_loader`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetEnv { Browser, Node }

pub const NODE_BUILTINS: &[&str] = &[ /* ... */ ];
```

- [ ] Create `import_resolver/src/resolve.rs` with the trait, `Resolution`, `TargetEnv`, `NODE_BUILTINS`
- [ ] Copy `NODE_BUILTINS` list verbatim from the swc source
- [ ] Re-export from `import_resolver/src/lib.rs`

## Rename `swc_resolver` → `node_resolver`

The directory name becomes misleading once swc is gone.

- [ ] `git mv crates/import_resolver/src/swc_resolver crates/import_resolver/src/node_resolver`
- [ ] Update `mod swc_resolver;` → `mod node_resolver;` in `lib.rs`
- [ ] Rename the inner `node_resolver.rs` (the actual `CachingNodeModulesResolver`) to `caching.rs` to avoid the module-name collision
- [ ] Update `use` paths across the workspace (search for `swc_resolver::`)

## Replace `FileName` with `&Path`, `swc Resolve` with local `PathResolver`

Files to edit ([crates/import_resolver/src/node_resolver/](crates/import_resolver/src/node_resolver/)):

- `mod.rs` (`MonorepoResolver`)
- `combined_resolver.rs`
- `caching.rs` (formerly `node_resolver.rs`)
- `internal_resolver.rs`
- `tsconfig_resolver.rs`
- `tsconfig.rs`
- `pkgjson_rewrites.rs`

Pattern:

```rust
// before
use swc_common::FileName;
use swc_ecma_loader::resolve::{Resolution, Resolve};
impl Resolve for X {
    fn resolve(&self, base: &FileName, specifier: &str) -> Result<Resolution, Error> {
        let base_path = match base { FileName::Real(p) => p, _ => bail!("...") };
        // ...
    }
}

// after
use crate::resolve::{PathResolver, Resolution};
impl PathResolver for X {
    fn resolve(&self, base: &Path, specifier: &str) -> anyhow::Result<Resolution> {
        // base used directly
    }
}
```

- [ ] Replace `&FileName` parameter with `&Path` in every `resolve(...)` signature
- [ ] Drop the `FileName::Real(p) => p, _ => bail!(...)` match — base is already a `&Path`
- [ ] Replace `swc_ecma_loader::resolve::Resolution` with `crate::resolve::Resolution` everywhere
- [ ] Replace `impl Resolve for ...` with `impl PathResolver for ...`
- [ ] Replace `swc_common::collections::AHashMap` (used in `pkgjson_rewrites.rs`) with `ahashmap::AHashMap`
- [ ] Replace `swc_ecma_loader::TargetEnv` references with `crate::resolve::TargetEnv`
- [ ] Replace `swc_ecma_loader::NODE_BUILTINS` references with `crate::resolve::NODE_BUILTINS`

## Update callers outside `import_resolver`

Call sites that pass `FileName::Real(...)` or import `swc_ecma_loader::resolve::Resolve`:

- `import_resolver/src/manual_resolver.rs` — top-level helpers `resolve_with_extension`
- `unused_finder/src/unused_finder.rs` (imports `swc_ecma_loader::{resolve::Resolve, TargetEnv}`)
- `unused_finder/src/parse/data.rs`
- Any napi shims under `crates/*_napi/`

- [ ] Change `resolve_with_extension(base: FileName, ...)` to `resolve_with_extension(base: &Path, ...)`
- [ ] Update each caller to pass `&PathBuf` / `Path::new(...)` instead of `FileName::Real(...)`
- [ ] Replace `impl Resolve + Sync` trait bounds with `impl PathResolver`
- [ ] Update `unused_finder/src/parse/data.rs::try_resolve` to take `&impl PathResolver`

## Drop swc deps from import_resolver

- [ ] Remove `swc_common` and `swc_ecma_loader` from `import_resolver/Cargo.toml`
- [ ] Run `cargo shear` (or `cargo-shear`) on the crate to confirm
- [ ] `cargo test -p import_resolver` passes (entire existing test suite must still pass)
- [ ] `cargo test --workspace` passes
