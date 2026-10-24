# ftree_cache

Context-cache values and misses retain original explicit input proofs.
`ContextData::read_context_data_with_inputs` must describe actual consumed data;
the legacy default and prepopulated values are deliberately unproven.
Forward observations through `WithCache` and replay them on hits. Derived-cache
owners retain/replay their own scoped probes. Never reconstruct provenance by
restatting a cached value. Fresh validation bypasses memoization and enumerates
no directories; ordinary inputs seal kind/identity/links, not source contents.
`PathInput::checked_from_parts` is the persisted-proof bridge: it freshly
validates every supplied field/content seal before returning a reconstructed
observation. Never add an unchecked constructor for disk provenance.

Context cells atomically publish values with original proofs; return Arc-backed
handles, never shard guards, and initialize outside DashMap locks. Explicit
InputSnapshot epochs single-flight paths, shared prefixes and exact bytes.
Snapshot checks compare original observations; final freshness requires a new
snapshot. Do not use a live build snapshot as a final validation authority.
PathInput handles share immutable original observation records and link-prefix
slices; content seals remain per-handle scalars. InputProof clones share immutable
observation/error arrays, while request collectors remain separately mutable.
Retained proofs own only their consulted records, never the snapshot/epoch.
Sharing must not recapture, normalize or discard negative/link/content evidence.
