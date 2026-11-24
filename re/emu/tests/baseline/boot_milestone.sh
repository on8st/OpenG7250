#!/bin/bash
# Boot-milestone regression (needs the local decrypted image-map).
#
# Separate from the fast m1_smoke.sh: this actually boots the stock Main and
# asserts it reaches its service-loop milestones (System started, AXP PSU,
# VersaClock, the forced default-IP LAN init, codec, RX mode). Skips cleanly
# when no image-map is configured. Run: bash tests/baseline/boot_milestone.sh
set -u
cd "$(dirname "$0")/../.." || exit 2
REPO="$PWD"; OUT="$REPO/.private/m1"; fail=0
ok(){ printf '  ok   %s\n' "$1"; }; bad(){ printf '  FAIL %s\n' "$1"; fail=1; }

if ! python3 harness/g7250emu/imagemap.py main >/dev/null 2>&1; then
  echo "boot milestone: SKIP (no image-map; set local.config + manifest hash)"; exit 0
fi

# The run ends 10 s after the last milestone is seen; 120 s is the hard limit, so a hang still
# fails. (Until 2026-11-24 a fixed 35 s budget: at a clean load the last milestone came at about
# 26 s, and a slower host ran out of it.)
LIMIT=120; HOLD=10
echo "[boot] stock Main, until the milestones are seen (+${HOLD} s), hard limit ${LIMIT} s"
rm -f "$OUT/boot_milestones.txt"
python3 harness/g7250emu/m1_run.py --until-milestones --hold $HOLD --timeout $LIMIT > "$OUT/m1_run.out" 2>&1
C="$OUT/main_console.txt"
# the margin, and the step's files kept for every run (the log gzipped; later boots overwrite $OUT)
if [ -f "$OUT/boot_milestones.txt" ]; then sed 's|^|  |' "$OUT/boot_milestones.txt"
else echo "  milestones: no margin file (m1_run.py failed, see .private/m1/m1_run.out)"; fi
K="$OUT/boot-runs/$(date -u +%Y%m%dT%H%M%SZ)"; mkdir -p "$K"
for f in boot.resc main_console.txt m1_run.out boot_milestones.txt m1_observe_report.md; do [ -f "$OUT/$f" ] && cp -p "$OUT/$f" "$K/"; done
gzip -c "$OUT/m1_renode.log" > "$K/m1_renode.log.gz" 2>/dev/null
echo "  kept: ${K#$REPO/}"
grep -q "System started."              "$C" && ok "System started"        || bad "System started"
grep -q "Init PSU"                      "$C" && ok "AXP223 PSU init"        || bad "AXP223 PSU init"
grep -q "5p49v6965"                     "$C" && ok "VersaClock 6 ready"     || bad "VersaClock 6"
grep -q "IP 192.168.3.199"              "$C" && ok "LAN default IP (A1.12)" || bad "LAN default IP"
grep -q "aic3204"                       "$C" && ok "audio codec ready"      || bad "audio codec"
grep -qi "Start RX mode"                "$C" && ok "TRX RX mode"            || bad "TRX RX mode"
grep -q "source IP set to 192.168.3.199" "$OUT/m1_renode.log" && ok "CH394 chip-side default IP (A1.12)" || bad "CH394 chip-side IP"

echo
[ $fail -eq 0 ] && { echo "boot milestone: ALL PASS"; exit 0; } || { echo "boot milestone: FAILURES"; exit 1; }
