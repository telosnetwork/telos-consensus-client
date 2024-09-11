#!/usr/bin/env bash

export RUST_BACKTRACE=1
export RUST_LOG=info
./target/release/telos-consensus-client --config config.toml
