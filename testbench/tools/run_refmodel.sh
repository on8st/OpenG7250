#!/bin/sh
# Run the test bench against the reference model on 127.0.0.1.
#   testbench/tools/run_refmodel.sh <outdir> [model options...] [-- testbench run options...]
# Example: run_refmodel.sh /tmp/tb --tx locked
# Builds the model (offline), checks that every port it and the bench need is free, starts it,
# exports the implemented v1.0 tests, runs them, stops it.
#
# Ports: the model runs on its own port base (default 20000: spec port p at 20000 + p, control
# at 27290, bench hosts at 29000-29199, below the ephemeral range), never on the spec ports 1024-7251 or on 30000-38299,
# which the emulator runner uses on this machine (testlist-format v1.4 section 3). The model and
# the emulator runner must never share local ports. A base of 0 (the spec ports themselves) is
# refused here unless G7250_ALLOW_SPEC_PORTS=1 is set, after checking that no emulator run is
# going.
# GPL-3.0-or-later, (c) 2026 ON8ST.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/.." && pwd)
out=$1; shift
model_args=""
base=""
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
    if [ "$1" = "--port-base" ]; then base=$2; fi
    model_args="$model_args $1"; shift
done
if [ -z "$base" ]; then
    # first base whose every port is free; all of them avoid 1024-7251, 30000-38299 and the
    # ephemeral range (the emulator runner's per-run base can be anywhere else)
    for b in 20000 17000 14000 11000 8000; do
        if python3 "$here/tools/ports_free.py" "$b" 2>/dev/null; then base=$b; break; fi
    done
    [ -n "$base" ] || { echo "not starting: no free port base" >&2; exit 2; }
    model_args="--port-base $base $model_args"
fi
[ "${1:-}" = "--" ] && shift
if [ "$base" = "0" ] && [ "${G7250_ALLOW_SPEC_PORTS:-0}" != "1" ]; then
    echo "refusing --port-base 0: the spec ports belong to the emulator runner (set G7250_ALLOW_SPEC_PORTS=1 only when no emulator run is going)" >&2
    exit 2
fi
mkdir -p "$out"
command -v cargo >/dev/null 2>&1 || PATH="$(dirname "$(rustup which cargo)"):$PATH"
# G7250_PROFILE=debug runs the model's debug build, with overflow checks (default: release)
profile=${G7250_PROFILE:-release}
if [ "$profile" = debug ]; then rel=""; else rel="--release"; fi
(cd "$root/product/refmodel" && cargo build $rel --offline -q)
python3 "$here/tools/ports_free.py" "$base" || { echo "not starting: ports in use" >&2; exit 2; }
# shellcheck disable=SC2086
"$root/product/refmodel/target/$profile/g7250-refmodel" --port-base "$base" $model_args --endpoint-file "$out/ep.json" >"$out/model.log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT INT TERM
sleep 1
cd "$here"
if [ -n "${G7250_TESTS:-}" ]; then
    # only these tests (comma-separated ids), e.g. the keyer tests against --cal full
    sel=""
    for t in $(echo "$G7250_TESTS" | tr ',' ' '); do sel="$sel --test $t"; done
    # shellcheck disable=SC2086
    python3 -m testbench run --endpoint "$out/ep.json" $sel -o "$out/result.json" "$@"
    exit $?
fi
python3 -m testbench list --implemented --scope v1.0 -o "$out/tests.json"
python3 -m testbench run --endpoint "$out/ep.json" --tests "$out/tests.json" -o "$out/result.json" "$@"
