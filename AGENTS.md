# Contributing to Bimap

For user-facing changes, read the [README](README.MD). For capability planning,
read the [roadmap](ROADMAP.MD).

- Preserve useful error reporting. Network failures should produce a result or
  propagated error rather than a panic or silently discarded error.
- For behavior changes, add regression coverage that reproduces the problem.
  Change assertions only when the intended behavior changes, and explain why.
- Validate affected network flows with real client/server processes. Use the
  [firewall lab](labs/clab/README.md) when a change depends on filtering.
- Keep new dependencies justified by the capability they provide; reuse existing
  protocol libraries where suitable.

## Checks

Use the [CI workflow](.github/workflows/ci.yml) and
[pre-commit hook](.githooks/pre-commit) for the required checks.

Enable the hook locally:

```sh
git config core.hooksPath .githooks
```

Raw-ICMP tests are ignored by default. Run relevant ignored tests as root
when changing ICMP behavior.
