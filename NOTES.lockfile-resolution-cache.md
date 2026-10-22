# Lockfile-scoped resolution cache replacement

## Goal and constraints

Replace whole-inventory resolution fingerprints with lockfile-derived,
package-local persistent resolution caches. Normal literal-import builds must
not enumerate unrelated installed packages or hash arbitrary JSON files.
Preserve current Node/browser resolution, package exports, aliases, conditions,
import/require differences, workspace links, source maps, CSS/Sass and finite
contexts. Keep extraction sidecars independent.

Use the published MIT-licensed `aube-lockfile = "=2.8.1"`. The packaged crate's
`.cargo_vcs_info.json` records source SHA
`963d9b8202ced528d5eb3f72254b896ecf9d10f9`, distinct from the upstream tag
reference `a022c6790282f51c1dc84e0309192db480b6df43`. Cargo compatibility
has been checked. Its normalized graph is input, not authority to change
installed-tree resolution. Client-web uses Yarn Berry metadata version 10;
exercise its actual lockfile before relying on parser compatibility.

Immutable locked package identity includes source/integrity, peer and patch
context. Mutable workspace/package metadata, local lookup candidates, link
targets, and hoisted/undeclared fallbacks require targeted validation. Include
dependency names and package identity, not a bag of dependency hashes. Handle
cycles deterministically without omitting changes within a cycle. Keep the
resolver-policy/version key small and independent of unrelated package contents.
No lockfile means correct request-scoped resolution, not a whole-tree fallback.

## Reviewed commit sequence

Each stage has a fresh implementation agent and an independent review agent.
The implementer does not commit. The reviewer reports high-confidence correctness
issues against the actual diff. Address findings and rerun focused validation
before the coordinator runs `scripts/spot-check` and creates that stage's commit.
Update affected crate READMEs and concise AGENTS contracts with their stage.
Never modify the dirty perf-optim worktree, afix, or the client-web migration.

1. **Lockfile package fingerprints.** Add the portable pinned parser dependency
   and a narrow package/importer adapter. Establish deterministic package-instance
   and named dependency fingerprints, including cycles, peers, patches and local
   identities. Resolve dependency/toolchain compatibility before proceeding.
   Test actual client-web parsing plus focused graph mutation/isolation cases.
2. **Scoped resolver input tracking.** Record the positive and negative
   filesystem probes that determine each request, including metadata and link
   targets; retain their provenance when using intra-build caches. Buffer manifest
   parsing rather than letting serde read an unbuffered File byte by byte.
   Preserve existing resolver results/errors and test shadowing, missing higher
   priority candidates, manifest edits, symlink retargets and import/require.
3. **Package-local FlatBuffer storage.** Add an independently versioned typed
   resolution format without invalidating unrelated source extraction records.
   Verify structure, integrity, discriminants and borrowed semantic invariants
   before exposing immutable records. Store portable identities/paths and
   per-request inputs with package fingerprints. Use atomic package-local writes,
   existing cache enable/read/write policy and explicit cache diagnostics.
   Cover corruption, truncation, incompatible versions, round trips and policy.
4. **Replace global fingerprint integration.** Integrate per-package persisted
   requests into BuildResolver. Stop trusting BuildSession targets based on a
   whole-inventory key; compare actual resolved edges for downstream invalidation.
   Keep intra-build package/probe memoization and derive output reuse from actual
   inputs/policy. Recheck consulted inputs before success, including disk hits
   and no-op builds. Test separate-process reuse and mutable-input invalidation.
5. **Demand-driven input inventory.** Remove eager full-tree inventory from
   ordinary builds and final checks. Admit/check reachable literal sources and
   resources on demand; enumerate only directories needed by finite contexts.
   Retain compact canonical/named link relationships where actually needed,
   reserved-path boundaries, external entry policy and deterministic CSS names.
   Keep directory-context additions/removals and consulted input mutations visible.
   Test that unused/broken node_modules subtrees are never traversed, while
   existing linked-inventory, context, CSS, Sass and map contracts still hold.
6. **Adoption and performance evidence.** Add focused native fixture and
   cross-session regressions exercising the complete feature. Extend/run existing
   cargo benchmarks rather than add another benchmark framework. Compare native
   fixture outcomes against the recorded baseline and run the real client-web
   release/maps workload with local timing evidence. Confirm the original
   315-second global inventory and giant Debug fingerprint are absent; distinguish
   any newly reached pre-existing app failure from cache regressions. Update the
   current adoption report and document measured cold/warm behavior honestly.

## Acceptance and evidence

- Relevant tests pass at each stage; every code commit follows spot-check.
- Cache hits use verified borrowed FlatBuffer access, not unpacked cache trees.
- A changed unrelated lockfile package does not invalidate unaffected package
  records. Dependency, policy and relevant mutable-input changes cannot reuse a
  stale resolution, including misses that become nearer/priority matches.
- No-lockfile, disabled-cache, read-only/unwritable-package and corruption paths
  preserve resolution correctness and explicit diagnostics.
- Literal-entry builds do not prewalk all of node_modules, either initially or
  during final validation. Context enumeration is limited by its actual pattern.
- Preserve existing portable runtime/graph/map/resource results. Existing fixture
  issues (async-cycle nondeterminism and the stale TypeScript assertion) are
  recorded baseline issues, never silently normalized or weakened.
- Keep raw client code, traces, maps, lockfile contents and profiles local.

## Progress

Stage 1 implementation and the three concrete review corrections are applied;
independent re-review and coordinator spot-check remain pending. Stages 2–6 are
pending. The adapter remains test-gated and does not change live resolution.

- Preserve Berry raw locators/workspaces and deterministic SCC/named-edge hashes.
- Mark every npm/shrinkwrap and Classic Yarn record incomplete because normalized
  graphs drop declarations and npm install-path instances. Include whole raw
  lockfile hashes (plus Classic root manifest hashes) in identities so discarded
  declarations/integrities still change fingerprints. Package isolation for these
  formats requires a future lossless overlay; no installed-tree authority claimed.
- Resolve normalized peers only through explicit selected dependency/optional
  providers; an exact peer constraint cannot select a disconnected package.
- Focused `cargo test -p bundler --lib lockfile::tests`: **9 passed, 1 ignored**.
  Four parser-backed regressions cover dropped npm required/importer declarations,
  Classic required/importer/optional declarations, collapsed npm installations,
  and exact peers with disconnected nodes (including explicit-provider success).
  The actual-project ignored probe was not rerun for these narrow corrections.
  Existing unrelated `ftree_cache` lifetime warning remains.
