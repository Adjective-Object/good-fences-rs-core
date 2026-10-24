# ftree_cache

Caches filesystem context lookups, including absent contexts, and optional
derived values. Call `mark_dirty_root` when replacing filesystem epochs.
`ContextData` remains usable without an observer.

Context entries retain their original scoped filesystem inputs. Explicit
observers replay those inputs on cache hits; fresh proof validation never uses
the context cache. Only content-dependent reads (such as manifests) seal bytes;
ordinary file probes record kinds, canonical identities and intermediate links,
without directory enumeration.

Persisted consumers can use `PathInput::checked_from_parts` to reconstruct one
observation only after fresh filesystem validation, including supplied identity,
links and exact content seals. It does not manufacture an unchecked proof from
arbitrary disk fields. Consumers retain their borrowed wire owner, not a decoded
tree of reconstructed observations.

Custom readers override `read_context_data_with_inputs` to describe their actual
reads; legacy readers and prepopulated values remain usable but cannot
authorize tracked hits. Windows identities use metadata-only directory/file
handles with the repository's pinned nightly toolchain.

Run `cargo test -p ftree_cache` (also supported with `--features deadlock_ref`).
Observation/revalidation errors include the operation and input path while
retaining underlying IO causes.

`InputSnapshot` defines one explicit filesystem epoch. Snapshot-aware observers
share single-flight path, link-prefix and exact-content reads; hits replay the
original observations without filesystem access. Start a new snapshot for fresh
boundary checks (`InputProof::validate_in`), never reuse the live build snapshot
to certify final freshness. `checked_from_parts_in` validates borrowed disk
observations against that epoch without unpacking a proof tree.
After filesystem changes, invalidate or replace both the snapshot and the
context cache together.

`PathInput` clones share their immutable original observation record and
link-prefix slice rather than duplicating native paths and link buffers.
Content seals remain independent scalar values on each handle. Immutable
`InputProof` arrays are shared across clones; mutable request collectors
still snapshot/replay observations in their original order. Retained proofs
keep only consulted records alive, not the filesystem snapshot, cached file
contents or unrelated speculative observations. All owners remain reclaimable
through ordinary reference-counted cleanup.

Context values and proofs are published together through per-key cells.
Returned handles own an `Arc`, not a DashMap shard lock; initializers run outside
shard locks, and unrelated contexts can initialize concurrently. Invalidation
removes cells without invalidating outstanding handles.
