# Architecture — Bimap

## Language & Runtime

Rust, single binary. Async via `tokio`.

## Dependency budget (12 crates)

```
clap              CLI
tokio             async runtime, TCP/UDP I/O, timers
tokio-rustls      TLS over tokio streams
rustls            TLS engine (control channel + TLS test)
rcgen             ephemeral self-signed cert generation
serde             serialization derive
serde_json        JSON control messages
socket2           socket options
libc              raw syscalls for ICMP socket I/O
sha2              SHA-256 hashing for data integrity verification
async-trait       async trait method support
hickory-proto     DNS protocol structures (RFC-compliant)
```

DNS uses `hickory-proto` for RFC-compliant wire format.
IP, TCP, UDP, ICMP headers use `pnet_packet` crate (migrated from manual C-style structs in 0.2.0).

## Binary layout

```
src/
  main.rs              entry point, mode dispatch (--server | --client)
  cli.rs               clap arg definitions
  control/
    mod.rs             ControlChannel struct — TLS connect/listen, message send/recv
    tls.rs             ephemeral cert generation (rcgen), fingerprint print
    msg.rs             serde-annotated message types, newline-delimited framing
  orchestrator.rs      test scheduling, execution, result comparison
  test/
    mod.rs             TestProtocol trait, TestRegistry, protocol enumeration
    icmp.rs            ICMP echo / icmp-full (L3), uses libc for raw socket I/O
    port.rs            open + 1kb transfer (L4, TCP/UDP via tokio)
    tls_test.rs        TLS handshake + 1kb data transfer (L7, via rustls)
    dns.rs             DNS query/response via TCP & UDP (L7)
  packet/
    mod.rs             ICMP type constants + type_name helper
    icmp.rs            ICMP type table (ALL_ICMP_TYPES), type_name()
    (IP, TCP, UDP headers removed in 0.2.0 — pnet_packet handles these)
```

IP, TCP, UDP, ICMP headers use `pnet_packet` crate (migrated from manual C-style structs in 0.2.0).
DNS uses `hickory-proto` for RFC-compliant wire format.

## Control protocol

JSON over TLS on TCP/443. One JSON object per line (newline-delimited).

### Certificates
- Server generates an ephemeral self-signed certificate at startup.
- Prints SHA-256 fingerprint to stderr and stdout.
- Client receives the cert, prints its fingerprint, prompts for trust (or `--fingerprint <hash>`).
- No CA, no persisted keys. Security is out of scope for v0.

### Message types

```
→ Client to Server
← Server to Client
```

```
←  {"type":"hello","version":1,"fingerprint":"sha256:..."}

→  {"type":"configure","tests":["open","1kb","dns"],
    "port_ranges":[{"transport":"tcp","start":1,"end":1024}],
    "bidir":false}
←  {"type":"ack","ok":true}

→  {"type":"test","id":0,"protocol":"open","transport":"tcp",
    "port":22,"direction":"->"}
←  {"type":"report","id":0,
    "sent":{"bytes":1,"sha256":"a1b2..."},
    "received":{"bytes":1,"sha256":"a1b2..."}}

→  {"type":"test","id":1,"protocol":"1kb","transport":"tcp",
    "port":80,"direction":"<-"}
←  {"type":"report","id":1,
    "sent":{"bytes":1024,"sha256":"e4d5..."},
    "received":{"bytes":1024,"sha256":"e4d5..."}}

→  {"type":"done"}
←  {"type":"bye","summary":{"passed":2,"failed":0}}
```

### Direction encoding
- `"->"` = client initiates traffic to server (normal mode)
- `"<-"` = server initiates traffic to client (bidir mode)

### Test flow per (protocol, transport, port, direction)

1. Orchestrator sends `test` message.
2. The **initiator** binds/connects and sends test data.
3. The **target** binds/listens and receives what arrives.
4. Both sides — independently — measure bytes sent/received + SHA-256 of payload.
5. Both send `report` messages back to the orchestrator.
6. Orchestrator compares `sent.sha256 == received.sha256` — pass if equal.
7. If neither side could bind/connect, report as `error` (skip, continue).

Timeouts are per-test (`--timeout` ms). A timeout is recorded as `fail:timeout` and the next test proceeds.

### Error handling
Every possible network failure is caught and logged — no panics.
Connection refused, timeout, partial reads, ECONNRESET, EADDRINUSE, EACCES — all mapped to a structured `FailReason` or `ErrorReason` enum.

## TestProtocol trait

```rust
#[async_trait]
pub trait TestProtocol: Send + Sync {
    fn name(&self) -> &'static str;
    fn layer(&self) -> Layer;
    fn transports(&self) -> &[Transport];

    async fn run(&self, ctx: TestCtx) -> TestOutcome;
}

pub struct TestCtx {
    pub direction: Direction,
    pub transport: Transport,
    pub port: u16,
    pub local_addr: SocketAddr,    // our side
    pub remote_addr: SocketAddr,   // peer side
    pub timeout: Duration,
}

pub enum TestOutcome {
    Pass { sent: u64, received: u64, sha256: [u8; 32] },
    Fail { reason: FailReason, sent: u64, received: u64 },
    Error { reason: ErrorReason },
}

pub enum FailReason { Timeout, Refused, Reset, Mismatch, NoData, ... }
pub enum ErrorReason { Bind(EADDRINUSE), Permission, Internal, ... }
```

## Protocol specifics

### ICMP
- Raw socket via `libc` (no `socket2` needed for raw I/O).
- Requires root or `CAP_NET_RAW`.
- `icmp-ping`: send echo request (type 8), wait for echo reply (type 0).
- `icmp-full`: iterate all known ICMP types/subtypes (0–18, 30, 31, 33–35, 37, 38, 40, 42), send each, log any reply.
- ICMP is stateless at the server — kernel handles echo reply. Server side only reports what it observed on the wire.
- ICMP header structure from `pnet_packet::icmp`.

### Port (open / 1kb)
- `open`: TCP `connect()` → send 1-byte payload → read echo → verify. UDP `sendto()` 1 byte → no receive required.
- `1kb`: same flow with a 1024-byte payload.
- Server side binds, reads, echoes back the received bytes.
- TCP/UDP packet structures from `pnet_packet::tcp` / `pnet_packet::udp`.

### TLS
- Server binds TCP, wraps with `rustls::ServerConnection`.
- Client connects, wraps with `rustls::ClientConnection` (accepts unknown CA for ephemeral test).
- Client sends 1 KB over the TLS stream; server reads and echoes.
- Verifies data integrity end-to-end.
- No client or server certificate validation (test mode). Cert mismatch is recorded as `fail:tls-handshake`.

### DNS
- DNS protocol via `hickory-proto` — full RFC-compliant message encoding/decoding.
- Server runs a minimal DNS responder that echoes queries or returns known records.
- Client sends standard DNS query (A record for `bimap.test.`), verifies response structure.
- Supports TCP/53 (RFC 1035 length prefix) and UDP/53.
- Severed/truncated response → `fail:dns-malformed` (detected by `hickory-proto` parsing).

## Extension guide

1. `src/test/myproto.rs` — implement `TestProtocol`.
2. Register in `src/test/mod.rs` registry vector.
3. Use existing crate for protocol headers where possible (pnet, hickory-proto, etc.).
4. Rebuild. No other files touched.

No new dependencies unless the protocol has an exotic wire format requiring non-trivial parsing. Prefer existing crates over manual implementations — networking code is hard.

## Testing strategy

### Principle
Log everything, crash never. Every network outcome is `pass` | `fail(reason)` | `error(reason)`.
All protocol parsing is defensive. Truncated, malformed, or unexpected data → logged, not panicked.
Each test runs independently; one port failure does not abort the session.

### Coverage matrix (excerpt: `open` + TCP)

| Case | Client observes | Server observes | Outcome |
|---|---|---|---|
| success | connected, sent 1B | received 1B | pass |
| timeout | ETIMEDOUT | no connection | fail:timeout |
| refused | ECONNREFUSED | port not bound | fail:refused |
| reset | connected, then RST | accepted, then RST | fail:reset |
| partial | sent 1B | recv 0B | fail:mismatch |
| bind conflict | — | EADDRINUSE | error:bind |

Same coverage exists for UDP, ICMP (no-permission, no-reply, host-unreachable), TLS (handshake-fail, cipher-mismatch), DNS (NXDOMAIN, truncation, FORMERR).

### Test layers

**1. Unit tests** (`#[cfg(test)]` in `src/`)
- Control message JSON serde roundtrip for all message types.
- Unknown fields tolerated, no panic.
- Malformed JSON → `Err`, not `panic!`.
- Newline framing: split reads, multiple messages in one buffer.
- Protocol header roundtrips via crate-provided types (pnet, hickory-proto).

**2. Integration tests** (`tests/integration.rs`)
- `loopback` — ephemeral server on localhost, full protocol cycle per test type.
- `bidir` — both directions produce correct reports.
- `concurrent` — 10 ports × 4 protocols in parallel, no cross-talk.
- `timeout` — target port not listening → timeout, not hang.
- `partial_read` — server sends fewer bytes than expected → mismatch logged.

**3. E2E tests** (`tests/e2e.rs`)
- Spawn `bimap --server`, parse fingerprint from stderr, connect client.
- `--json` outputs one JSON line per result.
- `--bidir` doubles test count.
- Exit codes: 0 = all pass, 1 = any fail, 2 = config error, 3 = connection error.
- `-q` suppresses passing tests from stdout.

**4. Fault injection** (`tests/fault.rs`)
- Truncated TLS handshake (close after ClientHello).
- Garbage data mid-test stream.
- Control channel disconnect mid-sequence → client detects, logs, exits cleanly.
- Server SIGKILL mid-session → client detects lost control, prints error, exit 3.

### Test data constants

All payloads are deterministic for verifiable SHA256:
- 1B: `0xAA`
- 1KB: `b"\x00\x01\x02...\xFF"` pattern looped
- DNS: query A `bimap.test.`
- TLS: same 1KB pattern as L4 test

### Logging

| Flag | Stdout | Stderr |
|---|---|---|
| default | pass/fail table | errors, warnings |
| `-q` | failures only | errors |
| `-v` | +timing, transfers | +debug, raw hex |

### CI notes
- ICMP tests: `#[cfg(unix)]` + check `geteuid() == 0` at runtime. Skip with warning if not root.
- All other tests run without privileges.
