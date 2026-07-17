# bimap demo lab — containerlab

Two-host topology with a Linux middlebox running nftables, designed to showcase
bimap's ability to **reveal** a firewall policy through a single forward scan.
The firewall is also pre-configured asymmetrically (different rules per
direction), reserved for a future `--bidir` demo of bimap.

```
 10.0.0.0/24                       10.0.1.0/24
 client:eth1 ──── firewall:eth1     firewall:eth2 ──── server:eth1
 10.0.0.2/24      10.0.0.1/24       10.0.1.1/24        10.0.1.2/24
```

- `client` and `server` are `debian:12-slim` containers; the host-built
  `target/release/bimap` binary is bind-mounted into both (`binds:`, see
  `bimap.clab.yml`).
- `firewall` is a `debian:12-slim` container with `NET_ADMIN` and IPv4
  forwarding enabled. Its nftables ruleset is installed at deploy time via
  containerlab's `exec:` hook.
- The bimap **control channel** runs on `tcp/4242` and sits outside the
  tested port range; the firewall leaves it always-open.

## The four-quadrant firewall matrix

The firewall policy is intentionally invisible to the client — that's the whole
point of using bimap. One forward scan of `tcp/1-200` reveals part of it; the
full asymmetric policy is reserved for the future `--bidir` demo (see the
"Why bidir?" section below).

| Sub-range        | `→` client→server | `←` server→client | Real-world meaning |
|------------------|:-----------------:|:-----------------:|--------------------|
| `tcp/1-50`       | allow             | allow             | Public service open both ways |
| `tcp/51-100`     | allow             | drop              | Server publishes, client pulls only |
| `tcp/101-150`    | drop              | allow             | Server pushes only (outbound policy) |
| `tcp/151-200`    | drop              | drop              | Fully closed (admin / dangerous) |

`tcp/4242` (bimap control channel) is outside this range and always allowed.

In a forward-only (`→`) scan you see two clusters of outcomes:

- `tcp/1-100`   → **all pass**   (forward direction allowed)
- `tcp/101-200` → **all fail**   (forward direction dropped)

The asymmetry lives in the firewall's `←` rules, which `--bidir` would
reveal — see "Why bidir?".

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
# 1. Build bimap once (the lab bind-mounts this binary into every container).
cargo build --release

# 2. Launch the lab + tmux panes in one go.
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

Forward-only — runs in seconds.

### 1. Server pane — start bimap server (note the fingerprint)

```sh
bimap server --bind 0.0.0.0:4242
# expect on stderr:
#   fingerprint: SHA256:<64 hex chars>
#   listening on 0.0.0.0:4242
```

Copy the fingerprint — the client uses it next.

### 2. Client pane — fire one forward scan over the matrix

```sh
bimap client --control-server 10.0.1.2:4242 \
  --fingerprint SHA256:<paste-the-fingerprint> \
  --test 1kb --port-range tcp/1-200 \
  -q
```

What you should see (with `-q`, only failures):

- `tcp/1-100` silent (all 100 pass in the forward direction)
- `tcp/101-200` printed as 100 fail lines (forward direction dropped)

That's 100 success / 100 fail across two clean ranges. Exit code 1.

### 3. Firewall pane — show the rule that bimap just revealed

```sh
nft list chain inet bimap forward_chain
```

You'll see four `counter drop` rules — two per direction. The forward scan
in step 2 only exercised two of them (the `→` drops on `tcp/101-150` and
`tcp/151-200`). The other two (`←` drops on `tcp/51-100` and `tcp/151-200`)
are invisible until `--bidir` works cross-host — see "Why bidir?" below.

### 4. Live policy flip — close the "public service" range forward

Insert a new rule at the top that drops `tcp/1-50` in the client→server
direction, then re-run bimap and watch the matrix change in real time:

```sh
nft 'insert rule inet bimap forward_chain ip saddr 10.0.0.0/24 ip daddr 10.0.1.0/24 tcp dport 1-50 counter drop'
```

In the client pane, re-run the same bimap command:

```sh
bimap client --control-server 10.0.1.2:4242 \
  --fingerprint SHA256:<fp> \
  --test 1kb --port-range tcp/1-200 -q
```

Now `tcp/1-50` also fails forward, so you see 150 fail lines and 50 pass
lines (`tcp/51-100` still passes forward). The whole `tcp/1-100` block is
no longer uniformly green.

### 5. Restore the original policy

Find the rule you just inserted and delete it by handle:

```sh
nft -a list chain inet bimap forward_chain | grep 'tcp dport 1-50'
# Pick the handle of the insert-on-top rule (the one with saddr 10.0.0.0/24
# and dport 1-50 — the original rules cover 101-150/151-200, not 1-50).
# Suppose handle is 14:
nft 'delete rule inet bimap forward_chain handle 14'

# Re-run bimap — 1-100 green forward, 101-200 red, as in step 2.
```

### 6. Tear down

```sh
# detach from tmux with Ctrl-B d; then:
containerlab destroy -t labs/clab/bimap.clab.yml --cleanup
tmux kill-session -t bimap-demo 2>/dev/null || true
```

## Why bidir?

The firewall matrix above is intentionally **asymmetric** — a real-world
pattern that `bimap --bidir` is specifically designed to surface. With
`--bidir` cross-host working, a single scan of `tcp/1-200` would print four
clean quadrants, one per row of the matrix table:

- `tcp/1-50`     `→` pass, `←` pass    (open both ways)
- `tcp/51-100`   `→` pass, `←` fail   (inbound publishing only)
- `tcp/101-150`  `→` fail, `←` pass   (outbound only)
- `tcp/151-200`  `→` fail, `←` fail   (fully closed)

### Why isn't `--bidir` in the asciinema today?

bimap's `--bidir` mode cross-host is currently broken: the `--target` CLI
flag is overloaded as both the connect address (for `→`) and the listen
address (for `←`) on the client side, and as both the bind address (for `→`)
and connect-back address (for `←`) on the server side (`src/orchestrator.rs:580`,
`src/orchestrator.rs:92,130-131`). On any non-loopback topology with two
different IPs at each end, one of bind/connect is guaranteed to fail because
the two directions need different addresses and bimap only sends one.

Workaround: this demo presents the forward direction only. The firewall's
asymmetric rules are kept in place so the future `--bidir` story works the
moment bimap core is patched (likely by sending a separate `client_addr` in
the `Configure` message, plus a `--bind-listen <IP>` CLI flag — see
`src/control/msg.rs:24-30` for the message struct and `src/cli.rs:38-40`
for the existing `--target` flag).

The matrix topology itself is unchanged: once the bimap core fix lands, the
exact same `bimap.clab.yml` will support the full bidir demo by swapping
`bimap client … --bidir …` in step 2 of the demo flow.

## Files

| File | Purpose |
|---|---|
| `bimap.clab.yml` | containerlab topology — three Debian containers, veth links, nftables rules baked into the firewall's startup `exec:`. |
| `demo.tmux.sh` | deploys the lab if needed, then opens a 3-pane tmux session attached to the three containers. |
| `README.md` | this file. |

## Re-iteration loop

Because `bimap` is bind-mounted into the containers, a sources change only
needs:

```sh
cargo build --release
# then re-run the bimap client/server commands in the existing tmux panes.
```

Re-deploy the lab itself (e.g. after editing `bimap.clab.yml`) with:

```sh
containerlab destroy -t labs/clab/bimap.clab.yml --cleanup
containerlab deploy  -t labs/clab/bimap.clab.yml
```

## Restarting bimap inside a container

`debian:12-slim` doesn't ship `pkill`/`pgrep`/`killall`. To stop a bimap
process running in the foreground inside a tmux pane, just press `Ctrl-C`.
To kill a bimap backgrounded with `&` from a shell, use `kill %1` or
`kill -9 $(docker exec server pgrep bimap)` (after `apt-get install procps`
if you want `pgrep`). The simplest reset during a demo is to `Ctrl-C` the
foreground bimap in the pane and start it again — `SO_REUSEADDR` is set on
the control listener so there is no `TIME_WAIT` contention on port 4242.

## Notes / caveats

- `target/release/bimap` is built **dynamically against glibc** on the host.
  For this reason all three containers use `debian:12-slim` and not Alpine
  (musl). If you want Alpine, build bimap with
  `cargo build --release --target x86_64-unknown-linux-musl` after
  `rustup target add x86_64-unknown-linux-musl`.
- The lab uses **stateless** nftables rules (no conntrack) so per-direction
  drops survive container restarts and stay readable in the asciinema output.
- The `sleep 8` inside `demo.tmux.sh` lets the firewall finish
  `apt-get install nftables` post-deploy. On a slow first run you may need to
  bump it; subsequent deploys keep nftables cached in the image layer.
- bimap's default control channel port is 443. We use **4242** to avoid
  needing `CAP_NET_BIND_SERVICE` inside the container — keeps the demo
  unprivileged and mirrors what most real deployments will do when not
  running as root.