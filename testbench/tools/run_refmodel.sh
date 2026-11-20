#!/bin/sh
# Run the test bench against the reference model on 127.0.0.1.
#   testbench/tools/run_refmodel.sh <outdir> [model options...] [-- testbench run options...]
# Example: run_refmodel.sh /tmp/tb --tx locked -- --param lock_reason=11
# Builds the model (offline), starts it, exports the implemented v1.0 tests, runs them, stops it.
# GPL-3.0-or-later, (c) 2026 ON8ST.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/.." && pwd)
out=$1; shift
model_args=""
while [ $# -gt 0 ] && [ "$1" != "--" ]; do model_args="$model_args $1"; shift; done
[ "${1:-}" = "--" ] && shift
mkdir -p "$out"
command -v cargo >/dev/null 2>&1 || PATH="$(dirname "$(rustup which cargo)"):$PATH"
(cd "$root/product/refmodel" && cargo build --release --offline -q)
# shellcheck disable=SC2086
"$root/product/refmodel/target/release/g7250-refmodel" $model_args --endpoint-file "$out/ep.json" >"$out/model.log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
sleep 1
cd "$here"
python3 -m testbench list --implemented --scope v1.0 -o "$out/tests.json"
python3 -m testbench run --endpoint "$out/ep.json" --tests "$out/tests.json" -o "$out/result.json" "$@"
