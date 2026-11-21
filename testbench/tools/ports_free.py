#!/usr/bin/env python3
"""Exit 0 if every local port the reference model and the bench would use with this port base is
free on 127.0.0.1, else print the ports in use and exit 1. Uses `lsof` (macOS, Linux).

  ports_free.py <base>        (base 0: the spec ports themselves)

The model binds base + each spec port of Table B2-1 and its control socket at base + 7290; the
bench binds its hosts at base + 9000 .. base + 9199 (testlist-format v1.4 section 3).
GPL-3.0-or-later, (c) 2026 ON8ST.
"""
import re
import subprocess
import sys

SPEC_PORTS = (1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)


def wanted(base: int) -> set[int]:
    ports = {base + p for p in SPEC_PORTS} | {base + 7290}
    if base:
        ports |= set(range(base + 9000, base + 9200))
    return ports


def in_use() -> set[int]:
    out = subprocess.run(["lsof", "-nP", "-iUDP", "-iTCP"], capture_output=True, text=True).stdout
    used = set()
    for m in re.finditer(r"(?:127\.0\.0\.1|\*|0\.0\.0\.0|\[::\]|localhost):(\d+)\b", out):
        used.add(int(m.group(1)))
    return used


def main() -> int:
    base = int(sys.argv[1]) if len(sys.argv) > 1 else 20000
    clash = sorted(wanted(base) & in_use())
    if clash:
        print(f"ports in use for base {base}: {clash[:20]}{' ...' if len(clash) > 20 else ''}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
