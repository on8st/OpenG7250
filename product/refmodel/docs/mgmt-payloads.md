# Management payload layouts (provisional, reference model T1)

The spec fixes the header, framing and status codes of the management channel (A1.4, Tables
A1-1 to A1-3) and leaves the payload of each command to be "fixed with the reference model
before M5" (A1 intro; 0020 §27). These are the layouts the reference model uses now. They are a
proposal for the spec session (via openg7250); until adopted, a tool must not rely on them
beyond the tests.

All values are big-endian. TLV = tag (1 byte), length (1 byte), value.

| Command | Request payload | Reply payload (status 0) |
|---|---|---|
| `IDENTIFY` 0x0001 | empty | TLV list, below |
| `GET_STATS` 0x0002 | empty | repeated: name length (1), name (ASCII), value (u64) |
| `GET_CHALLENGE` 0x0010 | empty | challenge, 16 bytes (length: finding SF-08) |
| `SET_FIRST_SECRET` 0x0011 | secret, 32 bytes | empty |
| `CHANGE_SECRET` 0x0012 | new secret XOR HMAC-SHA256(old secret, "G7NS" ‖ challenge), 32 bytes; the challenge is the one this request is authenticated with | empty |
| `END_WRITE_SESSION` 0x0013 | empty | empty |
| `SWITCH_LEVEL` 0x0023 | target level, 1 byte | empty |
| `RESTART` 0x0024, `FACTORY_RESET` 0x0022 | empty | empty |
| `READ_LOG` 0x0030 | first sequence number wanted (u32); 0 = oldest | repeated: sequence (u32), uptime ms (u32), text length (1), text (ASCII) |
| `CLEAR_LOG` 0x0031 | empty | empty |
| `UPDATE_STATUS` 0x0045 | empty | state (1): 0 idle, 1 update in progress |

Authenticated request: flag bit 0 set, 32-byte trailer = HMAC-SHA256(secret, challenge ‖ bytes
0 … 17 + N) (A1.4, A1.10).

`IDENTIFY` TLVs (A1.5 items I-1 to I-6):

| Tag | Length | Content |
|---|---|---|
| 1 | var | model, ASCII "G7250" (I-1) |
| 2 | 6 | MAC address (I-1) |
| 3 | var | serial number, only if the hardware has one (I-1; HIS-15); absent in the model |
| 4 | 4 | running image version: major, minor, build (u16) (I-2) |
| 5 | 4 | other slot version, 0 = empty (I-2) |
| 6 | 4 | boot stage version (I-2) |
| 7 | 4 | hardware revision, 0 = not identified (I-2; HIS-18) |
| 8 | 1 | bit 0 stock image retained, bit 1 vendor recovery supported; both 0 in this version (I-2a) |
| 9 | 2 | integrity of the running image and of the other slot: 0 none, 1 good, 2 bad (I-3) |
| 10 | 1 | update state: 0 normal, 1 trial boot, 2 rolled back (I-3) |
| 11 | 1 | secret set (I-3a) |
| 12 | 3 | first-secret window open (1), seconds left (u16) (I-3a) |
| 13 | 1 | active level (I-4) |
| 14 | 6 | owner IP address and UDP port, zeros if none (I-4) |
| 15 | 4 | `TX_ARM_STATE` word (I-4) |
| 16 | 4 | uptime in ms (I-5) |
| 17 | 1 | last reset cause, 0 unknown (I-5; HIS-13) |
| 18 | 5 | versions: management major, minor; P2 (44); record major, minor (I-5) |
| 19 | 1 | controller state: 0 unknown, 1 not detected, 2 detected (I-6) |
| 20 | 1 | TX calibrated (unlocked); proposal of finding SF-05, not in the spec |

A reader MUST skip unknown tags, so tags can be added.
