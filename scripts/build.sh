#!/usr/bin/env bash

INSTALL_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
OG_DIR="$(pwd)"
cd $INSTALL_ROOT
cargo build --release
cd $OG_DIR
