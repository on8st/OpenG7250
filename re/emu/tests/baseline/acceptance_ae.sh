#!/bin/bash
# A–E acceptance gate: runs one targeted check per emulator-job action-list item
# and prints a per-item PASS/FAIL tally. Needs the local image-map; the --fpga
# leg is slow (~3 min). Run: bash tests/baseline/acceptance_ae.sh
set -u
cd "$(dirname "$0")/../.." || exit 2
REPO="$PWD"; OUT="$REPO/.private/m1"
declare -a R
pass(){ R+=("PASS  $1"); }; fail(){ R+=("FAIL  $1"); }
# known failures: reported, do not fail the run; an unexpected pass is XPASS
kfail(){ R+=("KFAIL $1"); }; xpass(){ R+=("XPASS $1"); }

if ! python3 harness/g7250emu/imagemap.py main >/dev/null 2>&1; then
  echo "acceptance: SKIP (no image-map)"; exit 0; fi

echo "[1/5] smoke (models + platforms: B1/C models, observe/E1, r0_sim/E2-pred)"
bash tests/baseline/m1_smoke.sh >/dev/null 2>&1 && pass "E1 observe + models (smoke 17 checks)" || fail "E1/models (smoke)"

echo "[2/5] boot milestones + superloop (A, A1.12, B2 chip-side IP)"
bash tests/baseline/boot_milestone.sh > "$OUT/boot_milestone.out" 2>&1 && rc=0 || rc=1
C="$OUT/main_console.txt"
BM=$(grep "^  milestones:" "$OUT/boot_milestone.out" | sed 's|^  ||'); echo "  $BM"; grep "^  kept:" "$OUT/boot_milestone.out"
[ $rc -eq 0 ] && pass "A1-A4 Main boot milestones (System started .. RX mode; $BM)" || fail "A1-A4 boot milestones ($BM)"
# A: the superloop itself. "Init Radio Server" plus >= 128 IWDG refreshes after it
# (harness/g7250emu/superloop_run.py; needs the MDMA and PA plant models).
python3 harness/g7250emu/superloop_run.py --timeout 240 --hold 30 >/dev/null 2>&1 \
  && pass "A  Main reaches its superloop (IWDG refreshed after Init Radio Server)" \
  || fail "A  Main superloop not reached"
grep -q "IP 192.168.3.199" "$C" && pass "A5 forced default IP 192.168.3.199" || fail "A5 default IP"
grep -q "Settings data succesfully loaded from EEPROM" "$C" && pass "B1 functional EEPROM (settings loaded)" || fail "B1 EEPROM"
grep -q "source IP set to 192.168.3.199" "$OUT/m1_renode.log" && pass "B2 CH394 chip-side SIPR (A1.12)" || fail "B2 CH394"

echo "[3/5] PW-1 confirmation (E2: Main matches the R0 prediction)"
python3 harness/g7250emu/r0_confirm.py --reuse >/dev/null 2>&1 && pass "E2 PW-1 emulator confirmation" || fail "E2 PW-1"

echo "[4/5] SLIP two-machine (D1: both boot, USART2<->USART2 carries SLIP)"
python3 harness/g7250emu/slip_run.py --timeout 45 >/dev/null 2>&1 && pass "D1 SLIP link (frames transmitted, Main receives)" || fail "D1 SLIP"

echo "[5/5] Control GUI loop (C/C2) + FPGA fused model (D2, --fpga, slow)"
python3 harness/g7250emu/control_run.py --timeout 50 >/dev/null 2>&1
grep -q "GT911 detected" "$OUT/../control/control_console.txt" 2>/dev/null && pass "C/C2 Control GUI-loop init (GT911/codec)" || fail "C/C2 Control"
bash tests/baseline/fpga_boot.sh >/dev/null 2>&1 && pass "D2a FPGA boot bus check (init, version, Check complete!)" || fail "D2a FPGA boot bus check"
# D2b KNOWN-FAIL until the FPGA model delivers realistic RX data (EMU-JOB-STATUS):
# runtime health errors in the superloop part of the superloop run's console.
SC="$OUT/../superloop/console.txt"
if [ -f "$SC" ] && grep -q "Init Radio Server" "$SC"; then
  RT=$(sed -n '/Init Radio Server/,$p' "$SC")
  nbe=$(printf '%s' "$RT" | grep -c "FPGA bus error"); nbu=$(printf '%s' "$RT" | grep -c "FPGA buffer underrun")
  if [ "$nbe" -eq 0 ] && [ "$nbu" -eq 0 ]; then xpass "D2b FPGA runtime health (no bus error / underrun in the superloop)"
  else kfail "D2b FPGA runtime health: $nbe x 'FPGA bus error', $nbu x 'FPGA buffer underrun' in the superloop"; fi
else
  fail "D2b FPGA runtime health (no superloop console)"
fi
# HIS: umbrella his/ (PR #4) when present; otherwise the his/ folder of another
# worktree (historically the separate g7250-his checkout),
# located via  his_dir = /abs/path  in local.config (git-ignored) or env G7250_HIS.
HIS="${G7250_HIS:-$(sed -n 's/^[[:space:]]*his_dir[[:space:]]*=[[:space:]]*//p' local.config 2>/dev/null | head -1)}"
for h in "$REPO/../../his" "$HIS"; do
  [ -n "$h" ] && { [ -f "$h/80-rf-fpga.md" ] || [ -f "$h/his/80-rf-fpga.md" ]; } && { HISOK=1; break; }; done
[ "${HISOK:-0}" = 1 ] && pass "E4 HIS 00-80 present" || fail "E4 HIS (umbrella his/ or his_dir)"

echo; echo "===== A–E acceptance tally ====="
ok=1; for r in "${R[@]}"; do echo "  $r"; [[ $r == FAIL* ]] && ok=0; done
nk=0; for r in "${R[@]}"; do [[ $r == KFAIL* ]] && nk=$((nk+1)); done
echo; [ $ok -eq 1 ] && { [ $nk -eq 0 ] && echo "A–E ACCEPTANCE: ALL PASS" || echo "A–E ACCEPTANCE: ALL PASS (with $nk known failure(s))"; } || echo "A–E ACCEPTANCE: FAILURES"
[ $ok -eq 1 ]
