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

Stage 1 and the three concrete review corrections were independently approved,
spot-checked and committed as `6aeec15`. Stage 2 was independently reviewed,
spot-checked and committed as `cf3bc83`. Stage 3 is implemented below;
stages 4–6 remain pending. The adapter remains test-gated and
does not change live resolution.

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

### Stage 2 — scoped resolver input tracking

Implementation, independent review and coordinator spot-check are complete,
committed as `cf3bc83`. No stage 3–6 integration,
inventory/key changes or source-cache schema changes are included.

- Added explicit request-owned `InputObserver`/`InputProof` capture at existing
  resolver/context reads. Ordered probes include actual priority candidates,
  positive/negative ancestor node_modules/package contexts, canonical and file
  identities, and intermediate symlink/junction targets. No directory enumeration
  or hidden thread-local/global collector is used.
- `StructuralResolver::resolve_dependency_with_inputs` returns the unchanged
  dependency plus original proofs. Callers must key request mode/policy/revision
  and call `inputs.validate()` before authorizing reuse or success. Validation
  reads the filesystem directly, never intra-build caches. Old
  `DependencyResolver` behavior and independent import/require caches remain.
- Context positives/misses, derived browser probes and positive/negative package
  target caches retain original provenance, including values first populated by
  legacy calls. Prepopulated and unknown legacy context readers are explicitly
  unproven; they cannot authorize tracked hits. Masked probe IO errors are retained
  as proof errors, rather than silently authorizing a fallback.
- Manifests parse buffered raw bytes via `serde_json::from_slice`; SHA-256 seals
  exactly those parsed bytes. Ordinary candidates seal kind/identity/links only,
  leaving source-content validation to extraction.
- Added seven cross-platform resolver regressions covering higher-priority
  extension creation (and source-byte independence), cached positive/manifest
  provenance, cached target and ancestor misses/nearer shadows, independent modes,
  cached browser probes/manifest edits, package-scope misses, and intermediate
  junction/symlink retargeting under both identity policies. Both Windows junction
  policies ran successfully. Two context-cache tests cover negative provenance,
  invalidation and deliberately unproven readers/prepopulation. An additional
  Unix-only symlink-loop IO/fallback regression is present but not run on Windows.
- Created `ftree_cache/README.md` before editing that package; updated affected
  READMEs and concise AGENTS contracts.

Validation (from the explicit source-maps worktree; TEMP/TMP set to the ignored
`target\test-temp` managed-test root):

- `cargo test -p import_resolver -p packagejson -p ftree_cache -p packagejson_exports`:
  **37 passed** (19/4/2/12), all doctests passed.
- `cargo test -p ftree_cache -p import_resolver --features ftree_cache/deadlock_ref`:
  **21 passed** (2/19), all doctests passed.
- `cargo check -p bundler --lib`: **passed**.
- Changed Rust files formatted with the pinned `rustfmt`; `git diff --check`
  passed. The pre-existing ftree_cache lifetime-syntax warning remains.
- Full spot-check deliberately not run by the implementation agent.

### Stage 3 — independent package-local FlatBuffer storage

Implemented on the clean `source-maps` checkout at `cf3bc83`; independent review
and coordinator spot-check/commit remain pending. No resolver integration,
inventory removal or production source schema/extractor change is included.

- Authoritative `flatbuffers/resolution.fbs` defines independently versioned
  schema 1 (`GFRS`): typed import/require requests and file/external targets,
  package-instance/dependency/policy SHA-256 keys, anchored portable paths,
  ordered positive/negative probes, identity/content seals and link metadata.
  Pinned local flatc 25.2.10 generates `resolution_generated.rs` exactly.
  `generate-cache-flatbuffers.ps1 -Schema Resolution` leaves source bindings
  untouched; source schema 18/extractor 25 remain unchanged.
- `cache_flatbuffers::resolution::WireResolution` privately retains shared
  immutable bytes. Size-prefixed framing/checksum, independent version and all
  fingerprints, bounded verification (16 MiB, depth 16, 262144 tables), unknown
  discriminants, portable paths, proof shape and strictly ordered/unique request
  keys are checked through borrowed tables. Construction-only object APIs never
  become warm-load adapters.
- `PathInput::checked_from_parts` reconstructs only after fresh validation of
  supplied kind/canonical/identity/links/content against the filesystem. The
  bundler adapter reconstructs and discards one observation at a time, retaining
  no decoded proof tree. Empty proven footprints are valid for policy-only
  explicit externals; file targets require a positive canonical file proof.
- Private/test-gated `bundler::resolution_cache` supplies `Roots`,
  `CheckedResolution::{encode,requests,find,file_target,revalidate}` and
  `Store::{new,load,save}`. Stage 4 must remove the module test gate, derive a
  complete resolver/platform/policy key and check lockfile eligibility; no new
  lockfile authority is implied. File construction inputs are canonical targets,
  package-relative importers and original Stage 2 proofs, not restatted seals.
- Each package stores one complete aggregate per policy in reserved
  `.bundler-resolution-v1-<policy-sha256>.fb`. Updates replace that aggregate;
  preserve desired other requests explicitly. Different policies have different
  files. Existing cache modes/status/miss/write diagnostics are reused. Atomic
  same-directory tempfile write/flush/sync/rename never unlinks an old destination
  on failure. Windows reader contention can deny rename; failure is explicit and
  leaves old complete bytes. This is best-effort caching, not a build error.
- Runtime anchors distinguish package/project/cwd lexical, canonical and native
  Windows spellings; root retargeting is checked too. The cwd anchor is necessary:
  real Stage 2 bare-package resolution probes cwd-relative candidates even when
  cwd lies outside the project. Project-ancestor anchors cover Node lookup
  candidates/directories and ancestor `package.json`, without persisting machine
  roots. Internal Windows junctions roundtrip. Arbitrary external/cross-volume
  observations, unsafe/non-UTF8 names, absolute filesystem request spellings and
  unproven inputs are explicit construction failures: Stage 4 must skip
  persistence without dropping any footprint. UNC spelling is modeled, but no
  network-UNC filesystem test was available. Unix relative-symlink branches are
  present but not executed on Windows.
- Checked owners describe validation-time state. Stage 4 must revalidate them
  before reuse/build success, including no-op paths, and surface cache misses/
  failed-write diagnostics while continuing ordinary resolution. Saved identity
  changes on reinstall miss conservatively. Concurrent aggregate writers are
  last-successful-rename wins; there is no cross-process merge.

Stage 3 validation (Windows; managed test directories under ignored
`target\test-temp`, TEMP/TMP explicitly set there):

- `cargo test -p cache_flatbuffers -p ftree_cache`: **20 + 3 passed**;
  doctests passed. Six independent resolution-wire tests cover borrowing/shared
  ownership, positive/negative proof roundtrips, corruption/truncation/structure,
  fingerprint/version mismatches, all unknown discriminants, unsafe paths and
  duplicate/unordered requests, and verifier-table-budget exhaustion.
- `cargo test -p bundler resolution_cache --lib`: **10 passed**. Coverage includes
  actual Stage 2 package/empty-external footprints, scoped content/negative
  invalidation, modes/policy isolation, diagnosed corruption/publication errors,
  concurrent atomic replacement, project relocation/reinstall identity, internal
  Windows junction retargets and canonical-only package-anchor retargets.
- `cargo check -p bundler --lib`: **passed**.
- `cargo clippy -p cache_flatbuffers --all-targets -- -D warnings`: **passed**.
- `cargo test -p ftree_cache --features deadlock_ref`: **3 passed**, doctests
  passed. Pinned compiler regeneration reproduced byte-identical resolution
  bindings; all existing source schemas/bindings were confirmed unchanged.
- Changed handwritten Rust formatted; `git diff --check` passed.
- Full spot-check, independent review and commits belong to the coordinator.
  The pre-existing ftree_cache lifetime-syntax warning remains.
