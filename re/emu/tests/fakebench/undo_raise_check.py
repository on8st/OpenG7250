"""undo_raise_check.py <result.json>: m1_smoke [15], G7250_FAKEBENCH_RAISE=T-EMU-22:silent,T-EMU-23:erase-fault.
T-EMU-22 raised inside its ch394.silent stretch is one error, its undos ran (silent dropped and a power
cycle to an answering boot, slot.corrupt restored); T-EMU-23 then runs in an ordinary boot up to its own
raise, with an erase fault armed and not taken, and its undos ran (flash.fault clear, flash.timing);
T-EMU-39 after them finds no fault armed and passes."""
import json
import sys

r = {e["id"]: e for e in json.load(open(sys.argv[1]))["results"]}
a, b, z = r.get("T-EMU-22", {}), r.get("T-EMU-23", {}), r.get("T-EMU-39", {})
why = a.get("reason") or ""
why23 = b.get("reason") or ""
ok = [("T-EMU-22 is an error raised on purpose", a.get("status") == "error" and "raised on purpose at T-EMU-22, silent" in why),
      ("its undo ran: ch394.silent dropped and a power cycle", "undone after the error: ch394.silent" in why),
      ("its slot.corrupt undone", "undone after the error: slot.corrupt slot 2 restored" in why),
      ("no undo failed", "undo failed" not in why + why23),
      ("T-EMU-23 ran in an ordinary boot to its own raise point (one error, not a failure in a silent boot)",
       b.get("status") == "error" and "raised on purpose at T-EMU-23, erase-fault" in why23),
      ("its flash.fault clear ran first, then flash.timing",
       "undone after the error: flash.fault clear; undone after the error: flash.timing erase_us null" in why23),
      ("T-EMU-39 after them: no fault armed, the clears work", z.get("status") == "pass")]
for name, good in ok:
    print(" ", "ok  " if good else "FAIL", name)
print("T-EMU-22:", why[:300])
print("T-EMU-23:", why23[:300])
print("T-EMU-39:", z.get("status"), [c["name"] for c in z.get("checks", []) if not c["ok"]][:5])
sys.exit(0 if all(g for _, g in ok) else 1)
