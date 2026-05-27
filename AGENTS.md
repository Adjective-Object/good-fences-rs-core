# Making Changes

Before commiting any changes, check:

- have I written necessary tests for any code changes?
- have I run `./scripts/spot-check`?
- have I updated any out of date README.md files in the crates I have changed?

## Notes

- `cargo` commands can take a long time to compile. If you read `Compiling <foo>` in the output, DO NOT kill the command, wait for it.
- Bazel shell checks that invoke `cargo` should include `@rules_rust//rust/toolchain:current_cargo_files` and prepend the toolchain `bin` directory from runfiles to `PATH`.

## Style

- Integration tests that require real filesystem access should be run from within `tempdir` managed temporary directories
- never use `#[allow(lint)]`, always use `#[expect(lint)]` in case the underlying issue is ever fixed.
  If it is only errors in some build configurations, use e.g. `#[cfg_attr(feature = 'test', dead_code)]`
- Always use `eyre` for errors, never use `thiserror`
- Use `insta` for general purpose snapshot testing
- Use `inquire` for interactive user prompts
- Use `cargo bench` for benchmarking. Never use `criterion`.
- Raw git operations are usually kept in a `crate::git` module
- Prefer using `git2` for git operations over calling the `git` CLI, unless we are using git network operations.
- Use `prompt-manager` for LLM prompts, not builtin strings.

## Organization

Leave CONCISE notes on persistent design in AGENTS.md files in the roots of the crates, or the repo root for cross-cutting concerns.
Any temporary notes about mid-refactor or mid-implementation belong in the NOTES file instead.

- TODO.md files are formatted as a series of sections
- Implementation notes go in NOTES.\*.md for the current workplan.
- Architectural notes go in AGENTS.md. no long-form guides or source code excerpts
- `-cli` crates export both a lib and a bin. The lib exports the clap definition so commands can be embedded as subcommands in other binaries (e.g., `mx`)
