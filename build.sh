#!/bin/bash

INSTALL_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
OG_DIR="$(pwd)"

cargo build --release --features bad_sig_padding

cd $OG_DIR