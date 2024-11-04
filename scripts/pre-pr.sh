#!/usr/bin/env bash
set -ex

yarn beachball change
cargo fmt --check
cargo test
cargo clippy
cargo udeps
