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

Replace `src/packet/` manual unsafe headers with `pnet_packet` crate.
Fix: eliminate 300+ LOC of manual wire-format parsing. Reliability increase.

### Iteration 3 — CI pipeline (version 0.2.1)

GitHub Actions: build + test + lint on push/PR. Docker build CI step.

### Loop rule

After each user-persona validation, if any test fails → go back to Iteration start.
Only exit loop when user persona reports 100% pass on real instances.

---

## Current known issues (informed by user)

| Issue | Status | Notes |
|-------|--------|-------|
| icmp-ping broken | ✅ FIXED (91875d9) | Checksum included payload per RFC 792 |
| 1kb broken | ✅ FIXED (4bdcaa6) | UDP double-send eliminated |
| Manual packet headers | OPEN (Iter 2) | unsafe transmute decode in `src/packet/*` |
| No CI pipeline | OPEN (Iter 3) | No GitHub Actions |
| Doc inconsistency | OPEN (Iter 1) | AGENTS.md vs ARCHI.MD vs Cargo.toml mismatch |
