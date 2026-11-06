#!/usr/bin/env bash
# Starts the desktop app and checks that it is still running after a while.
# It shows that the app starts, not what its window looks like.
# Usage: packaging/try_desktop.sh <program>
set -euo pipefail
"$1" &
app=$!
sleep 15
if ! kill -0 "$app" 2>/dev/null; then
  wait "$app" || echo "the app ended with status $?"
  exit 1
fi
kill "$app"
