#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Build, test, seal and check the G7250 main firmware. Output in out/ (not committed):
#   out/g7250-boot.elf      boot stage at 0x08020000 (with symbols)
#   out/g7250-app-a.elf     application linked for slot A, 0x08040000 (with symbols)
#   out/g7250-app-b.elf     application linked for slot B, 0x08080000 (with symbols)
#   out/g7250.elf           boot stage + slot A in one file, for a single-file loader (0034)
#   out/*.bin               flat sealed images; out/SHA256SUMS
set -eu
here=$(cd "$(dirname "$0")" && pwd)
cd "$here"
out="$here/out"
mkdir -p "$out"

echo "== host tests"
cargo test --quiet

# One revision for every image of this build; the build scripts rerun when it changes.
if [ -z "${G7250_REV:-}" ]; then
  G7250_REV=$(git rev-parse --short=10 HEAD 2>/dev/null || echo unknown)
  if [ -n "$(git status --porcelain -- . 2>/dev/null)" ]; then G7250_REV="$G7250_REV-dirty"; fi
fi
export G7250_REV
echo "== revision $G7250_REV"

echo "== target build"
(cd board && cargo build --release -p g7250-boot -p g7250-app)
(cd board && cargo build --release -p g7250-app --features slot-b --target-dir target/slot-b)
rel=board/target/thumbv7em-none-eabihf/release
relb=board/target/slot-b/thumbv7em-none-eabihf/release
cp "$rel/g7250-boot" "$out/g7250-boot.elf"
cp "$rel/g7250-app" "$out/g7250-app-a.elf"
cp "$relb/g7250-app" "$out/g7250-app-b.elf"

echo "== flash erase and program run from RAM"
sh board/check-ramfunc.sh "$out/g7250-boot.elf"
sh board/check-ramfunc.sh "$out/g7250-app-a.elf"
sh board/check-ramfunc.sh "$out/g7250-app-b.elf"

echo "== fault-tolerant flash read: nothing leaves the FAULTMASK window (R0)"
python3 board/check-faultmask.py "$out/g7250-boot.elf" "$out/g7250-app-a.elf" "$out/g7250-app-b.elf"

echo "== RAM kept over a reset: one address in every image (0068, A3.5)"
python3 board/check-kept-ram.py "$out/g7250-boot.elf" "$out/g7250-app-a.elf" "$out/g7250-app-b.elf"

echo "== safe state within 20 us of the boot stage's first instruction (0045)"
python3 board/check-safe-margin.py "$out/g7250-boot.elf"

echo "== seal and check"
tool="cargo run --quiet -p g7250-imgtool --"
$tool seal boot "$out/g7250-boot.elf" --bin "$out/g7250-boot.bin"
$tool seal app-a "$out/g7250-app-a.elf" --bin "$out/g7250-app-a.bin"
$tool seal app-b "$out/g7250-app-b.elf" --bin "$out/g7250-app-b.bin"
$tool check boot "$out/g7250-boot.elf"
$tool check app-a "$out/g7250-app-a.elf"
$tool check app-b "$out/g7250-app-b.elf"
$tool merge "$out/g7250.elf" "$out/g7250-boot.elf" "$out/g7250-app-a.elf"
# Update packages, one per slot (A1.8; 0043): Table M-4 header, then the sealed image.
$tool package app-a "$out/g7250-app-a.bin" "$out/g7250-update-a.pkg"
$tool package app-b "$out/g7250-app-b.bin" "$out/g7250-update-b.pkg"

echo "== test build: compaction at every start (emulator only)"
(cd board && cargo build --release -p g7250-app --features test-compact-at-start --target-dir target/test-compact)
mkdir -p "$out/test"
cp board/target/test-compact/thumbv7em-none-eabihf/release/g7250-app "$out/test/g7250-app-a-compact.elf"
sh board/check-ramfunc.sh "$out/test/g7250-app-a-compact.elf"
$tool seal app-a "$out/test/g7250-app-a-compact.elf"
$tool merge "$out/test/g7250-compact.elf" "$out/g7250-boot.elf" "$out/test/g7250-app-a-compact.elf"

echo "== test packages (T-A1-88, 0067): never shipped"
for slot in a b; do
  for case in bad-hw bad-kind bad-entry; do
    $tool test-package "app-$slot" "$out/g7250-app-$slot.bin" "$out/test/g7250-test-$case-$slot.pkg" "$case"
  done
done

(cd "$out" && shasum -a 256 g7250.elf g7250-boot.elf g7250-app-a.elf g7250-app-b.elf ./*.bin ./*.pkg test/g7250-compact.elf test/*.pkg > SHA256SUMS)
echo "== done"
cat "$out/SHA256SUMS"
