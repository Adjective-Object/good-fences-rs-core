# ftree_cache

Context-cache values and misses retain original explicit input proofs.
`ContextData::read_context_data_with_inputs` must describe actual consumed data;
the legacy default and prepopulated values are deliberately unproven.
Forward observations through `WithCache` and replay them on hits. Derived-cache
owners retain/replay their own scoped probes. Never reconstruct provenance by
restatting a cached value. Fresh validation bypasses memoization and enumerates
no directories; ordinary inputs seal kind/identity/links, not source contents.
