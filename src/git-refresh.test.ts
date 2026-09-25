import { expect, it } from "vitest";
import { GitRefreshPolicy, changesInterval, marksInterval } from "./git-refresh";
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
