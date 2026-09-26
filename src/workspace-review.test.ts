import { beforeEach, expect, it, vi } from "vitest";
import type { Workspace } from "./types";
import type { reviewSummary } from "./components/git/review-summary";
import type { reviewEditor } from "./components/git/review-note";
import { anchorSelection } from "./review-comments";

const ui = vi.hoisted(() => ({
  summary: undefined as Parameters<typeof reviewSummary>[0] | undefined,
  save: undefined as Parameters<typeof reviewEditor>[2] | undefined,
  close: vi.fn(), add: vi.fn(),
}));
vi.mock("./ipc", () => ({ invoke: vi.fn() }));
vi.mock("./review-store", () => ({ reviews: {
  list: () => [], blocked: () => false, subscribe: vi.fn(), add: ui.add,
} }));
vi.mock("./ui", () => ({ formDialog: () => ({
  save: { remove() {} }, body: { append() {} }, open() {}, close: ui.close,
}) }));
vi.mock("./components/git/review-summary", () => ({ reviewSummary: (options: Parameters<typeof reviewSummary>[0]) => {
  ui.summary = options; return { close() {}, update() {} };
} }));
vi.mock("./components/git/review-note", () => ({
  reviewError: (error: unknown) => String(error),
  reviewEditor: (_anchor: unknown, _body: string, save: Parameters<typeof reviewEditor>[2]) => {
    ui.save = save; return { root: {}, focus() {} };
  },
}));
import { workspaceReview } from "./workspace-review";

beforeEach(() => { vi.clearAllMocks(); ui.save = undefined; ui.summary = undefined; });
it("keeps a resumed draft when its workspace becomes non-writable before saving", async () => {
  const ws = { id: "ws", repos: [] } as unknown as Workspace;
  const say = vi.fn();
  const review = workspaceReview({ workspace: () => ws, changed() {}, say, send() {}, jump() {} });
  const snapshot = review.snapshot(ws, "api", "changes", "", [])!;
  const anchor = anchorSelection("api", "a.ts", "changes", "@@ -0,0 +1 @@\n+x", 1, 1);
  snapshot.draftChanged(anchor, "Keep my note");
  await review.open();
  ui.summary!.resume(ui.summary!.drafts[0]);
  ws.cleaned = true;
  await expect(ui.save!("Keep my note")).rejects.toThrow("review.readonly");
  expect(ui.add).not.toHaveBeenCalled();
  expect(ui.close).not.toHaveBeenCalled();
  expect(review.count(ws.id)).toBe(1);
  expect(say).toHaveBeenCalled();
  ws.cleaned = false;
  await ui.save!("Keep my note");
  expect(ui.add).toHaveBeenCalledWith("ws", anchor, "Keep my note");
  expect(review.count(ws.id)).toBe(0);
  expect(ui.close).toHaveBeenCalledOnce();
});
