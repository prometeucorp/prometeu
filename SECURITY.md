# Security policy

## Supported versions

Use the [latest release](https://github.com/prometeucorp/prometeu/releases/latest).
Repairs for vulnerabilities go only into the latest release. Older releases do
not get repairs on a maintenance branch.

## Report a vulnerability privately

To speak to the maintainers, use the
[private vulnerability reporting](https://github.com/prometeucorp/prometeu/security/advisories/new)
of GitHub. Do not open a public issue or pull request that contains one of these
items:

- a vulnerability that has no repair at this time
- credentials
- the private content of a conversation

Include these items in the report:

- the version of Prometeu that has the problem
- the operating system
- the version of the agent CLI, if it is applicable
- the steps that make the problem occur again
- the possible effect
- a minimum example without secrets

Keep each sensitive proof of concept in the private report.

The maintainers will examine the report. They will discuss the effect and the
repair privately with you. They will also agree with you on the time to publish
the vulnerability. We do not promise a time limit for the response.

## Security boundaries

Agents run on your computer with the file and process permissions of your user.
Worktrees isolate the changes in Git, but they are not sandboxes. Use
only projects, scripts, plugins and MCP servers that you trust. Model providers
get the content of the task through their CLIs.

Shared conversations are optional. They use end-to-end encryption with trust on
first contact and they use changed keys automatically. There is no forward
secrecy. Before you share sensitive content, read the
[architecture](ARCHITECTURE.md), the
[limits of the encryption](docs/decisions/0022-end-to-end-encryption.md) and the
[privacy information](README.md#privacy).
