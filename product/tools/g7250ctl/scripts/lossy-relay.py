#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 ON8ST
"""Lossy UDP relay for g7250ctl runs against the reference model (T-A1-93 needs lost replies;
the model's fixtures lose none). Usage: lossy-relay.py LISTEN_PORT TARGET_PORT SECONDS.
Listens on 127.0.0.1:LISTEN_PORT, forwards to 127.0.0.1:TARGET_PORT, and drops the first reply
to each (request id, command) once. Logs one line per datagram. Standard library only."""
import select, socket, struct, sys, time

listen, target, seconds = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3])
front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
front.bind(("127.0.0.1", listen))
back = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
back.bind(("127.0.0.1", 0))
front.setblocking(False)
back.setblocking(False)
client = None
dropped = set()
end = time.time() + seconds
while time.time() < end:
    r, _, _ = select.select([front, back], [], [], 0.2)
    for s in r:
        d, a = s.recvfrom(2048)
        rid, cmd = struct.unpack(">IH", d[8:14]) if len(d) >= 14 else (0, 0)
        t = time.time() % 1000
        if s is front:
            client = a
            print(f"{t:8.3f} -> id {rid:08x} cmd {cmd:04x} len {len(d)}", flush=True)
            back.sendto(d, ("127.0.0.1", target))
        else:
            key = (rid, cmd)
            if key not in dropped:
                dropped.add(key)
                print(f"{t:8.3f} <- id {rid:08x} cmd {cmd:04x} status {struct.unpack('>H', d[14:16])[0]} DROPPED", flush=True)
                continue
            print(f"{t:8.3f} <- id {rid:08x} cmd {cmd:04x} status {struct.unpack('>H', d[14:16])[0]}", flush=True)
            front.sendto(d, client)
