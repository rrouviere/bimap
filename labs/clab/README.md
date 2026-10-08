# Bimap firewall policy demo

This Containerlab topology puts a client and a target on opposite sides of a
stateful nftables firewall. A Bimap scan tests 1 KB exchanges on TCP ports
1–1024 and discovers that ports 22, 80, and 443 permit the exchange.
Bimap supplies the responders; no SSH, HTTP, or HTTPS service is needed.

```
client ── 10.0.0.0/24 ── firewall ── 10.0.1.0/24 ── target
```

The three tmux panes show the client scan, firewall rules and counters, and
target server.

## Requirements

- Docker
- [Containerlab](https://containerlab.dev/install/)
- tmux; asciinema for recording
- Rust toolchain

Build bimap and open the interactive three-pane lab:

```sh
cargo build --release
./labs/clab/demo.tmux.sh
```

In the target pane, press Enter to start the bimap server. Then press Enter in
the client pane to scan TCP ports 1–1024. The firewall pane is ready to show
the active rules. Detach with `Ctrl-B d`; stop the topology with:

```sh
containerlab destroy -t labs/clab/bimap.clab.yml
```

## Record the demo

Generate the checked-in asciicast from a live run:

```sh
./labs/clab/demo-record.sh
```

The script runs the scan and tears down the lab if it deployed it.
Pass a path to save a separate recording:

```sh
./labs/clab/demo-record.sh /tmp/bimap-demo.cast
```

![Bimap scanning TCP ports 1–1024 through a firewall](demo.gif)

Source: [demo.cast](demo.cast). Video: [demo.mp4](demo.mp4).

After recording, regenerate both media files with
[agg](https://docs.asciinema.org/manual/agg/installation/), FFmpeg, and Python 3:

```sh
./labs/clab/demo-render.sh
```

## Firewall policy

| Direction | Rule |
|---|---|
| Any | Drop invalid connections |
| Any | Allow established and related flows |
| Any | Allow the bimap control channel on TCP 4242 |
| Client → target | Allow TCP 22, 80, 443, and 8080 |
| Target → client | Allow all TCP |
| Otherwise | Drop |

The recorded scan covers ports 1–1024 and discovers TCP 22, 80, and 443. The
firewall permits the control channel separately from the tested range.
