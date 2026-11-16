#!/usr/bin/env bash
# Per-phase compile timings and LLVM IR sizes for std and a few examples.
# Usage: scripts/bench_phases.sh <out-dir>
# ROCKC overrides the compiler binary (default: release rockc in the cargo target dir).
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
TARGET_DIR=${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps --manifest-path "$REPO/Cargo.toml" | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')}
ROCKC=${ROCKC:-$TARGET_DIR/release/rockc}
OUT=$1
EXAMPLES=(examples/hello.rk examples/showcase.rk examples/string_utils.rk examples/matrix.rk)

mkdir -p "$OUT"
export ROCK_TIME_PHASES=1
cd "$REPO"
"$ROCKC" --entry-file std/src/lib.rk --crate-name std --crate-type lib --no-std --no-prelude \
    --no-link --emit-llvm --output-dir "$OUT/std" --emit-object "$OUT/std/std.o" \
    --emit-artifact "$OUT/std/std.rkca" > "$OUT/std.time" 2>&1
for ex in "${EXAMPLES[@]}"; do
    name=$(basename "$ex" .rk)
    "$ROCKC" --entry-file "$ex" --no-std --extern-artifact std="$OUT/std/std.rkca" \
        --emit-llvm --output-dir "$OUT/$name" > "$OUT/$name.time" 2>&1
done
for f in "$OUT"/*.time; do echo "== $f"; cat "$f"; done
find "$OUT" -name '*.ll' -printf '%s %p\n'
