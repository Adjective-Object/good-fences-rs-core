# quickjs-bundler-harness

- Native QuickJS only; resolve the current example's in-memory map plus `harness`.
- Effects own immediate JSON snapshots, never guest argument references or IDs.
- Keep guest fixtures ordinary functions, not trusted Ecco purity declarations.
- Every `Hazard::ALL` addition needs a real runtime trigger in the coverage test.
- Drain jobs and check entry settlement; never compare failed runs as empty logs.
- `fuzz/` is a standalone workspace: total bounded byte decoding, real native
  output under all four policies, no source-error rejection or matching errors.
