# Source-maps branch against client-web

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
- Client inputs are unchanged, including the pre-existing tracked modification.
  No proprietary source, source-containing maps, generated output or logs are
  committed or uploaded. Diagnostic copies and full logs remain local.

**The unmodified application does not bundle yet.** There is no successful full
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

## Confirmed blockers

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
scoping still changes intended global selectors. The original module-format
blockers remain; no new full-application pass is claimed.

## Remaining application coverage

The format blockers prevent exercising the complete dependency graph. Ori's
build constants, localization/loader plugins, worker packaging, lazy-module
configuration, CSS loading/rendering and authenticated app startup have not
been validated. Their existence is not evidence of additional reproduced
bundler failures. Do not claim the app will run merely after the two format
gates are removed.

Before a further app attempt: settle the source/vendor module-format policy,
configure mixed global/module styles, then rerun all four entries and inspect
the next actual failures. Avoid transpiling or flattening the dependency tree
into an approximate application graph and calling that an unmodified app pass.

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
