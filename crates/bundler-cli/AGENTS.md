# bundler-cli

Export `Cli` as both the standalone parser and embeddable clap arguments.
Keep argument syntax in `cli`, path/mode resolution in `invocation`, bounded
strict JSON and BuildOptions translation in `config`, orchestration in `execute`,
the fixture-compatible JSON contract in `report`, and scoped tracing in `trace`.
Libraries remain subscriber-free; tracing uses a local dispatch and flushes on
errors. The CLI always reports bundler::progress events to stderr; the optional
Chrome layer records audited duration spans only. Keep stdout JSON-only.
Use one process-wide uptime clock for ordinary and lockfile log prefixes;
enable ANSI severity colours only on interactive stderr, never redirected logs.
The progress layer renders coordinator-owned bundler::progress::lockfile counts
as a stderr terminal bar, or throttled plain updates when redirected. Clear
active bars on all exits; never put package names/paths in progress events.
The separate bundler::progress::discovery target samples root-relative filenames
and traversed directories on stderr only; never record those events in Chrome traces.
Trace files are create-new only, outside input/output except reserved
root-level `.bundler-trace-*` files.
Project discovery is cwd-only; positional fixture calls never discover config.
`resolveExtensions` is an ordered replacement for resolver suffixes, not an
append. Normalize optional leading dots, preserve compound suffixes and defaults
when omitted, and retain the effective order in resolver cache policy.
