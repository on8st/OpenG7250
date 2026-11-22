"""Which failed tests of a product run failed only on a wall-clock wait across an erase.

  python3 harness/g7250emu/erase_wait.py <run dir>

A test is listed when (1) it failed or errored, (2) every failed check (or, without
checks, the reason) is an unanswered request or a timeout, and (3) at least two
management requests (UDP 7251, a retransmission) reached the radio while an erase was busy, or one
did (or came at most 50 ms before the erase) and its answer left only after the erase's end
(flash-log.jsonl, net-log.jsonl). Each listed test gives the answer's radio time after the request:
where it is longer than the bench's own wait in radio time too, the bench's limit, not only the
emulation's speed, is at work. During an erase the emulation runs at about 0.22 of real time
(docs/FLASH-model.md), so a bench wait counted in wall time ends inside the erase.
Such a result is not an image fault (bench decision 0069: waits across flash work in
radio time). The rule is a heuristic on the logs; each listed test names the erase.
"""
import json
import os
import re
import sys

UNANSWERED = re.compile(r"answered None|no reply|timed out|timeout|TimeoutError|unanswered|no answer|got \(None, '|got None", re.I)


def load(path):
    if not os.path.isfile(path):
        return []
    return [json.loads(l) for l in open(path) if l.strip()]


def failed_texts(e):
    bad = [c for c in e.get("checks", []) if not c.get("ok")]
    if bad:
        return ["%s %s" % (c.get("name", ""), c.get("detail", "")) for c in bad]
    return [e.get("reason") or ""]


def classify(run):
    res = json.load(open(os.path.join(run, "result.json")))
    out = []
    for e in res["results"]:
        if e.get("status") not in ("fail", "error"):
            continue
        texts = failed_texts(e)
        if not texts or not all(UNANSWERED.search(t) for t in texts):
            continue
        if any(t.startswith("timeout: ") for t in texts):      # the runner's own test timeout, not a bench wait
            continue
        d = os.path.join(run, e["id"])
        erases = [x for x in load(os.path.join(d, "flash-log.jsonl")) if x.get("op") in ("erase", "bank_erase") and x.get("t_end_us")]
        every = [x for x in load(os.path.join(d, "net-log.jsonl")) if x.get("radio_port") == 7251]
        mgmt = [x for x in every if x.get("dir") in ("to_radio", "dropped")]
        hits, answer = [], None
        for er in erases:
            inside = [m for m in mgmt if m.get("boot", 1) == er.get("boot", 1) and er["t_us"] <= m["t_us"] <= er["t_end_us"]]
            # a request in the erase or just before it, answered only after the erase's end
            near = [m for m in mgmt if m.get("boot", 1) == er.get("boot", 1) and m.get("dir") == "to_radio"
                    and er["t_us"] - 50000 <= m["t_us"] <= er["t_end_us"]]
            late = []
            for m in near:
                ans = [r for r in every if r.get("dir") == "from_radio" and r.get("boot", 1) == m.get("boot", 1)
                       and r["t_us"] > m["t_us"] and r.get("peer") == m.get("peer") and r.get("head", "")[16:24] == m.get("head", "")[16:24]]
                if ans and ans[0]["t_us"] >= er["t_end_us"]:
                    late.append((ans[0]["t_us"] - m["t_us"]) / 1e6)
            if len(inside) >= 2 or late:
                hits.append((er, len(inside)))
                answer = max(late) if late else answer
        if hits:
            er, n = hits[0]
            out.append({"id": e["id"], "status": e["status"],
                        "erase": "%s sector %s, %d-%d us (boot %s), %s s wall" % (er["op"], er.get("sector"), er["t_us"], er["t_end_us"],
                                                                           er.get("boot", 1), er.get("wall_s")),
                        "requests_during_erase": n, "failed": texts[0][:200], "answer_radio_s": answer})
    return out


if __name__ == "__main__":
    for x in classify(sys.argv[1]):
        print("%s %s: bench wall-clock wait across an erase (%s; %d management requests during it%s): %s"
              % (x["id"], x["status"], x["erase"], x["requests_during_erase"],
                 "; answered %.3f s of radio time after the request" % x["answer_radio_s"] if x["answer_radio_s"] is not None else "", x["failed"]))
