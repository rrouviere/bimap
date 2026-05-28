# Usage — Bimap

```
bimap --server [options]
bimap --client <server-ip> [options]
```

## Server options

```
--bind <addr>         bind address (default 0.0.0.0:443)
--timeout <seconds>   idle timeout before shutdown (default 60)
```

## Client options

```
--server <ip>             server address (required)
--port <port>             control port (default 443)
--test <name>             test to run, repeatable for multiple
--port-range <spec>       port range to probe, repeatable
--bidir                   run all tests in reverse direction
--timeout <ms>            per-test timeout (default 5000)
--fingerprint <hash>      trust this SHA-256 fingerprint, skip prompt
--json                    output one JSON object per result line
-q, --quiet               suppress passing tests from output
-v, --verbose             debug output including timing and raw hex
-h, --help                print help
--version                 print version
```

## Tests

| Name | Layer | Transport | Description | Root |
|---|---|---|---|---|
| `icmp-ping` | L3 | ICMP | Echo request → reply | yes |
| `icmp-full` | L3 | ICMP | All known ICMP types/subtypes | yes |
| `open` | L4 | TCP/UDP | Connect + send 1 byte, verify delivery | no |
| `1kb` | L4 | TCP/UDP | Connect + send 1 KB, verify integrity | no |
| `tls` | L7 | TCP | TLS handshake + 1 KB data transfer | no |
| `dns` | L7 | TCP/UDP | DNS query, verify valid response | no |

## Port range syntax

```
--port-range tcp/1-1024
--port-range udp/8000-8999
--port-range icmp/any          # ICMP ignores ports, this just enables ICMP tests
```

## Examples

```sh
# Server
$ sudo bimap --server
[bimap] listening on 0.0.0.0:443
[bimap] fingerprint: SHA256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08

# Client: TCP port scan 1–1024
$ bimap --client 10.0.0.2 --test open --port-range tcp/1-1024
 open  tcp/22   pass  1/1B
 open  tcp/80   pass  1/1B
 open  tcp/443  pass  1/1B
 open  tcp/999  fail  timeout  1/0B
---
  passed: 3  failed: 1

# DNS + bidirectional
$ bimap --client 10.0.0.2 --test dns --port-range tcp/53 --port-range udp/53 --bidir
 dns  udp/53  ->  pass  29/45B
 dns  tcp/53  ->  pass  29/45B
 dns  udp/53  <-  pass  29/45B
 dns  tcp/53  <-  pass  29/45B
---
  passed: 4  failed: 0

# Full ICMP sweep (root required)
$ sudo bimap --client 10.0.0.2 --test icmp-full --port-range icmp/any
 icmp  type=8  pass  echo-reply
 icmp  type=13 pass  timestamp-reply
 icmp  type=17 fail  no-reply
---
  passed: 9  failed: 5

# JSON output for scripting
$ bimap --client 10.0.0.2 --test 1kb --port-range tcp/8080 --json
{"test":"1kb","transport":"tcp","port":8080,"direction":"->","result":"pass","sent":1024,"received":1024}
```

## Exit codes

| Code | Meaning |
|---|---|
| 0 | All tests passed |
| 1 | At least one test failed |
| 2 | Configuration error (bad args) |
| 3 | Connection error (cannot reach server, lost control channel) |
