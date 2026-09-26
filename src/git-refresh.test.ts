import { expect, it } from "vitest";
import { GitRefreshPolicy, TurnSettlePolicy, changesInterval, marksInterval } from "./git-refresh";
import type { BackgroundContext } from "./background";

const power = (value: BackgroundContext["power"]): BackgroundContext =>
  ({ visible: true, focused: true, power: value, revision: 0 });

it("refreshes on workspace or repository changes but ignores ordinary board updates", () => {
  const policy = new GitRefreshPolicy();
  const first = { id: "one", repos: [{ worktree: "/repo/one", base: "main", name: "api" }] };
  expect(policy.consider(first)).toBe(true);
  expect(policy.consider({ ...first, repos: [...first.repos] })).toBe(false);
  expect(policy.consider({ ...first, repos: [...first.repos, { worktree: "/repo/two", base: "main", name: "ui" }] })).toBe(true);
  policy.clear();
  expect(policy.consider(first)).toBe(true);
});

it("pauses hidden Git fallbacks and budgets visible Changes and Files independently", () => {
  expect(changesInterval(power("ac"))).toBe(5_000);
  expect(changesInterval(power("battery"))).toBe(10_000);
  expect(marksInterval(power("ac"))).toBe(15_000);
  expect(marksInterval(power("unknown"))).toBe(30_000);
  expect(changesInterval({ ...power("ac"), visible: false })).toBeNull();
  expect(marksInterval({ ...power("ac"), focused: false })).toBeNull();
});

it("reports a settled turn once, only for the workspace that was running", () => {
  const policy = new TurnSettlePolicy();
  const tabs = (...status: ("rodando" | "pronta" | "querendo")[]) => status.map((value) => ({ status: value }));
  expect(policy.settled({ id: "one", tabs: tabs("rodando", "pronta") })).toBe(false);
  expect(policy.settled({ id: "one", tabs: tabs("rodando", "pronta") })).toBe(false);
  expect(policy.settled({ id: "one", tabs: tabs("querendo", "pronta") })).toBe(true);
  expect(policy.settled({ id: "one", tabs: tabs("pronta", "pronta") })).toBe(false);
  expect(policy.settled({ id: "two", tabs: tabs("rodando") })).toBe(false);
  expect(policy.settled({ id: "one", tabs: tabs("pronta") })).toBe(false);
});

it("retains a short turn until a deferred workspace redraw consumes it", () => {
  const policy = new TurnSettlePolicy();
  const ready = { id: "one", tabs: [{ status: "pronta" as const }] };
  expect(policy.settled(ready)).toBe(false);

  policy.observe({ ...ready, tabs: [{ status: "rodando" }] });
  policy.observe(ready);
  // Menus and rename inputs can defer several board redraws after the turn ends.
  policy.observe(ready);
  expect(policy.settled(ready)).toBe(true);
  expect(policy.settled(ready)).toBe(false);
});

it("does not carry a deferred settlement to another workspace", () => {
  const policy = new TurnSettlePolicy();
  policy.observe({ id: "one", tabs: [{ status: "rodando" }] });
  policy.observe({ id: "one", tabs: [{ status: "pronta" }] });
  expect(policy.settled({ id: "two", tabs: [{ status: "pronta" }] })).toBe(false);
  expect(policy.settled({ id: "one", tabs: [{ status: "pronta" }] })).toBe(false);
});

it("forgets observed turns when leaving the workspace", () => {
  const policy = new TurnSettlePolicy();
  policy.observe({ id: "one", tabs: [{ status: "rodando" }] });
  policy.clear();
  expect(policy.settled({ id: "one", tabs: [{ status: "pronta" }] })).toBe(false);
  policy.observe({ id: "one", tabs: [{ status: "rodando" }] });
  policy.observe({ id: "one", tabs: [{ status: "pronta" }] });
  policy.clear();
  expect(policy.settled({ id: "one", tabs: [{ status: "pronta" }] })).toBe(false);
});
