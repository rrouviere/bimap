# bimap demo lab — containerlab

Two-host topology with a Linux middlebox running nftables, designed to showcase
bimap's ability to **reveal** a firewall policy through a single forward scan.

```
 10.0.0.0/24                       10.0.1.0/24
 client:eth1 ──── firewall:eth1     firewall:eth2 ──── server:eth1
 10.0.0.2/24      10.0.0.1/24       10.0.1.1/24        10.0.1.2/24
```

- `client` and `server` are `debian:12-slim` containers; the host-built
  `target/release/bimap` binary is bind-mounted into both.
- `firewall` is a `debian:12-slim` container with `NET_ADMIN` and IPv4
  forwarding enabled. Its nftables ruleset is installed at deploy time.
- The bimap **control channel** runs on `tcp/4242` and sits outside the
  tested port range; the firewall leaves it always-open.

## Firewall policy

The firewall uses stateful conntrack with a per-port allowlist:

| Direction | Policy |
|---|---|
| any | Drop invalid connections |
| any | Allow established/related |
| any | Allow `tcp/4242` (control channel) |
| client→server | Allow `tcp/{22,80,443,8080}` only |
| server→client | Allow all TCP |
| default | Drop |

A forward scan of `tcp/1-200` from the client reveals exactly which ports
the firewall allows through: SSH (22) and HTTP (80). Everything else is
blocked. The asymmetry (server→client allows all TCP) is hidden from the
client — that's the whole point of using bimap.

## Prerequisites

- **Docker**
- **containerlab** — install: <https://containerlab.dev/install/>
- **tmux** (`sudo apt install tmux`)
- **Rust toolchain** to build bimap: `cargo build --release`

```sh
# Quick install (Ubuntu/Debian) — installs Docker + containerlab
curl -sL https://containerlab.dev/setup | sudo -E bash -s "all"
```

## Quick start

```sh
cargo build --release
./labs/clab/demo.tmux.sh
```

You'll land in a tmux session with three panes:

```
 ┌─────────────────────────┬──────────────────────────┐
 │ server pane             │ client pane              │
 │ bash#                   │ bash#                    │
 ├─────────────────────────┴──────────────────────────┤
 │ firewall pane                                      │
 │ bash#                                              │
 └────────────────────────────────────────────────────┘
```

## Demo flow

### 1. Server pane — start bimap server (note the fingerprint)

```sh
bimap server --bind 0.0.0.0:4242
```

Copy the fingerprint from stderr — the client needs it next.

### 2. Client pane — fire a forward scan

```sh
bimap client --control-server 10.0.1.2:4242 \
  --fingerprint SHA256:<paste-the-fingerprint> \
  --test 1kb --port-range tcp/1-200
```

What you should see:

- `tcp/22,80` pass (firewall allows SSH and HTTP)
- `tcp/1-21,23-79,81-200` fail (everything else blocked)

### 3. Firewall pane — show the rule that bimap just revealed

```sh
nft list chain inet bimap forward_chain
```

### 4. Live policy flip — block SSH forward

```sh
nft 'insert rule inet bimap forward_chain ip saddr 10.0.0.0/24 ip daddr 10.0.1.0/24 tcp dport 22 counter drop'
```

Re-run the same bimap client command. Now `tcp/22` also fails — only
`tcp/80` passes forward.

### 5. Restore the original policy

```sh
nft -a list chain inet bimap forward_chain | grep 'tcp dport 22'
# Pick the handle of the insert-on-top rule, e.g. handle 14:
nft 'delete rule inet bimap forward_chain handle 14'
```

### 6. Tear down

```sh
# detach from tmux with Ctrl-B d; then:
containerlab destroy -t labs/clab/bimap.clab.yml
tmux kill-session -t bimap-demo 2>/dev/null || true
```

## Files

| File | Purpose |
|---|---|
| `bimap.clab.yml` | containerlab topology — three Debian containers, veth links, nftables rules. |
| `demo.tmux.sh` | deploys the lab if needed, opens a 3-pane tmux session. |
| `firewall.nft` | nftables ruleset loaded into the firewall container. |
| `README.md` | this file. |

## Notes

- `target/release/bimap` is built against glibc on the host, so containers
  use `debian:12-slim` (not Alpine/musl). For musl, build with
  `cargo build --release --target x86_64-unknown-linux-musl`.
- The lab uses **stateful** nftables rules (conntrack) so that bimap's own
  TCP handshake completes — the SYN-ACK return path is allowed for
  established connections.
- `tcp/4242` is used for the control channel to avoid `CAP_NET_BIND_SERVICE`.
- `debian:12-slim` doesn't ship `pkill`. Press `Ctrl-C` to stop a foreground
  bimap process. `SO_REUSEADDR` is set so port 4242 has no `TIME_WAIT` issue.
