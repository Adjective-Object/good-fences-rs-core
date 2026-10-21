# Native bundler differential fuzzing

`bundle_effects` generates bounded, valid in-memory ESM programs, runs them in
native QuickJS, then compares the complete owned effect log with real native
bundler output under all four combinations of generated async boundaries
off/on and `min_chunk_bytes` 0/4096. Source errors are generator bugs; parse,
build, output execution and effect mismatches are failures, never rejected inputs.
No filesystem module loading, Node, guest references or identity recorder is used.

This follows the Rust Fuzz Book's [cargo-fuzz tutorial](https://rust-fuzz.github.io/book/cargo-fuzz/tutorial.html)
and [structure-aware fuzzing](https://rust-fuzz.github.io/book/cargo-fuzz/structure-aware-fuzzing.html):
bytes select valid operations and module relationships instead of spending the
campaign rejecting random JavaScript syntax. libFuzzer supplies mutation,
coverage guidance, crash artifacts, corpus replay and minimization.

## Run

From `crates/quickjs-bundler-harness`, with `cargo-fuzz` and nightly installed
(the commands below use the repository's installed nightly):

```sh
cargo +nightly-2026-05-01 test --manifest-path fuzz/Cargo.toml --lib --locked
cargo +nightly-2026-05-01 fuzz build bundle_effects
# Copy tracked seeds into an ignored writable campaign corpus.
mkdir -p fuzz/corpus/bundle_effects
cp fuzz/seeds/* fuzz/corpus/bundle_effects/
cargo +nightly-2026-05-01 fuzz run bundle_effects -- -runs=100 -max_len=28
cargo +nightly-2026-05-01 fuzz run bundle_effects -- -max_total_time=3600 -max_len=28
cargo +nightly-2026-05-01 fuzz run bundle_effects fuzz/artifacts/bundle_effects/crash-HASH
cargo +nightly-2026-05-01 fuzz tmin bundle_effects fuzz/artifacts/bundle_effects/crash-HASH
```

Cargo-fuzz finds the nested `fuzz/` package from the harness directory. Its own
workspace and lockfile keep fuzz-only dependencies out of ordinary builds.
The default cargo-fuzz address sanitizer instruments Rust dependencies, and
`rquickjs-sys` also forwards the address-sanitizer setting to QuickJS's C build.
Interpreter coverage feedback is separate and requires Clang coverage flags:

```sh
CC=clang \
CFLAGS="-fsanitize=address -fsanitize-coverage=inline-8bit-counters,pc-table,trace-cmp -fno-omit-frame-pointer" \
cargo +nightly-2026-05-01 fuzz run bundle_effects \
  --target-dir fuzz/target/quickjs-coverage \
  -- -max_len=28 -max_total_time=3600
```

Use a compatible installed Clang (or its absolute path) for `CC`. This adds
interpreter C-edge feedback, not per-JavaScript-source branch counters. The
separate target directory keeps this optional build apart from the default.
Run workers as isolated processes: QuickJS limits are not an OS security sandbox.

## Input grammar and oracle

Every input, including empty and truncated prefixes, is valid. Only the first
28 bytes matter: four header bytes (missing bytes are zero), followed by at most
24 operation bytes. Header 0 selects shared versus exclusive static islands,
header 1 selects dynamic import order/repetition, header 2 sets initial state,
and header 3 selects an async-function guard and signed-input offset. Each operation byte selects one
of 16 operation families by its low nibble and a small numeric argument by its
high nibble. Its upper two bits additionally select direct, guarded, caught, or
awaited execution. There are exactly six modules and no unbounded guest loops.
Guards can skip operations; signed inputs exercise caught exceptions from the
private functions eligible for generated boundaries.

The vocabulary covers all eight Ecco hazards and pure controls, live exported
state/aliases, mutable snapshots, callback receivers, global reads, caught
throws, async functions, early iterator close, static/dynamic dependencies and
repeated imports. Exclusive islands permit parent hoisting; shared pure binding
islands permit multi-owner library clustering. A dependency-free module supplies
private functions eligible for opt-in generated boundaries. All awaits are
serialized, with no competing jobs, clocks, randomness or TDZ observations.

A small, identical unbundled guest entry imports `"harness"`, installs its
namespace as `globalThis.h`, imports the selected source or emitted entry,
and awaits its exported `main()` workload. An unfinished workload fails even
if its effect-log prefix matches.
The generated modules use that global without importing an external module:
unknown external seams otherwise conservatively disable the relocation passes.
This is fuzz-only setup, not a QuickJS host prelude or a trusted Ecco declaration.
Distinct island function names keep observable-name collisions from blocking
the supported parent-hoisting cases. Regression tests assert actual generated
boundaries, parent co-location, and a reduction in chunk count from clustering.

Panic diagnostics include the exact input bytes, configuration, source map,
source log, and (when available) emitted map and output log. Replay the saved
artifact with the standard cargo-fuzz command; no separate replay tool is needed.
