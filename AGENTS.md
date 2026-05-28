# Bimap — AI Agent Instructions

## Build & Test

```bash
cargo fmt -- --check      # format check
cargo clippy --all-targets -- -D warnings  # lint gate
cargo test --lib          # unit tests (<2s)
cargo test --test integration  # integration tests (<10s)
cargo build --release     # required before E2E
cargo test --test e2e     # E2E tests (<30s)
cargo test --test fault   # fault injection tests (<15s)
cargo test                # full suite (unit only, skips E2E/fault/integration)
```

Pre-commit hook runs: `fmt → clippy → unit → build → integration → e2e → fault`.

## Project conventions

- **No unwrap().** Use `?` for error propagation, `.context("...")?` for context.
- **No silently discarded errors.** Never `let _ = fallible_call()`. Use `?` or explicit `match`.
- **No panic in network code paths.** Every I/O error maps to a structured `FailReason` or `ErrorReason`.
- **No abbreviations in public names.** Use `connection`, not `conn`. `received`, not `recv`.
- **Files named after the struct they contain.** `port.rs` exports `OpenTest` + `KbTest`, not `port_test.rs`.
- **Comment only the "why", never the "what".** Code should be self-documenting.

## Architecture (tl;dr)

```
cli.rs ─> main.rs ─┬─> control/mod.rs (TLS channel, JSON messages)
                    ├─> orchestrator.rs (test scheduling, result comparison)
                    └─> test/mod.rs (TestProtocol trait + registry)
                          ├── port.rs   (open + 1kb, L4 TCP/UDP)
                          ├── icmp.rs   (icmp-ping + icmp-full, L3)
                          ├── tls_test.rs (TLS handshake + 1KB, L7)
                          └── dns.rs    (DNS query/response, L7)
                    └─> packet/*.rs (manual header structs: ip, tcp, udp, icmp, dns)
```

See `ARCHI.MD` for full architecture. See `USAGE.MD` for CLI reference.
See `TESTING.MD` for test strategy and coverage matrix.
See `ROADMAP.MD` for phase dependencies and parallelization plan.

## Extension flow

Adding a new protocol:
1. `src/test/myproto.rs` — implement `TestProtocol` trait
2. Register in `src/test/mod.rs` registry vector
3. If raw packet needed, add header struct in `src/packet/`
4. Write tests first (see TESTING.MD)

## Testing rules (TDD)

1. Never weaken a failing test assertion to make it pass.
2. Test names: `<protocol>_<transport>_<scenario>_<expected>`.
3. No `#[should_panic]` — if code panics, it's a production bug.
4. L1 integration tests use ports 10000–19999 on localhost.
5. ICMP tests are `#[ignore = "root_required"]` + runtime `geteuid() == 0` guard.

## Dependency budget

7 crates. Do NOT add new deps without updating ARCHI.MD and justifying in commit message.

```
clap, tokio, tokio-rustls, rustls, rcgen, serde, serde_json, socket2
```

All packet headers (IP, TCP, UDP, ICMP, DNS) are manual structs — no `pnet`, no `hickory-dns`.

## Gotchas

- Control messages are newline-delimited JSON. One JSON object per line. Framing matters.
- Server generates ephemeral cert each run. No persisted keys. Fingerprint printed to stderr.
- `-q` means stdout shows ONLY failures. `--json` outputs one JSON line per result.
- Exit codes: 0=all pass, 1=any fail, 2=config error, 3=connection error.
- `--bidir` doubles the test count: each test runs in both directions.
- ICMP tests require root. Skip with `#[ignore]` in test config.
