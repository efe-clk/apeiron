#!/usr/bin/env bash
# Backs a folder up with an installed program, restores it and compares it.
# Usage: packaging/try.sh <program>
set -euo pipefail
program=$1
work=$(mktemp -d)
mkdir "$work/src"
echo "hello" > "$work/src/a.txt"
"$program" --version
"$program" init "$work/repo"
"$program" --repo "$work/repo" backup "$work/src"
"$program" --repo "$work/repo" verify
"$program" --repo "$work/repo" restore main "$work/out"
cmp "$work/src/a.txt" "$work/out/a.txt"
rm -rf "$work"
