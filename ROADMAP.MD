# Roadmap — Bimap

## Status: RELEASED (all original phases complete)

52 tests pass: 35 unit + 4 integration + 11 e2e + 2 fault.
Pre-commit hook runs fmt → clippy → unit → build → integration → e2e → fault.

---

## Phase tracker

| Phase | Status | Files | Notes |
|-------|--------|-------|-------|
| P1 — Scaffolding | DONE | `Cargo.toml`, `src/main.rs`, `src/cli.rs`, `src/control/mod.rs`, `src/test/mod.rs`, `src/packet/mod.rs` | 7 deps, clap CLI with all flags |
| P2a — Control messages | DONE | `src/control/msg.rs` | 7 message types, serde, newline framing, unit tests |
| P2b — TLS + cert | DONE | `src/control/tls.rs` | Ephemeral cert via `rcgen`, fingerprint, TLS accept/connect |
| P2 — Control channel | DONE | `src/control/mod.rs` | `ControlChannel` (TLS, JSON framing, verbose dump at `-vvv`) |
| P3a — TestProtocol trait | DONE | `src/test/mod.rs` | Trait + registry + `Layer`/`Transport`/`Direction` enums |
| P3b — Orchestrator | DONE | `src/orchestrator.rs` | `run_server()`, `run_client()`, `ClientConfig`, bidir logic, result comparison |
| P4a — IP + ICMP headers | DONE | `src/packet/ip.rs`, `src/packet/icmp.rs` | Manual C-style structs, encode/decode, checksum |
| P4b — TCP + UDP headers | DONE | `src/packet/tcp.rs`, `src/packet/udp.rs` | Manual C-style structs, checksum |
| P4c — DNS headers | DONE | `src/packet/dns.rs` | Wraps hickory-proto (query build, response parse) |
| P5A — Port tests | DONE | `src/test/port.rs` | `OpenTest` + `KbTest` (TCP+UDP, retry loops, SHA-256 verify) |
| P5B — ICMP tests | DONE | `src/test/icmp.rs` | `IcmpPingTest`, `IcmpFullTest`, full ICMP type enum, raw socket |
| P5C — TLS test | DONE | `src/test/tls_test.rs` | TLS handshake + 1KB, 20-attempt retry loop |
| P5D — DNS test | DONE | `src/test/dns.rs` | UDP+TCP DNS, retry loop on UDP |
| P5 — Protocol integration | DONE | `src/test/mod.rs` (registry), `src/cli.rs` | All 6 protocols registered, `--test` accepts any |
| P6 — E2E harness | DONE | `tests/e2e.rs` | 11 scenarios: help, version, full-open loopback, fingerprint, exit codes, port-range mismatch |
| P7 — Pre-commit hook | DONE | `.githooks/pre-commit`, `AGENTS.md`, `ARCHI.MD`, `USAGE.MD`, `TESTING.MD`, `ROADMAP.MD` | Full docs + hook |

---

## Post-phase fixes and refinements

Discovered during integration/E2E testing after the original phase delivered:

### DNS TCP double-read bug (`src/test/dns.rs`)
Server had two consecutive `read_exact` calls on the 2-byte length prefix — first wrapped in `tokio::time::timeout`, second without. The second read overwrote `len_buf` with the first 2 bytes of the query body (`[0, 0]` → `query_len = 0`), causing desync on all subsequent messages. Fixed by removing the duplicate read.

### UDP retry loop (`src/test/dns.rs`, `src/test/port.rs`)
UDP tests were flaky due to a race between initiator and target: `send_to` returns immediately, but `recv_from` on the target might not be ready yet. Added 5-attempt retry with 200ms spacing. Affects DNS UDP, open UDP, and 1KB UDP.

### TLS infinite connect loop (`src/test/tls_test.rs`)
TLS initiator retried `connect` in an infinite loop on `ConnectionRefused`. Capped at 20 attempts with `tokio::time::timeout`.

### `--json` output (`src/orchestrator.rs`)
Added after P6. Each result printed as one JSON line: `{"id":..., "protocol":..., "transport":..., "port":..., "direction":..., "status":..., "reason":..., "tx":..., "rx":...}`.

### IPv6 support (`src/main.rs`, `src/test/port.rs`)
Original code unwrapped `IpAddr` to `Ipv4Addr` via `to_canonical()`. Changed to pass `IpAddr` through directly. All socket creation uses `match addr { IpAddr::V4 => ..., IpAddr::V6 => ... }`.

### Server life cycle (`src/orchestrator.rs`)
Original server had a 60s idle timeout that terminated after first client. Removed — server now runs forever, accepting multiple sequential client connections.

### `SO_REUSEADDR` (`src/main.rs`)
Server TCP bind uses `tokio::net::TcpSocket::set_reuseaddr(true)` before `bind()` to avoid `EADDRINUSE` on restart.

### Port-range / test validation (`src/main.rs`, `src/orchestrator.rs`)
- ICMP-only tests now implicitly add `icmp/any` when no `--port-range` given → exit 3 (connection error) instead of exit 2 (config error)
- ICMP tests with mismatched `--port-range tcp/42` also auto-add `icmp/any`
- L4/L7 tests without `--port-range` → exit 2 (config error)
- L4/L7 tests with partial coverage (e.g. `open` with `tcp/35000` only, covering TCP but not UDP) → run TCP, skip UDP, no error
- After all tests run, if zero results → error `"no tests matched the given port ranges"` instead of silent success (exit 0)

### Bind error hints (`src/main.rs`)
If bind fails on port 443, hint `(try with sudo or use a different --port)`. If `EADDRINUSE`, hint `(address already in use, wait or change port)`.

### GLIBC compatibility (`Dockerfile`)
Docker final stage changed from `debian:bookworm-slim` to `debian:trixie-slim` (Debian 13) to match GLIBC version in `rust:1.90-slim` builder image.

### `open` partial transport coverage (`src/orchestrator.rs`)
`open` supports both TCP and UDP. When user specifies `--port-range tcp/35000-35001`, only TCP is covered. Original code errored about missing UDP. Fixed: only error when NO transport for a protocol has a matching range.

---

## Current known issues

### ICMP requires root
ICMP raw sockets need `CAP_NET_RAW` or root. Tests are `#[ignore = "root_required"]` with runtime `geteuid() == 0` guard. Docker tests can't cover ICMP without `--privileged`.

### No CI pipeline
No GitHub Actions / CI config. Tests run locally via pre-commit hook. Docker build is manual (`docker build . && docker run`).

### Manual packet headers
IP, TCP, UDP, ICMP headers are manual C-style structs with `unsafe` transmute decode. ARCHI.MD flags `pnet_packet` as a future replacement to reduce risk of wire-format bugs.

### IPv6 e2e not covered
E2E tests only exercise IPv4 loopback. IPv6 code paths (socket creation, address display) are untested by CI.

### No parallel test execution
Tests run sequentially within a single client connection. Adding tokio::join or bounded channel concurrency is future work.

---

## Future work

| Item | Effort | Notes |
|------|--------|-------|
| CI pipeline (GitHub Actions) | M | Build + test + lint on push/PR |
| Docker CI test | M | `docker build .` as part of CI |
| `pnet_packet` replacement | M | Drop manual headers, reduce unsafe |
| Parallel test execution | L | tokio::join inside orchestrator loop |
| IPv6 e2e tests | S | Add dual-stack loopback scenarios |
| ICMP in CI | S | Docker `--cap-add=NET_RAW` or rootless ICMP |
| Additional protocols | M | HTTP, SNMP, NTP, etc. (see TESTING.MD) |
| `--rate` / `--workers` CLI | M | Thundering herd mitigation for parallel runs |
