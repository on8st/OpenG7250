"""The fixture log of T-EMU-31 (m1_smoke step [14]): every power.cut entry names its outcome and,
when it fired, the boot it fired in and the flash.log entry it ended.
usage: cut_pon_check.py <test folder>   (prints OK or the failures; exit 1 on a failure)"""
import json
import os
import sys

d = sys.argv[1]
fx = [json.loads(x) for x in open(os.path.join(d, "fixture-log.jsonl")) if x.strip()]
fl = [json.loads(x) for x in open(os.path.join(d, "flash-log.jsonl")) if x.strip()]
cuts = [x for x in fx if x["op"] == "power.cut" and (x.get("answer") or {}).get("ok")]
bad = []


def need(c, why):
    if not c:
        bad.append(why)


pon = [x for x in cuts if x["args"].get("at_power_on")]
plain = [x for x in cuts if not x["args"].get("at_power_on")]
need(len(pon) == 7 and len(plain) == 1, "power.cut entries: %d at_power_on (want 7), %d plain (want 1)" % (len(pon), len(plain)))
for x in cuts:
    need(isinstance(x.get("boot"), int), "an entry without the boot of the call: %s" % x)
    need("fired" in x, "an entry without an outcome: %s" % x)
    if x.get("fired"):
        e = x.get("fired_entry") or {}
        match = [y for y in fl if y.get("boot") == x.get("fired_boot") and y.get("result") == "cut"
                 and y.get("op") == e.get("op") and y.get("address") == e.get("address") and y.get("t_us") == e.get("t_us")]
        need(isinstance(x.get("fired_boot"), int) and match, "fired, but its boot or flash.log entry is not found: %s" % x)
if len(pon) == 7:
    a1, a3, a4, a5, a6a, a6b, end = pon
    for x, op in ((a1, "erase"), (a3, "program"), (a5, "erase"), (a6b, "erase")):
        need(x.get("fired") is True and x.get("fired_boot") == x.get("armed_boot") and x["fired_boot"] > x["boot"]
             and (x.get("fired_entry") or {}).get("op") == op, "at_power_on fired in the boot it was armed for, after the call's boot (%s): %s" % (op, x))
    need(a4.get("fired") is False and a4.get("outcome") == "not fired: boot %s ended by a power cycle" % a4.get("armed_boot"),
         "not fired: dropped at the power cycle after its boot: %s" % a4)
    need(a6a.get("fired") is False and str(a6a.get("outcome", "")).startswith("replaced by a later power.cut at_power_on"),
         "replaced by the second at_power_on: %s" % a6a)
    need(end.get("fired") is False and end.get("outcome") == "not armed: no power-on after it before the end of the test",
         "armed at the end, never used: %s" % end)
if plain:
    p = plain[0]
    need(p.get("fired") is True and p.get("fired_boot") == p.get("boot"), "the plain cut fired in its own boot: %s" % p)
    if len(pon) == 7:
        need(pon[3]["fired_boot"] == p["fired_boot"] + 1, "the at_power_on cut fired in the start after the plain cut")
print("OK" if not bad else "\n".join(bad))
sys.exit(1 if bad else 0)
