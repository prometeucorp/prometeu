# Prometeu documentation

`docs/` is the repository's knowledge bundle. Standing instructions remain in
[`AGENTS.md`](../AGENTS.md); the product and system maps remain at the root.
When the location is unknown, choose an area, read its index, then open the
relevant concept and follow its dependencies. Known paths can be opened directly.

- [Architecture](architecture/README.md) — System flows, responsibilities and dependency rules.
- [Contracts](contracts/README.md) — Formats and guarantees that cross process or layer boundaries.
- [Decisions](decisions/README.md) — Implemented architectural choices, rationale and separately listed proposals.
- [Quality](quality/README.md) — Provider support, verification coverage and measurement protocols.
- [Operations](operations/README.md) — Development, contribution, CI and release procedures.

## Update rule

A change must update the document that answers the affected question:

- visible behavior: `README.md` or the feature documentation;
- responsibility or flow: architecture;
- format between layers: contract;
- choice and trade-offs: ADR;
- support per agent: the provider matrix;
- build, test or publication: operations.

Every concept has YAML `type`, `title`, `description` and `status`, and a line
in its folder's `README.md`. Folder indexes are navigation, not concepts; see
the [format and decision lifecycle](decisions/README.md).

List `draft` proposals separately from current guidance. They do not describe
the current system until implemented. Finished implementation plans become
contracts or ADRs, or leave the tree; Git keeps their history.
