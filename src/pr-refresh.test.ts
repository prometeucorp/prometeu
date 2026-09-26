import { describe, expect, it } from "vitest";
import { PrScanPolicy } from "./pr-refresh";
import type { BackgroundContext } from "./background";

const context = (power: BackgroundContext["power"], visible = true, focused = true): BackgroundContext =>
  ({ power, visible, focused, revision: 1 });

describe("general PR discovery", () => {
  it("uses foreground AC, battery and hidden budgets", () => {
    const policy = new PrScanPolicy();
    policy.mark(1_000);
    expect(policy.remaining(context("ac"), 1_000)).toBe(180_000);
    expect(policy.remaining(context("battery"), 1_000)).toBe(300_000);
    expect(policy.remaining(context("unknown"), 1_000)).toBe(300_000);
    expect(policy.remaining(context("ac", false), 1_000)).toBe(900_000);
    expect(policy.remaining(context("ac", true, false), 1_000)).toBe(900_000);
    expect(policy.remaining(context("ac"), 181_000)).toBe(0);
  });

  it("refreshes on foreground return and counts from the latest request", () => {
    const policy = new PrScanPolicy();
    expect(policy.returnedToForeground(context("ac", false), context("ac"), 0)).toBe(true);
    expect(policy.returnedToForeground(context("battery"), context("ac"), 0)).toBe(false);
    policy.mark(500_000);
    expect(policy.remaining(context("battery"), 500_100)).toBe(299_900);
  });

  it("spaces scans on frequent returns to the foreground", () => {
    const policy = new PrScanPolicy();
    policy.mark(0);
    expect(policy.returnedToForeground(context("ac", true, false), context("ac"), 15_000)).toBe(false);
    expect(policy.returnedToForeground(context("ac", true, false), context("ac"), 59_999)).toBe(false);
    expect(policy.returnedToForeground(context("ac", true, false), context("ac"), 60_000)).toBe(true);
  });
});
