# Contributing to Prometeu

We accept bug reports, changes to the documentation and changes to the code.
Before you open a report, examine the
[current issues](https://github.com/prometeucorp/prometeu/issues). Include these
items in a report:

- the steps that make the problem occur again
- the result that you thought was correct and the result that you got
- the operating system and the version of Prometeu
- the version of the agent CLI, if it is applicable

A public report must not contain credentials or private conversations.

## Make a change

1. Fork the repository and make a branch from `main`.
2. Install the dependencies as the [development guide](docs/operations/development.md)
   tells you.
3. Run the app or the browser mock as the development guide tells you.
4. Read the [architecture map](ARCHITECTURE.md) and the contract for your area.
5. For a change to a provider, IPC, collaboration or the Cloud, use the
   [contributor recipes](docs/operations/contributing.md).
6. Make the change small and keep it on one subject.
7. Update the contracts and tests that the change touches in the same PR.
8. If you change an architectural decision, obey the
   [decision lifecycle](docs/decisions/README.md).
9. If you add or expand browser tests, obey the
   [E2E scope policy](docs/operations/development.md#e2e-scope).
10. Run the tests that are nearest to the change first.
11. For a PR that changes code, then run `npm run check`.
12. If you cannot run a check, tell which check and why in the PR.
13. For a native change, also do a check on the desktop. Browser tests use the
    mock.
14. Open a PR against `main`. Give the problem, the behavior after the change,
    the validation and the limits that stay.
15. If there is a related issue, add a link to it.

Write comments and documentation in English. Write technical documentation in
ASD-STE100 Simplified Technical English, Issue 9, as the
[technical writing guide](docs/operations/technical-writing.md) tells you. Add
interface text to the two i18n catalogs. Use the
[design system](docs/architecture/design-system.md) again for controls.

Commits obey [Conventional Commits](docs/operations/release.md). The git hook of
the repository and CI validate them. The short [agent guide](AGENTS.md) applies
the same boundaries to automated contributions.

## Community and security

The
[good first issues](https://github.com/prometeucorp/prometeu/issues?q=is%3Aissue%20is%3Aopen%20label%3A%22good%20first%20issue%22)
are small tasks that are good for a start. Report a vulnerability privately as
[SECURITY.md](SECURITY.md) tells you. Do not report it in a public issue.
