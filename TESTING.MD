# Testing Strategy — Bimap

## TDD Workflow

1. Write failing test (encodes the contract)
2. `cargo test` — must fail (red)
3. Implement minimum code — must pass (green)
4. Refactor, keep green
5. `cargo fmt && cargo clippy -- -D warnings`
6. `git commit` — pre-commit hook runs full suite

### Rule: never weaken an assertion

If a test fails:
- Fix the implementation (default choice)
- OR change the test AND justify in commit message body why the contract changed

### Test naming convention

```
<protocol>_<transport>_<scenario>_<expected>
```

```
open_tcp_success_matches_sha256
open_tcp_timeout_reports_timeout_fail
dns_udp_truncated_reports_malformed_fail
tls_connrefused_reports_refused_fail
```

## Test hierarchy

| Level | Location | Run | Time | Scope |
|-------|----------|-----|------|-------|
| L0 unit | `#[cfg(test)]` in `src/` | `cargo test --lib` | <2s | pure functions, no I/O |
| L1 integration | `tests/integration.rs` | `cargo test --test integration` | <10s | tokio tasks on localhost |
| L2 E2E | `tests/e2e.rs` | `cargo test --test e2e` | <30s | spawn subprocesses |
| L3 fault | `tests/fault.rs` | `cargo test --test fault` | <15s | inject broken protocol |

## L0 — Packet & control unit tests

Per source file `#[cfg(test)]` block:

- Header encode/decode roundtrip (IP, TCP, UDP, ICMP, DNS)
- Checksum correctness (RFC 1071 vectors)
- Edge values: all-zeroes, max fields, wraparound
- Control message JSON serde roundtrip (all 7 message types)
- Newline framing: split reads, multi-msg buffers, empty lines

## L1 — Integration (per protocol × transport × failure)

Two tokio tasks on localhost. Port range 10000–19999.

### open / 1kb — TCP

| # | Scenario | Expected |
|---|----------|----------|
| 1 | connect + send, server echoes | pass, sha256 match |
| 2 | port not bound | fail:timeout |
| 3 | server refuses (close before accept) | fail:refused |
| 4 | connect + reset before send | fail:reset |
| 5 | server reads 0 bytes | fail:0/sent |
| 6 | server reads 512 of 1024 | fail:mismatch |
| 7 | bind conflict (addr in use) | error:bind |

### open / 1kb — UDP

| # | Scenario | Expected |
|---|----------|----------|
| 1 | sendto + recvfrom | pass |
| 2 | no reader bound | pass (1B UDP), fail:timeout (1KB) |

### ICMP

| # | Scenario | Expected |
|---|----------|----------|
| 1 | echo request → reply | pass |
| 2 | no root privs | error:permission |
| 3 | no reply (blackhole) | fail:no-reply |

### TLS

| # | Scenario | Expected |
|---|----------|----------|
| 1 | handshake + 1KB echo | pass |
| 2 | plain TCP on TLS port | fail:tls-handshake |
| 3 | close after ClientHello | fail:tls-handshake |

### DNS

| # | Scenario | Expected |
|---|----------|----------|
| 1 | valid query → A response, UDP | pass |
| 2 | valid query → A response, TCP | pass |
| 3 | no response | fail:timeout |
| 4 | malformed reply (0 bytes) | fail:dns-malformed |

## L2 — E2E (subprocess)

Spawns `bimap --server` and `bimap --client` as separate processes.
Requires `cargo build --release` first.

| # | Scenario | Assertion |
|---|----------|-----------|
| 1 | `open tcp/10000-10010` all bound | exit 0, 10 pass |
| 2 | `open tcp/10000-10010` only 5 bound | exit 1, 5 pass, 5 fail |
| 3 | `1kb tcp/10000` | JSON line: sent=1024, received=1024 |
| 4 | `dns udp/10053` | exit 0, pass |
| 5 | `dns tcp/10053` | exit 0, pass |
| 6 | `tls tcp/10443` | exit 0, pass |
| 7 | `--bidir` | report count = test count × 2 |
| 8 | `--json` | each line is valid JSON with `test`,`transport`,`port`,`result` |
| 9 | `-q` | stdout contains no `pass` lines |
| 10 | `--test nosuch` | exit 2 |
| 11 | server unreachable | exit 3 |
| 12 | server SIGKILL mid-session | exit 3, no hang (10s timeout) |

## L3 — Fault injection

| # | Injection | Expected behavior |
|---|-----------|-------------------|
| 1 | close control channel mid-test | client exit 3, stderr logs disconnect |
| 2 | truncated TLS handshake (`close()` after 1 byte) | fail:tls-handshake, not panic |
| 3 | garbage bytes on test data stream | fail:mismatch |
| 4 | malformed JSON control message | error logged, channel dropped |
| 5 | oversized control message (>1MB) | rejected, error logged |

## Test data constants

All payloads are deterministic for verifiable SHA256:
- 1B: `0xAA`
- 1KB: `b"\x00\x01\x02...\xFF"` pattern looped
- DNS: query A `bimap.test.`
- TLS: same 1KB pattern as L4 test

## Pre-commit hook

`.githooks/pre-commit`:

```sh
#!/bin/sh
set -e
echo "[pre-commit] fmt";          cargo fmt -- --check
echo "[pre-commit] clippy";       cargo clippy --all-targets -- -D warnings
echo "[pre-commit] unit+int";     cargo test --lib --test integration
echo "[pre-commit] build";        cargo build --release --quiet
echo "[pre-commit] e2e";          cargo test --test e2e
echo "[pre-commit] fault";        cargo test --test fault
echo "[pre-commit] ok"
```

Install: `git config core.hooksPath .githooks`

## CI (no root)

```sh
cargo test --lib --test integration --test e2e --test fault
```

ICMP tests use `#[ignore = "root_required"]` and `geteuid() == 0` guard at runtime.

## Logging invariants

Every `Fail` and `Error` outcome carries a human-readable reason string.
No outcome is silent. Tests assert reason presence:

```rust
assert!(result.contains("timeout")
     || result.contains("refused")
     || result.contains("reset"));
```

Panics in test code indicate a panic in production code — never `#[should_panic]`.
