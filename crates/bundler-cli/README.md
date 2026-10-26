# bundler-cli

Source preparation and import resolution use bounded workers. An upfront local
package walk seeds speculative jobs, excluding dev dependencies and node_modules
sources; precise reachability still determines the bundle and its errors.
JSON `speculativeParses` counts unused eager extractions separately from demanded
`sourceParses` and emission reparsing.

Clap-based command-line entrypoint to `bundler::build::BuildSession`. Builds and
publishes native bundles, then writes one JSON result to stdout. Progress and
errors go to stderr; errors set a nonzero exit status. Error chains retain configuration/input paths,
the failing pipeline operation and underlying causes; embedded callers can inspect
typed causes or format the complete chain with `{error:#}`.

```sh
cargo build -p bundler-cli
cargo run -p bundler-cli -- --help
cargo run -p bundler-cli -- --config bundler.config.json
cargo run -p bundler-cli -- PROJECT OUTPUT ENTRY...
```

## Project configuration

With no positional arguments, the CLI reads `bundler.config.json` in the current
directory only. `--config FILE` or `--config=FILE` selects an explicit file;
missing/invalid files never fall back to discovery.

```json
{
  "root": ".",
  "outdir": ".bundler-out",
  "entries": ["src/index.ts"],
  "target": "browser",
  "workers": 4,
  "resolveExtensions": [".web.js", ".ts", ".tsx", ".js", ".jsx", ".json", ".node", ".jsonc"],
  "output": {"source_maps": true}
}
```

`entries` must be nonempty. `root` defaults to the config directory; relative
roots resolve from that directory. Entry paths, `outdir` (default
`.bundler-out`) and optional `traceFile` resolve from the root.
`--trace-file FILE` overrides `traceFile` and resolves from cwd.
Absolute paths stay absolute.

Configuration is strict JSON, bounded to **16 KiB**, rejecting unknown fields at
every level. This is a deliberately limited CLI schema, not serialized
`BuildOptions`. Build settings, with defaults:

```json
{
  "target": "node",
  "workers": 1,
  "cache": "read-write",
  "policyAllowlist": [],
  "resolveConditions": [],
  "output": {
    "base_url": "./",
    "min_chunk_bytes": 0,
    "synthesize_async_boundaries": false,
    "lazy_runtimes": [],
    "source_maps": false
  },
  "css": {
    "load": "caller-loaded",
    "css_modules": false,
    "css_modules_exclude": []
  }
}
```

Targets are `node`/`browser`, workers are 1-64, and cache is `read-write`/`disabled`.
`resolveConditions` adds nonempty conditions to target defaults; `import` and
`require` cannot be overridden.
`resolveExtensions` replaces the default extension-probing list in the supplied
order, after exact files, for file and directory-index requests.
Omitting it preserves existing resolution: a `.js` fast path for extensionless
files, then suffixes `ts`, `tsx`, `js`, `jsx`, `json`, `node`, `jsonc`.
Explicit lists bypass that fast path; `[]` disables extension probing, not
exact-file imports or explicit TypeScript substitutions.
Suffixes accept an optional leading dot and may be compound, for example
`[".web.ts", ".web.tsx", ".web.js", ".ts", ".tsx", ".js", ".jsx", ".json"]`.
They must have nonempty dot-separated components and contain no whitespace,
control characters, path separators or reserved filename characters.
Changing the list or its order changes resolver cache policy.
`lazy_runtimes` accepts at most 16 exact
`module`/`import`/`action` objects, each containing `module` (a portable
root-relative file) and `export` (a nonempty, control-free name, max 256 bytes).

### Explicit unsafe policy waivers

Policy rejections include a stable name, source path, byte span and bounded code
excerpt with line/column when available. Preparation logs demanded failures as
they arrive, finishes reachable source/facts preparation, then fails with a sorted
summary. It does not plan, link or publish after any unallowed failure.

```json
{
  "policyAllowlist": [
    {
      "source": "node_modules/lottie-web/build/player/lottie.js",
      "policies": ["uses-eval"]
    }
  ]
}
```

**Bundled output may be incorrect.** Each encountered waived occurrence warns
immediately on stderr, even if another file later fails, and appears in successful
JSON warnings. Waivers preserve source code; they do not repair it, evaluate code
strings or authorize any other policy/file. Paths are exact, case-sensitive,
slash-only canonical root-relative source identities: no globs, traversal,
absolute paths or alias expansion. Unknown names, unknown fields, missing/empty
policies and malformed paths are errors. Removing a waiver rejects the source
again on warm disk, session-memory and no-op builds without reparsing hits.

Supported names: `uses-eval`, `require-reassignment`, `require-indirect-use`,
`mixed-esm-commonjs`, `module-access`, `esm-require-context`, `wrapper-arguments`,
`wrapper-redeclaration`, `import-attributes`, `json-import-bindings`,
`dynamic-import-options`, `import-type-options`, `with-statement`, `import-meta`,
`commonjs-top-level-await`, `resource-management-transform`, `jsx-transform`,
`decorator-transform`, `auto-accessor-transform`, `typescript-enum-transform`,
`typescript-namespace-transform`, `typescript-parameter-property-transform`,
`typescript-import-assignment`, `typescript-export-assignment`,
`typescript-namespace-export`. Waiving a source-local gate does not bypass parser,
semantic, extraction, resolution, linking or output validation errors.

CSS loading is `caller-loaded` or `auto-mount`; `--css-auto-mount` overrides it.
CSS Modules accepts `false`, `true`, `"everything"` or `"ext:.custom.suffix"`;
exclusion suffixes override scoping. See the bundler README for resource,
source-map, lazy-runtime and CSS semantics. Auto-mount requires a usable DOM;
inserting links synchronously does not promise stylesheet readiness.

## Positional and fixture builds

```sh
cargo run -p bundler-cli -- PROJECT OUTPUT --fixture-config settings.json ENTRY...
```

ROOT/OUTPUT are cwd-relative; entries are root-relative. Positional calls never
discover project configuration. `--fixture-config` accepts the build settings
above, but not project `root`, `outdir`, `entries` or `traceFile`.
Project `--config` cannot be mixed with positional or fixture arguments.
Use `--` before entry filenames beginning with a hyphen.

The JSON report preserves the fixture driver's entries, backend/layout,
generation directory, resources/stylesheets, build counters, warnings and
module/segment ownership. The fixture and browser harnesses use the compiled
`target/debug/bundler-cli` binary (`.exe` on Windows).

## Tracing

Every build installs a scoped tracing reporter for `bundler::progress` events.
It reports configuration, declarations, resolution/module preparation, graph
construction, planning, emission, rendering, input rechecks and publication on
stderr, with module/cache counts during preparation (at most once per second)
and a final elapsed-time/write-count summary. Help/version remain quiet.
Embedded calls use the same scoped reporter without changing the global subscriber.
Log prefixes include elapsed program time in seconds, shared across scoped CLI
invocations. Interactive stderr uses coloured severity prefixes; redirected
stderr remains plain text. Lockfile count logs use the same uptime clock.
Initial discovery reports package counts and samples root-relative filenames and
traversed directories, at most once per second per kind. Package probes and the
`jwalk` directory walk both use bounded workers; sampled names never enter traces.
Lockfile progress distinguishes parsing from package-graph fingerprinting;
duration traces separately measure parsing, normalization, fingerprinting and
input validation/rechecks.
Fingerprinting uses the configured `workers` limit, with dependency-ready hash
jobs and cycle-safe component scheduling. A live stderr progress bar counts
completed package/importer nodes in terminals (redrawn at most 10 times/second).
Redirected stderr gets plain start/end counts and at most one intermediate
update/second. The bar clears on success, failure and early teardown; stdout
remains JSON-only. Trace spans include coordinator-owned `lockfile.fingerprint`
and worker-owned `lockfile.member`/`lockfile.component` durations.

`--trace-file FILE` writes a new Chrome/Perfetto-compatible local duration trace.
It cannot overwrite an existing file or write inside output/published trees.
Keep it outside inputs, or use a root-level `.bundler-trace-*` filename.
Parent traversal and alternate data streams are rejected. Traces contain only
audited bundler/cache spans and flush on returned errors without changing stdout
JSON. Choose a new filename per run; do not commit traces.

## Embedding and layout

The library exports `bundler_cli::Cli`, usable with `clap::Parser` directly or as
a clap subcommand payload. `Cli::run` uses cwd; `Cli::run_from` accepts an explicit
absolute working directory. Both take an output writer and return `eyre::Result`.

`cli` owns clap syntax, `invocation` resolves modes/paths, `config` validates JSON
and translates options, `execute` builds/publishes, `report` writes JSON, and
`trace` owns scoped tracing and destination safety; `progress` renders lockfile
counts. `main` only parses and runs.

Validate with `cargo test -p bundler-cli`, then
`cargo build -p bundler-cli && node scripts/bundler-fixtures.mjs`.
