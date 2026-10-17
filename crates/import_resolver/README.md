# import_resolver

Resolves Node/package and tsconfig paths using the repository's cached resolver.
`PathResolver` is the legacy single-policy interface. Resolver cache owners must
invalidate filesystem/configuration changes through `mark_dirty_root`.

Run `cargo test -p import_resolver`; filesystem tests use managed directories.

`structural::StructuralResolver` implements `DependencyResolver` with explicit
import/require conditions and separate caches (the legacy caches key by raw
specifier, not conditions). Only exact declared externals are accepted;
undeclared builtins fail. External export names are delegated to the host, not
validated as local files. Options retain explicit canonical/preserved symlink
policy. Graph source identities must use the same policy.

Call `invalidate` after filesystem/package/tsconfig edits, additions or removals;
`set_policy` invalidates automatically. Invalidation intentionally replaces both
cache trees, including misses/config ancestry. `revision` tracks these policy/
filesystem epochs. This correctness-first API does not promise minimal relinking.
