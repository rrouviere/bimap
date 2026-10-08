# bimap firewall discovery demo

This Containerlab topology puts a client and a target on opposite sides of a
stateful nftables firewall. A bimap scan probes TCP ports 1–100 from the
client and identifies the two services the firewall permits: SSH (22) and
HTTP (80).

```
client ── 10.0.0.0/24 ── firewall ── 10.0.1.0/24 ── target
```

The demo uses three equal, vertical tmux panes labelled `client`, `firewall`,
and `target`. The target pane runs bimap's control server, the client pane
enumerates the policy, and the firewall pane shows the live nftables rules and
packet counters.

## Requirements

- Docker
- [Containerlab](https://containerlab.dev/install/)
- tmux and asciinema
- Rust toolchain

Build bimap and open the interactive three-pane lab:

```sh
cargo build --release
./labs/clab/demo.tmux.sh
```

In the target pane, press Enter to start the bimap server. Then press Enter in
the client pane to scan TCP ports 1–100. The firewall pane is ready to show the
active rules. Detach with `Ctrl-B d`; stop the topology with:

```sh
containerlab destroy -t labs/clab/bimap.clab.yml
```

## Record the demo

Generate the checked-in asciicast from a live run:

```sh
./labs/clab/demo-record.sh
```

The script deploys the topology if needed, opens the same three-pane layout,
starts the server, runs the scan, refreshes firewall counters, and tears down
the lab if it deployed it. Pass a path to save a separate recording:

```sh
./labs/clab/demo-record.sh /tmp/bimap-demo.cast
```

The [published recording](https://asciinema.org/a/MqxnpuVBERsXnZVR) is a real
terminal capture of the tmux session; the local copy is `labs/clab/demo.cast`.

## Firewall policy

| Direction | Rule |
|---|---|
| Any | Drop invalid connections |
| Any | Allow established and related flows |
| Any | Allow the bimap control channel on TCP 4242 |
| Client → target | Allow TCP 22, 80, 443, and 8080 |
| Target → client | Allow all TCP |
| Otherwise | Drop |

The recorded scan covers ports 1–100, so the client discovers TCP 22 and 80.
The firewall permits the control channel separately from the tested range.

## Files

| File | Purpose |
|---|---|
| `bimap.clab.yml` | Three-node topology and point-to-point links. |
| `demo.tmux.sh` | Interactive client, firewall, and target tmux layout. |
| `demo-record.sh` | Automated live scan and asciinema recording. |
| `demo.cast` | Latest generated asciicast. |
| `firewall.nft` | Stateful nftables policy loaded by Containerlab. |
