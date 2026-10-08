# Bimap roadmap

Bimap's goal is to characterize how middleboxes permit and alter traffic.
The [README](README.MD) describes the tests available today.

## Phase I — L4 policy characterization, plus ICMP

Build a reliable picture of allowed transports, ports, and connection directions.

- Make TCP/UDP and ICMP observations reproducible. Resolve reported UDP
  flakiness and distinguish exchange failures from setup errors.
- Present observations as a policy map, including uncertainty where failures
  cannot establish filtering.
- Aim to output candidate rules that match observed behavior. These describe
  the tested paths and conditions; they do not recover the original configuration.

Phase I is complete when repeated tests through a known policy produce consistent,
explainable results in both directions. Existing TLS/DNS support remains
experimental.

## Phase II — Protocol-specific transformation detection

First establish an extensible API for probes, observations at both endpoints,
and comparison of what was sent with what arrived. It must support new protocols
without rebuilding the measurement and comparison machinery for each one.

Then expand detection to transformations such as TCP flag modification and HTTP
middlebox behavior. Results should describe observed changes and the conditions
that trigger them.

The API is a prerequisite for Phase II. Success means a new protocol can reuse it
to expose a known transformation in a controlled middlebox test.
