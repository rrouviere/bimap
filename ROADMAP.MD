# Bimap roadmap

Bimap's goal is to characterize how middleboxes permit and alter traffic.
The [README](README.MD) describes the tests available today.

## Phase I — Policy characterization and a useful report

- [ ] Make TCP, UDP, and ICMP observations repeatable in both directions.
- [ ] Distinguish setup errors from unsuccessful exchanges.
- [x] End human-readable scans with pass, fail, and error counts.
- [x] Group successful observations by test, transport, ports, and direction as
  **candidate allow rules**. State that these describe tested conditions, not
  the middlebox's actual configuration.
- [x] Report payload differences separately from connectivity results.
- [x] Keep failures visible without treating timeouts as proof of filtering or
  claiming where or why traffic failed.
- [x] Include the summary findings in JSON export; keep JSON-lines output usable
  as individual result records.
- [x] Keep the final summary visible in quiet mode while hiding per-test passes.
- [ ] Confirm consistent results through a known policy, including reverse
  connections.
- [ ] Make TLS and DNS results repeatable enough to leave experimental status.

## Phase II — Explain protocol transformations

- [ ] Define a reusable interface for probes and observations at both endpoints.
- [ ] Compare sent and received data while preserving protocol-specific evidence.
- [ ] Report each observed change with before/after values, test conditions,
  direction, and affected traffic.
- [ ] Detect TCP flag changes.
- [ ] Detect HTTP middlebox behavior.
- [ ] Keep measured changes separate from inferred causes; name a responsible
  device or rule only when the evidence supports it.
- [ ] Validate a known transformation in a controlled middlebox test and report
  it consistently in human-readable output and JSON.
