# Bimap - Bidirectionnal network mapper

Bimap is like nmap, but with a collaborating client in the other side of the network.
It can be used for firewall rule enumeration and middlebox characterisation.

Bimap requires the client to be able to communicate with the server over https (tcp/443) to check whether the data received at one end is the same as what was sent.
This is a current limitation.

<a href="https://asciinema.org/a/BMMyEkpRd3IIh0Ez"><img src="https://asciinema.org/a/BMMyEkpRd3IIh0Ez.svg" alt="bimap demo: discovering firewall policies with a forward scan" width="100%" /></a>

## Features
Bimap is composed of tests suites. Each test has a client and server component and the result is checked over the control connection

### Layer 3
- ICMP test between 2 IPs
  - `icmp-ping`
  -  ̀icmp-full`: Send the all availables 
### Layer 4 port testing
- Port enumeration between 2 IPs
  - `open`: Open a connection (if tcp, else just) and send one packet of data
  - `1kb`: Transfer 1kb
### Layer 7
- `tls`: Start an TLS connection and transfer 1kb 
- `dns`: Send a DNS query

## Usage

First start the server-side: `bimap --server`.
Then you can point the client to it and select the tests:
- `bimap --client <Server IP> --test=1kb --port=tcp/0-1024 --port=udp/8000-8999`
- `bimap --client <Server IP> --test=dns --port=tcp/53 --port=udp/53`
- `bimap --client <Server IP> --test=tls --port=tcp/53 --port=udp/53`

Note that the `--bidir` option allows to run all tests in reverse.

## Architecture

Bimap is a single binary that can run in two modes:
- Client
- Server

Note that this does not imply a client-server architecture
 
