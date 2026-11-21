# HIS gate review 7 — his/next 6e2208a (CH394 maximum SPI clock)

Scope: `git diff 5f30ea1 6e2208a -- his` (his/40, his/90): F2-Q1, the
maximum SPI clock part, closed from the public WCH CH394 manual.

## Findings

| Statement | Judgement |
| --- | --- |
| 40: "the CH394 SPI device interface runs at up to 62 MHz in SPI mode 0 or 3 (CH394DS1 V1.2, features list; VERIFIED, datasheet)" | A datasheet fact about the chip, which is what review 6 (P2) asked for. VERIFIED fits a manual citation. I cannot open the manual from the allowed sources. The same figure appears independently in `re/emu/docs/G7250-emulator-test-spec-v0.5.md` (CH394 throughput row, "the public manual states up to 62 MHz SPI"), so the citation is consistent. "Mode 0 or 3" is consistent with the pin table (mode 0, VERIFIED). |
| 40: "About 17 MHz is the upper value seen in use on the board link, not a datasheet limit (MEDIUM)" | Unchanged from review 6, which judged it acceptable (A1). It stays useful now that the chip maximum is known, because 62 MHz says nothing about the board traces or connector. |
| 90 row F2-Q1, narrowed to the minimum RST# low time | Correct. The maximum SPI clock is noted as closed, and the RST# low time stays open as a datasheet look-up. |

No stock behaviour is added: no clock tree, no prescaler, no timing that stock
chose.

## Advisories (do not block)

- **B1 (40).** Say what the two clock values mean together, so that 62 MHz is
  not read as the board's limit. For example: "Until the board link is
  measured, the lower value (about 17 MHz) is the safe bound; 62 MHz is the
  chip limit only." A clock that is too fast causes link errors, not damage,
  which is why this is advisory.
- **B2 (40).** Cite the manual's section or page for the 62 MHz figure, as the
  other CH394 citations do (§10.4.3, table 6-1, §7.1.16). "Features list" alone
  is harder to trace.

## Verdict

**CLEARED** at 6e2208a (his/next). B1 and B2 are advisory.

Follow-up for the emu session: update the status in `his/README.md` to
6e2208a and review 7.

## Advisory check (diff 6e2208a..d07627e, his)

| Item | Result |
| --- | --- |
| B1 (40) | Applied. 62 MHz is stated as the chip limit only; the board link is not measured; about 17 MHz (MEDIUM) is the safe bound until it is. |
| B2 (40) | Applied. The citation is now chapter 1, features list, with the quoted line. Saying openly that the page number is not checked is acceptable: the chapter and the quote are enough to find the statement, and the tag stays a datasheet VERIFIED. |

No other change. Nothing about what stock does is added.

### Verdict (review 7, final)

**CLEARED** at d07627e (his/next).

Follow-up for the emu session: update the status in `his/README.md` to
d07627e and review 7.
