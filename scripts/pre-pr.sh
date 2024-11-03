#!/usr/bin/env bash
set -ex

cargo fmt --check
cargo test
cargo clippy
cargo udeps
