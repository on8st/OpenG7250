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

echo "== seal and check"
tool="cargo run --quiet -p g7250-imgtool --"
$tool seal boot "$out/g7250-boot.elf" --bin "$out/g7250-boot.bin"
$tool seal app-a "$out/g7250-app-a.elf" --bin "$out/g7250-app-a.bin"
$tool seal app-b "$out/g7250-app-b.elf" --bin "$out/g7250-app-b.bin"
$tool check boot "$out/g7250-boot.elf"
$tool check app-a "$out/g7250-app-a.elf"
$tool check app-b "$out/g7250-app-b.elf"
$tool merge "$out/g7250.elf" "$out/g7250-boot.elf" "$out/g7250-app-a.elf"

(cd "$out" && shasum -a 256 g7250.elf g7250-boot.elf g7250-app-a.elf g7250-app-b.elf ./*.bin > SHA256SUMS)
echo "== done"
cat "$out/SHA256SUMS"
