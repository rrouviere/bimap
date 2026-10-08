# Bimap roadmap

Bimap's goal is to characterize how middleboxes permit and alter traffic.
The [README](README.MD) describes the tests available today.

## Phase I — Policy characterization and a useful report

Build a reliable picture of which tested paths carry traffic, including TCP,
UDP, ICMP, and direction. Make repeated observations stable and distinguish test
setup errors from unsuccessful exchanges.

The CLI should end each human-readable scan with a compact summary that:

- Counts passed, failed, and errored tests.
- Groups successful observations by transport, port range, direction, and test.
  Present these as **candidate allow rules**, with wording that makes clear they
  describe tested conditions rather than the middlebox's actual configuration.
- Lists directly observed payload changes separately from connectivity results.
- Keeps timeouts and other inconclusive failures visible without calling them
  blocked ports. Explain that a failed exchange alone does not identify where or
  why traffic failed.

The same findings should be available in the JSON export. Quiet mode should hide
per-test passes while retaining the final summary; JSON-lines output should stay
usable as a stream of individual results.

Phase I is complete when repeated tests through a known policy produce consistent
results in both directions and the report states what passed, what failed, and
what can safely be inferred. TLS and DNS remain experimental until their results
are equally reproducible.

## Phase II — Explain protocol transformations

Add a reusable probe and observation interface so a protocol can compare what
each endpoint sent and received. The interface should let new probes share
collection and comparison code while preserving protocol-specific evidence.

Use it to identify transformations such as TCP flag changes and HTTP
middlebox behavior. For each finding, report the observed before/after values,
the test conditions, direction, and affected traffic. Separate measured changes
from inferred causes; do not claim which device or rule made the change without
evidence.

Phase II is complete when a new protocol can use the interface to detect a known
transformation in a controlled test and the CLI and JSON report describe that
finding consistently.
