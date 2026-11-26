# Prometeu documentation

The `docs/` folder is the knowledge bundle of the repository. The permanent
instructions stay in [`AGENTS.md`](../AGENTS.md). The maps of the product and of
the system stay at the root of the repository.

If you do not know the location of a subject, do these steps:

1. Select an area.
2. Read the index of that area.
3. Open the applicable concept.
4. Follow the dependencies of that concept.

If you know the path, open the file directly.

- [Architecture](architecture/README.md) — The flows of the system, the responsibilities and the dependency rules.
- [Contracts](contracts/README.md) — The formats and guarantees that go across the limits of processes or layers.
- [Decisions](decisions/README.md) — The architecture decisions that are in the code and their causes. Proposals are in a different list.
- [Quality](quality/README.md) — The support for each provider, the coverage of the checks and the measurement protocols.
- [Operations](operations/README.md) — The procedures for development, contribution, CI and release.

## Update rule

A change must update the document that answers the question that the change
is about:

- Behavior that a person can see: `README.md` or the documentation of the feature.
- Responsibility or flow: the architecture documents.
- Format between layers: a contract.
- Decision and trade-offs: an ADR.
- Support for each agent: the provider matrix.
- Build, test or publication: the operations documents.

Each concept has the YAML fields `type`, `title`, `description` and `status`.
Each concept also has a line in the `README.md` file of its folder. The folder
indexes are for navigation. They are not concepts. For more data, refer to the
[format and decision lifecycle](decisions/README.md).

Put the `draft` proposals in a list that is different from the current
guidance. A proposal does not give the current system until a person
puts it into the code. When an implementation plan is complete, it becomes a contract
or an ADR, or you remove it from the tree. Git keeps its history.
