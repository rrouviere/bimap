# Roadmap — Bimap

## Status: FIX & HARDEN LOOP

Original release done. User reports icmp-ping + 1kb broken.
Now in iterative fix loop: QA → Dev → User Persona → repeat until green.

---

## Original phase tracker (historical)

All P1–P7 complete. Original phases shipped but tests are failing in real-world scenarios.

---

## Current iteration plan

### Iteration 1 — Bug fixes (version 0.1.1)

| Step | Agent | Worktree | Deliverable | Status |
|------|-------|----------|-------------|--------|
| 0a | QA | `qa/icmp-ping` | Failing test reproducing icmp-ping breakage | ✅ DONE — checksum excludes payload |
| 0b | QA | `qa/1kb` | Failing test reproducing 1kb breakage | ✅ DONE — UDP double-send in all 4 functions |
| 0c | Dev | `fix/icmp-ping` | Fix icmp-ping, commit 91875d9 | ✅ DONE — 52/52 pass |
| 0d | Dev | `fix/1kb` | Fix 1kb, commit 4bdcaa6 | ✅ DONE — 52/52 pass |
| 0e | User persona | — | Validate both fixes end-to-end on real instances | ✅ DONE — 15/15 test_all pass, 7 manual spot checks all pass |

### Iteration 2 — pnet_packet migration (version 0.2.0)

| Step | Agent | Worktree | Deliverable | Status |
|------|-------|----------|-------------|--------|
| 2a | Dev | `feat/pnet-packet` | Replace manual headers with pnet_packet | ✅ DONE — commit 95ade15, -494 LOC |
| 2b | User persona | — | Validate real-world behavior after migration | ✅ DONE — all pass. Pre-existing UDP flakiness found (no retry loop on targets), not regression |

Replace `src/packet/` manual unsafe headers with `pnet_packet` crate.
Fix: eliminate 300+ LOC of manual wire-format parsing. Reliability increase.

### Iteration 3 — CI pipeline (version 0.2.1)

| Step | Agent | Worktree | Deliverable | Status |
|------|-------|----------|-------------|--------|
| 3a | Dev | `feat/ci` | GitHub Actions — build + test + lint on push/PR | ✅ DONE — commit f80d535 |
| 3b | User persona | — | Validate CI runs clean | ✅ DONE — all tests pass, pre-commit hook fires |

---

### Iteration 4 — Final audit & polish (version 0.2.2)

| Step | Agent | Worktree | Deliverable | Status |
|------|-------|----------|-------------|--------|
| 4a | Dev | `fix/audit-issues` | Convention compliance: unwrap, silent discards fixed | ✅ DONE — commit 8d29db3 |
| 4b | User persona | — | Final real-world validation | ❌ FAIL — DNS UDP timing race detected. New iteration needed |

---

### Iteration 5 — UDP target retry loop (version 0.2.3)

| Step | Agent | Worktree | Deliverable | Status |
|------|-------|----------|-------------|--------|
| 5a | QA | `qa/udp-retry` | Failing test proving UDP target race (DNS UDP, open UDP) | ⏳ PENDING |
| 5b | Dev | `fix/udp-retry` | Add retry loop to UDP target recv_from | ⏳ PENDING |
| 5c | User persona | — | Final real-world validation after fix | ⏳ PENDING |

---

## Known issues

| Issue | Status | Notes |
|-------|--------|-------|
| icmp-ping broken | ✅ FIXED (91875d9) | Checksum included payload per RFC 792 |
| 1kb broken | ✅ FIXED (4bdcaa6) | UDP double-send eliminated |
| Manual packet headers | ✅ FIXED (95ade15) | pnet_packet migration, -494 LOC |
| No CI pipeline | ✅ FIXED (f80d535) | GitHub Actions on push/PR |
| Doc inconsistency | ✅ FIXED | AGENTS.md, ARCHI.MD synced to 0.2.0+ |
| UDP target retry loop | OPEN | No retry on udp target recv_from → flaky under load |
