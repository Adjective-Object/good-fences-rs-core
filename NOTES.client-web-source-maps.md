# Source-maps branch against client-web

**Current status:** the module-format and global-style compatibility changes are
implemented, and branch integration fixes the UTF-8 comment-slicing panic.
The integrated four-entry release retry exceeded a 600-second bound without
publishing output or reporting a diagnostic. The upstream Oxc debug pure-comment
assertion remains unresolved. No successful full-application generation or
browser/runtime pass has been established.

## Scope

Local exercise on 2026-10-21:

- Bundler: `feature/bundler-source-maps`, `ca9170856637d17adbff1301482ebc334f4f1e85`.
- Client checkout: `4786fb2a0441ac042d3520654741fb4e41ea36a8`.
- Workload: Ori's **OWA Mail**, including mail bootstrap, data worker,
  analytics worker and the additional editor worker. Not just mail bootstrap.
- Build the release example with `cargo build -p bundler --release --example build`.
  Use the actual source tree, browser resolution, `source`/`fluentIconFont`/`style`
  conditions, four workers and source maps. Disable persistent caches so this
  exercise does not write sidecars into client-web.
- In this initial exercise, client inputs were unchanged, including the pre-existing tracked modification.
  No proprietary source, source-containing maps, generated output or logs are
  committed or uploaded. Diagnostic copies and full logs remain local.

**Initial result: the unmodified application did not bundle.** There is no successful full
mail output, browser execution, app startup measurement or full-app source-map
verification. Successful smaller controls below are not application passes.

## Reproduction

From the source-maps worktree, set `$app` to the client checkout and `$artifacts`
to an existing local directory outside both source trees. Write this fixture
configuration to `$artifacts\client-web-maps.json`:

```json
{
  "target": "browser",
  "workers": 4,
  "cache": "disabled",
  "resolveConditions": ["source", "fluentIconFont", "style"],
  "output": {"source_maps": true},
  "css": {"load": "auto-mount", "css_modules": "ext:.scss"}
}
```

The CSS selection is a diagnostic starting point, **not** a correct complete
application stylesheet policy; see the global/module distinction below.

```powershell
$entries = @(
  'packages\apps\mail\owa-mail-bootstrap\src\index.ts'
  'packages\libraries\worker\owa-data-worker-bootstrap\src\index.worker.ts'
  'shared\internal\owa-analytics-worker\src\index.worker.ts'
  'node_modules\@editor\index-readjuster-webworker-module-webpack5\lib\index-readjuster-worker.js'
)
.\target\release\examples\build.exe $app "$artifacts\mail-output" `
  --fixture-config "$artifacts\client-web-maps.json" @entries
```

Repeat with `output.source_maps: false` and a separate output directory to
distinguish input/format failures from map generation.

## Initial confirmed blockers

### ESM JavaScript in packages without a module type

The four-entry build fails on the editor worker:

```text
mixed CommonJS/ESM syntax at bytes 66..153:
this ESM .js input's package is missing required "type":"module"
in package.json (or explicitly selects CommonJS)
```

The diagnostic comes from `crates\oxc_utils_parse\src\lib.rs:50`. The installed
vendor entry contains an ESM import but its package does not declare the module
type required by the bundler. Browser target and export conditions do not
override that classification. This is an input compatibility blocker, not a
source-map diagnostic.

The maps-enabled attempt took **357.559 s** before returning this failure;
maps disabled returned the **same diagnostic** after **277.640 s**. Both exited
with status 1 and published no application generation. These are fresh-process,
cache-disabled command wall times, not successful build throughput numbers or
a controlled measurement of map overhead. Whole-root inventory happens before entry preparation;
unrelated repository contents remain a substantial preflight concern.
No trace was recorded, so this number does not attribute time to individual
stages.

### TypeScript source exports classified as CommonJS

Each original package independently fails before dependency traversal:

| Original package | Entry | Result |
| --- | --- | --- |
| `owa-mail-bootstrap` | `src\index.ts` | Oxc cannot lower ESM imports/exports to CommonJS |
| `owa-data-worker-bootstrap` | `src\index.worker.ts` | Same |
| `owa-analytics-worker` | `src\index.worker.ts` | Same |
| `owa-debounce` | `src\index.ts` | Same |

For a quick reproduction without whole-client inventory:

```powershell
.\target\release\examples\build.exe `
  "$app\shared\internal\owa-debounce" "$artifacts\debounce-output" `
  --fixture-config "$artifacts\client-web-maps.json" 'src\index.ts'
```

The packages use ESM TypeScript syntax without `"type":"module"`. The
precompiler preserves package-derived module format, and explicitly rejects
remaining module declarations in CommonJS output at
`crates\bundler\src\transpile.rs:115`. Native TS support therefore does not yet
make these workspaces directly consumable. These isolated failures return in
roughly 32-37 ms, without resolving any missing dependencies.

Adding `"type":"module"` to a **copied** leaf package's manifest lets its
unchanged TS sources compile and bundle. That proves the format gate, not that
changing every original workspace/vendor manifest would be a safe application
fix. A compatibility policy or explicit source-format adaptation is needed;
globally treating vendor CommonJS as ESM is not an acceptable workaround.

### Mixed global and modular Sass needs deliberate configuration

An isolated root with real linked Sass dependencies successfully compiles
`owa-expansion-animation`'s original stylesheet, including its webpack-style
`~owa-sass-mixins/lib/sass/owa-mixins.scss` import and Fluent UI transitive imports.
Native Sass, package conditions and linked input trees work here; external Sass
preprocessing is no longer a prerequisite.

At the initial revision, uniformly selecting all `.scss` inputs as CSS Modules
reproduced:

```text
missing Value export "default"
```

This came from `owa-global-styles`'s existing TS entry reexporting the default
of `globalstyles.global.scss`. The diagnostic copy has only its package format
adapted; its source and stylesheet bytes are unchanged. Repeating with
`css.css_modules: false` successfully publishes that same package.

Do not count the all-SCSS configuration failure as a Sass compiler or map bug.
The frontend follow-up below removes that missing-default-export failure.
It does not restore this stylesheet's intended global selector names when
module scoping is enabled. A full application adapter must
distinguish ordinary scoped styles, global styles and vendor exclusions instead
of applying one suffix selection indiscriminately. The bounded driver does not
reproduce Ori's per-file plugin rules.

## Successful bounded controls

| Input and explicit adaptation | Observed result |
| --- | --- |
| Copied `owa-debounce` source; manifest adds `"type":"module"` only | Native publish with maps on/off. Runtime debounce retains the last call only. JS map embeds exact TS contents and maps `clearTimeout` to original line 7, column 8. No maps emitted when disabled. |
| Copied `owa-error-codes` source; same manifest-only adaptation | 28 source parses, 28 JavaScript maps, 27 distinct mapped TS sources, 29 runtime exports. Every mapped `sourcesContent` matches the copied original; all executable files pass syntax checking. |
| Unchanged expansion-animation Sass copied to a bounded root; dependency junctions to real `owa-sass-mixins`, `@fluentui/react` and `@fluentui/react-theme-sass` | Native publish with maps on/off. CSS map sends `grid-template-rows` to original line 5, column 4. No stylesheet maps when disabled. |
| Copied global-styles source with module-format adaptation and CSS Modules disabled | Native publish succeeds with JS and stylesheet maps. This is not DOM/rendering verification. |

Mapped positions above use 1-based lines and 0-based UTF-16 columns, decoded
with the already-installed `@jridgewell/trace-mapping`. Sass reports five
individual deprecated-`@import` warnings plus one summary of 51 repetitive
warnings; they are visible diagnostics, not failed builds.

An early package-root-only Sass probe could not resolve ancestor dependencies
outside its bounded root. Supplying the three actual dependency junctions
resolved it without modifying Sass. That early failure is a harness boundary,
not evidence of missing native Sass package resolution.

Existing workspace junctions and ordinary inherited/commented tsconfigs did
not cause the historical inventory/tsconfig rejection. Do not carry those old
limitations forward from the earlier perf-optim investigation.

## CSS Modules default-map follow-up

The synthetic frontend now exports a default mapping object alongside individual
names under both CSS loaders. It uses the same normalized keys and composed
tokens. Named-only imports discard the unused object without discarding CSS or
auto-mount effects.

With the updated debug driver, the same copied global-styles entry and original
maps-enabled all-SCSS diagnostic configuration now publish through the native
backend. Importing that output with a recording document stub returns
`globalStyles` as an object with 17 string mappings and mounts one stylesheet;
a CSS source map is emitted. The missing-default link failure is fixed.
This is not real-browser rendering verification, and indiscriminate module
scoping still changes intended global selectors. At this follow-up, the original
module-format blockers remained; no full-application pass was claimed.

## Application coverage after the initial exercise

The initial format blockers prevented exercising the complete dependency graph. Ori's
build constants, localization/loader plugins, worker packaging, lazy-module
configuration, CSS loading/rendering and authenticated app startup have not
been validated. Their existence is not evidence of additional reproduced
bundler failures. Do not claim the app will run merely after the two format
gates are removed.

The next step was to settle the source/vendor module-format policy, configure
mixed global/module styles, then rerun all four entries and inspect actual
failures. Avoid transpiling or flattening the dependency tree into an approximate
application graph and calling that an unmodified app pass.

## Adoption compatibility follow-up

Bundler compatibility changes are in `8e08615`; the client migration remains in
its separate checkout:

- `client-web/scripts/setWorkspaceModuleTypes.mjs` adds `"type":"module"` to
  tracked workspace manifests, preserving explicit types, formatting, root
  tooling and dependencies. It updated **3211** manifests; its tests and
  idempotent `--check` pass, as do client-web's ESLint and oxlint under Node 24.
  The migration is staged separately in client-web. Its existing pre-commit
  hook rejects the configured Git email, so no client commit was created and
  neither the hook nor Git identity was changed. Unrelated client changes remain
  untouched.
- Static import/export syntax or `import.meta` disambiguates CommonJS-default
  `.js`/`.jsx`/`.ts`/`.tsx` inputs at parse time; dynamic import alone does not.
  Explicit `.mjs`/`.mts`/`.mjsx`/`.mtsx` remain ESM and
  `.cjs`/`.cts`/`.cjsx`/`.ctsx` remain CommonJS. Runtime ESM syntax in explicit
  CommonJS inputs is an error, not a classification override. Type-only imports
  in `.cts`/`.ctsx` remain erasable. Schema 18/extractor 25 stores
  the effective format and detection policy; warm hits reuse borrowed records
  without repeating syntax detection.
- `BuildOptions.css.css_modules_exclude` accepts dotted filename suffixes,
  overriding module scoping without changing stylesheet classification. It
  applies to entries, CSS import closures and composition targets. The app
  retry uses:

  ```json
  {
    "load": "auto-mount",
    "css_modules": "ext:.scss",
    "css_modules_exclude": [".global.css", ".global.scss", ".global.sass"]
  }
  ```

The **original**, uncopied `owa-debounce` and `owa-error-codes` packages now
publish native output with maps enabled and disabled. Debounce retains only the
last call, and its map sends `clearTimeout` to original line 7, UTF-16 column 8.
Error-codes produces 28 JavaScript maps and 29 runtime exports. All embedded
source contents match original files; neither maps-disabled control emits maps.
No package-format adaptation is needed beyond the requested workspace migration.

The bounded linked global-styles control also passes with `.global.scss`
excluded: existing global class/ID selectors remain unchanged, literal exported
class strings still agree, one link mounts with a recording document stub and
a stylesheet map is emitted. Its default is the existing **global stylesheet
URL**, not the CSS Modules mapping object. This is not real-browser rendering.

Affected Rust suites pass (721 tests across six crates), along with the required
spot-check and 15 focused final precompiler tests. Four CSS metadata tests and
five real-driver native CSS/Sass fixtures pass, including the extended global
suffix fixture. The full JavaScript harness unit suite is Windows-blocked by
its existing direct execution of fake `.mjs` drivers (`spawnSync ... EFTYPE`);
the real `.exe` driver does not have that launch limitation.

### Full-workload debug retries

Both four-entry debug attempts fail before publishing, with the same upstream
Oxc assertion at `oxc_parser/src/lexer/trivia_builder.rs:62`:
`assertion failed: comment.is_pure()`. The bundler then reports
`module preparation worker exited prematurely`. Maps enabled took 422.067 s;
maps disabled took 367.007 s. These are failed, cache-disabled command wall
times, not throughput or a controlled comparison with the earlier release runs.

The assertion is a `debug_assert!` in pure-comment retagging. It must not be
reported as a map-only failure or a proven release-build blocker. The current
panic diagnostic does not identify its input file. Bounded probes of the four
entry packages did not reproduce it; ordinary pure-comment controls also pass.
The triggering transitive input has not been localized.

### Full-workload release retries before branch integration

The rebuilt release driver gets past the original format diagnostics, but both
four-entry attempts fail before publication at
`crates\ast_segmenter\src\raw_module_deps.rs:41`:
`end byte index 20 is not a char boundary`. Maps enabled took 363.957 s;
maps disabled took 352.090 s. Both exit 1 through
`module preparation worker exited prematurely`, with no application generation.
These failed command wall times are not build-throughput measurements.

The root cause is `SymbolTags::from_comments_parent`: it trims a leading line
comment and slices its first 20 bytes to compare the ASCII
`@ALLOW-UNUSED-EXPORT` marker. A multibyte character crossing that offset panics
even when the comment has nothing to do with unused-export annotations.

A source-free reproduction is an `entry.mjs` whose first line is `// ` followed
by seven U+2500 characters, then `export const value = 1;` on the next line.
Before integration, both debug and release drivers reproduce the UTF-8 panic on this input. This
does not involve an ambiguous extension, package-format adaptation, proprietary
source, source maps or the upstream pure-comment assertion.

These were the two confirmed preparation blockers at this checkpoint.
Branch integration below fixes the Unicode prefix check; the Oxc assertion
still needs separate diagnosis. No full-graph link, published app maps, browser rendering,
authenticated startup or app-runtime pass is claimed. The successful bounded
controls above do not establish those outcomes.

## Branch integration follow-up

Imported all four missing `fix-linearization-client-web` commits, preserving
their authorship and recording original commit IDs. The code fix `cd935c8`
is now `9a1adc5`: comment annotation matching uses a checked UTF-8 prefix
instead of slicing through a character. The imported
`NOTES.fix-linearization-and-document-client-web.md` describes the older staged
input exercise, not a successful run of the current native TypeScript pipeline.

The `segment-aware` CSS changes were already represented:
`f70e2e1` / `6e1c7d9`, `39e2614` / `f9046a4`, `c3e78f4` / `8f1509e`,
and `443fdd2` / `0ade73d`. The first three pairs are patch-identical;
the last differs in pre-existing CLI context and historical fixture counts,
not missing CSS behavior. Replaying it would not add functionality.
The newer source-map, module-format, CSS default-export and exclusion behavior
is retained.

Integration checks:

- All **878 Rust library tests across eight crates** pass, including the new
  Unicode regression, plus the required spot-check.
- The exact source-free Unicode control above now builds with both debug and
  release drivers, with maps enabled and disabled. All four native outputs
  export `value = 1`; mapped outputs retain the exact Unicode source content,
  and unmapped outputs emit no maps.
- Original debounce/error-codes runtime and source-map controls, five native
  CSS/Sass contracts and the linked global-style exclusion control still pass.
- A saved pre-integration release driver still reproduces the Unicode panic.
  Comparing it with the integrated debug driver gives matching **72 native
  fixture passes / 93 emitted executions**, without legacy passes.
- The full fixture command is **not clean**: its extra diagnostic check still
  expects TypeScript to fail, although native TypeScript is supported. Both
  drivers fail that same stale assertion. An initial corpus run also failed
  the unbundled `native-async-cycles` source observation: concurrent cyclic
  imports occasionally log `unreachable a`. Later comparisons passed that
  case, but the source-side nondeterminism remains a real fixture issue,
  not a bundled-output pass or an integration regression.

The integrated release driver was also retried against the original four-entry
client workload with maps enabled, the same CSS exclusions and disabled caches.
It exceeded the **600-second bound** (600.179 s including timeout handling),
was terminated by the timeout, produced no stderr and published no generation.
The earlier UTF-8 panic was not observed before that bound, but an untraced
timeout does not identify the active stage or prove preparation completed.
This is neither a full-app pass nor evidence of a newly introduced performance
regression. Maps-disabled and full-app debug runs were not repeated after
integration; their earlier results remain historical.

## Measured client profile

Recorded the same four-entry release workload, maps enabled, caches disabled,
with detailed application spans and a PID-filtered samply recording. The build
was bounded at 480 s; timeout handling completed at 481.164 s with no generation
and no diagnostic. The Chrome trace is intentionally incomplete at termination;
aggregate exclusive durations account for open spans through that bound.
The samply recording attached later and covers approximately 285 s, not the
entire build. Its sampled CPU costs must not be added to phase wall times.

Before starting source workers, eager inventory consumed **315.106 s** for
**831,139 canonical files / 93,210 canonical directories**, encountering
927,560 directory entries:

| Inventory work | Exclusive wall time |
| --- | ---: |
| Entry processing, including canonicalization, metadata and JSON hashing | 238.764 s |
| Collecting config ancestors into a path-keyed BTreeSet | 39.528 s |
| Directory enumeration | 19.315 s |
| Existing/missing package and tsconfig probes | 10.300 s |
| Cross-tree scope validation | 3.921 s |

Formatting/hashing the full debug representation of that inventory for the
resolution fingerprint adds **7.021 s**. Preparation begins at about 322.4 s.
At the bound it has run for 158.758 s, including **147.103 s across 90,586
dependency-resolution attempts** on the coordinator. Nested context candidate
enumeration accounts for 5.577 s of that resolution time. The graph, linking,
emission and final recheck phases have not been reached.

Full local PDB symbolication, including inline frames, identifies these costs:

- The resolver's package-context loading path consumes **52.068 sampled CPU s**,
  with **51.358 CPU s** under `serde_json::from_reader` and **50.674 CPU s**
  through `std::io::Bytes<std::fs::File>`. At
  `crates/packagejson/src/packagejson.rs:142`, the reader receives an unbuffered
  `File`; repeated tiny reads incur kernel-call overhead even with warm disk
  caches. This is a concrete buffering opportunity, not evidence that JSON
  parsing itself requires a minute.
- The captured part of inventory accounts for **97.318 sampled CPU s**.
  Path component iteration/comparison and BTreeSet searches dominate the
  ancestor-collection stacks. The loop currently revisits every file's entire
  ancestry even when those directories are already known.
- Resolver canonicalization and file/directory probing remain substantial;
  `CachingNodeModulesResolver::wrap` alone includes **18.947 sampled CPU s**.
  The four extraction workers each accumulate only about **27 CPU s** during
  the preparation interval while the coordinator performs serial resolution.

Prioritize buffering package JSON reads, deduplicating ancestor traversal,
and replacing the inventory debug-string fingerprint with a structural key.
Then reduce redundant metadata/canonicalization using the checked inventory
without weakening junction handling or invalidation. These are proposed
optimizations; this change adds observability and does not implement them.
The observed delay is already explained before source-map emission or linking.

The existing `afix` library at `crates/common/iowalk` is a candidate for the
bounded read pipeline: it has pooled buffers, pre-read interception and native
Windows IoRing with a synchronous fallback. Its current public discovery API
returns tagged files, not the canonical directory/junction/alias graph required
by bundler inventory, so it is not a drop-in replacement. Parsing its supplied
package bytes would avoid reopening manifests through the unbuffered reader.
It currently inherits afix workspace helpers and Linux libxev/Zig dependencies;
choosing a pinned afix dependency versus extraction/vendor packaging remains
open. No afix files or cross-repository dependencies were changed.

Use the captured `symbolized.json.gz` with the matching local PDB directory
when loading samply. The original automatic symbol sidecar contained fallback
`fun_*` names; supplying `--symbol-dir` resolves actual Rust names and lines.
Samply 0.13.1 ignores `--duration` for Windows PID attachment and requires
Ctrl+C to finish recording. No profile or symbols were uploaded.

## Local evidence

Artifacts are under the local Copilot session's `files` directory:
`4f1953e4-a450-499e-be41-4e201dd09065`.

- `exercise-client-web.mjs`, `client-web-initial-results.json`: full workload
  arguments, maps-on/off outcomes and process wall times.
- `exercise-client-web-isolated.mjs`, `client-web-isolated-results.json`:
  original package failures and the manifest-only debounce control.
- `exercise-client-web-sass.mjs`, `client-web-sass-results.json`: bounded
  stylesheet input/dependency setup and maps-on/off results.
- `verify-and-expand.mjs`, `client-web-expanded-results.json`,
  `client-web-map-verification.json`: exact map positions, larger TS control,
  and global-style configuration comparison.
- Per-probe `.stdout`/`.stderr`, published generations and copied inputs stay
  local because maps and diagnostics can contain client source.
- `source-maps-spot-check.log`: required spot-check passes, including all 80
  ast-segmenter tests and the workspace check for the initial documentation-only
  investigation.
- `verify-css-defaults.mjs`, `css-default-verification.json`,
  `global-styles-default-map.stdout`/`.stderr`: default-map follow-up, five
  passing native CSS/Sass fixtures and copied global-styles runtime observations.
- `verify-original-workspaces.mjs`,
  `original-workspace-adoption-verification.json`: original workspace builds,
  runtime checks and exact source-map observations after the metadata migration.
- `verify-css-exclusions.mjs`, `css-exclusion-verification.json`: five native
  fixtures and the linked global-styles suffix-exclusion control.
- `adoption-rust-tests-2.log`, `adoption-final-checks.log`,
  `adoption-transpile-final.log`: affected Rust suites, corrected explicit-CJS
  diagnostic regression, required spot-check and rebuilt debug example.
- `client-module-lint-node24.log`, `client-module-oxlint.log`: client lint
  outcomes; root tooling remains unchanged despite Node's typeless-root warning.
- `exercise-client-adoption.mjs`, `client-adoption-results.json`: original
  four-entry debug retries with maps enabled/disabled and no published output.
- `adoption-debug-isolated-results.json`, `pure-comment-controls.json`: bounded
  entry probes and synthetic controls; neither localizes the full-graph panic.
- `adoption-release-build.log`, `client-adoption-release-results.json`: rebuilt
  release driver and the original maps-enabled/disabled four-entry failures.
  `exercise-client-adoption.mjs release` reproduces this profile with separate
  output directories and disabled persistent caches.
- `unicode-comment-control.json`: source-free UTF-8 prefix-slicing reproduction
  in both profiles; temporary input/output directories were cleaned up.
- `integration-rust-tests.log`, `integration-debug-build.log`,
  `integration-release-build.log`: integrated suites and rebuilt drivers.
- `verify-integrated-unicode.mjs`, `integration-unicode-results.json`: fixed
  Unicode control in both profiles and source-map settings.
- `integration-fixtures.log`: initial full-command failures, including the
  source-side async-cycle nondeterminism.
- `compare-integration-fixtures.mjs`, `integration-fixture-comparison.json`:
  exact baseline/integrated fixture outcome comparison and the unchanged
  stale TypeScript diagnostic check.
- `exercise-integrated-client-release.mjs`,
  `integration-client-release-results.json` and matching stdout/stderr:
  the bounded, maps-enabled four-entry release retry after integration.
- `client-profile-1792689011203`: full bounded Chrome trace, process result,
  samply recording, full-PDB symbol resolutions and analysis summaries. These
  are local artifacts, not committed inputs or published application output.
- `profile-client-build.ps1`, `summarize-client-trace.mjs`,
  `resolve-samply-symbols.mjs`, `analyze-samply.mjs`: local recording and analysis
  helpers; trace aggregation uses exclusive per-thread time.
- `profile-trace-tests.log`, `profile-example-tests.log`,
  `profile-inventory-tests.log`, `profile-spot-check.log`: tracing privacy,
  balanced spans, unchanged output/cache behavior, inventory contracts and
  required spot-check results.
