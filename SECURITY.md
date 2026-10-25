# Security policy

## Supported versions

Please use the [latest release](https://github.com/prometeucorp/prometeu/releases/latest).
Security fixes target the latest release; older releases do not have a separate
maintenance branch. Windows support is experimental.

## Report a vulnerability privately

Use GitHub's [private vulnerability reporting](https://github.com/prometeucorp/prometeu/security/advisories/new)
to contact the maintainers. Do not open a public issue or pull request with an
unpatched vulnerability, credentials or private conversation content.

Include the affected Prometeu version, operating system, agent CLI version when
relevant, reproduction steps, expected impact and a minimal example with secrets
removed. Keep any sensitive proof of concept in the private report.

Maintainers will review the report, discuss impact and remediation privately,
and coordinate disclosure with the reporter. A response deadline is not guaranteed.

## Security boundaries

Agents run locally with your user's file and process permissions. Worktrees
isolate Git changes; they are not security sandboxes. Only use projects, scripts,
plugins and MCP servers you trust. Model providers receive task content through
their CLIs.

Optional shared conversations use end-to-end encryption with trust on first
contact and automatic adoption of changed keys, without forward secrecy.
See the [architecture](ARCHITECTURE.md),
[encryption limits](docs/decisions/0022-end-to-end-encryption.md) and
[privacy information](README.md#privacy) before sharing sensitive content.
