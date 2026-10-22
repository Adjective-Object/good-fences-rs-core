# ftree_cache

Caches filesystem context lookups, including absent contexts, and optional
derived values. Call `mark_dirty_root` when replacing filesystem epochs.
`ContextData` remains usable without an observer.

Context entries retain their original scoped filesystem inputs. Explicit
observers replay those inputs on cache hits; fresh proof validation never uses
the context cache. Only content-dependent reads (such as manifests) seal bytes;
ordinary file probes record kinds, canonical identities and intermediate links,
without directory enumeration.

Custom readers override `read_context_data_with_inputs` to describe their actual
reads; legacy readers and prepopulated values remain usable but cannot
authorize tracked hits. Windows identities use metadata-only directory/file
handles with the repository's pinned nightly toolchain.

Run `cargo test -p ftree_cache` (also supported with `--features deadlock_ref`).
