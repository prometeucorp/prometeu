import { afterAll, beforeAll, beforeEach, expect, it, vi, type Mock } from "vitest";
import type { ConversationCommandV1 } from "./conversation";
import type { AnalyticsEvent } from "./analytics";

const panel = vi.hoisted(() => ({ send: undefined as ((session: string, frame: ConversationCommandV1) => Promise<unknown>) | undefined }));
vi.mock("./ipc", () => ({ invoke: vi.fn(async (command: string) => command === "island_current" ? { layout: { expanded: false, top: 32 }, notice: null } : undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("./island-view", () => ({
  requestPanels: (send: typeof panel.send) => { panel.send = send; return { panel() {}, prune() {} }; },
  redraw() {}, islandView() {},
}));
import { invoke } from "./ipc";
// `vi.mocked(invoke)` makes the checker relate the whole IPC registry to the mock type, which costs minutes.
const invokeMock = invoke as unknown as Mock;

beforeAll(async () => {
  vi.useFakeTimers();
  vi.stubGlobal("document", { body: { dataset: {} }, addEventListener() {} });
  vi.stubGlobal("ResizeObserver", class { observe() {} });
  await import("./notification-window");
});
beforeEach(() => invokeMock.mockClear());
afterAll(() => { vi.useRealTimers(); vi.unstubAllGlobals(); });

const events = () => invokeMock.mock.calls.flatMap(([command, args]) => command === "analytics_track" ? [(args as { event: AnalyticsEvent }).event] : []);

it.each([
  [{ outcome: "allow" }, { name: "approval_submitted", approved: true }],
  [{ outcome: "deny", message: "Private feedback" }, { name: "approval_submitted", approved: false }],
  [{ outcome: "answer", answers: { private_question: "Private answer" } }, { name: "request_answered" }],
] as const)("records the accepted outcome without request content: $0.outcome", async (response, event) => {
  await panel.send!("private-session", { v: 1, type: "request.respond", requestId: "private-request", response });
  expect(events()).toEqual([event, { name: "notification_action_used", source: "island", action: "answer" }]);
});

it("records neither request use nor notification success when control delivery fails", async () => {
  invokeMock.mockRejectedValueOnce(new Error("Control unavailable"));
  await expect(panel.send!("session", { v: 1, type: "request.respond", requestId: "request", response: { outcome: "allow" } })).rejects.toThrow("Control unavailable");
  expect(events()).toEqual([]);
});

it("does not classify a permission-mode command as an accepted request response", async () => {
  await panel.send!("session", { v: 1, type: "permission.mode.set", mode: "bypass" });
  expect(events()).toEqual([]);
});
