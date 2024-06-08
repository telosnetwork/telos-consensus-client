#!/bin/bash

INSTALL_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )/.." && pwd )"
OG_DIR="$(pwd)"
cd $INSTALL_ROOT

LOG_NAME="$(basename $INSTALL_ROOT)"
LOG_PATH="$INSTALL_ROOT/$LOG_NAME.log"

CLIENT_BIN="$INSTALL_ROOT/target/release/telos-consensus-client"

nohup $CLIENT_BIN --config config.toml \
                       "$@" >> "$LOG_PATH" 2>&1 &

PID="$!"
echo "telos-consensus-client started with pid $PID logging to $LOG_PATH"
echo $PID > $INSTALL_ROOT/telos-consensus.pid
cd $OG_DIR