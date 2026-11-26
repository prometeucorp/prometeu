# Claude usage fixture

`usage.ndjson` is a synthetic protocol scenario without content. It agrees with
the [official cost and usage contract](https://code.claude.com/docs/en/agent-sdk/cost-tracking),
which we examined on 2026-10-01. It is not a recording. It does not prove that a
live CLI agrees with the contract.

The first turn sets cumulative counters for each model. The second turn adds
usage of the primary agent and a child model that was not in the first turn.
The tests examine these items:

- the input totals, which include the cache
- the differences for each model
- the context of the primary agent, which the adapter keeps apart
- the identity of the reply

More adapter tests examine these conditions:

- totals that the adapter loads again
- resets
- crash results with zero values
- duplicate messages and results
- child work that goes across turn boundaries
