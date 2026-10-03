# Claude usage fixture

`usage.ndjson` is a synthetic, content-free protocol scenario based on the
[official cost and usage contract](https://code.claude.com/docs/en/agent-sdk/cost-tracking),
checked on 2026-10-01. It is not a recording or a live CLI conformance claim.

The first turn establishes cumulative model counters. The second adds main-agent
usage and a child model that was absent from the baseline. Assertions check
inclusive input totals, model deltas, separate main context, and reply identity.
Additional adapter tests cover restored totals, resets, zeroed crash results,
duplicate messages/results, and child work crossing turn boundaries.
