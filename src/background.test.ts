import { expect, it } from "vitest";
import { accept, batteryBudget, foreground, type BackgroundContext } from "./background";

const context = (visible: boolean, focused: boolean, revision: number): BackgroundContext =>
  ({ visible, focused, power: "battery", revision });

it("requires native visibility and focus for foreground work", () => {
  expect(foreground(context(true, true, 1))).toBe(true);
  expect(foreground(context(false, true, 2))).toBe(false);
  expect(foreground(context(true, false, 3))).toBe(false);
});

it("rejects an initial snapshot older than a native event", () => {
  const event = context(false, false, 3);
  expect(accept(event, context(true, true, 2))).toBe(event);
  expect(accept(event, context(true, true, 4))).toEqual(context(true, true, 4));
});

it("uses the battery budget when native power is unknown", () => {
  expect(batteryBudget({ ...context(true, true, 1), power: "unknown" })).toBe(true);
  expect(batteryBudget({ ...context(true, true, 1), power: "ac" })).toBe(false);
});
