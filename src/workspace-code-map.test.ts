import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CodeMapSnapshot } from "./code-map-types";
import type { WorkspaceCodeMap } from "./workspace-code-map";

type ViewState = {
  data: CodeMapSnapshot | null; query: string; loading?: boolean;
  syncing?: boolean; error?: string; expanded?: boolean;
};
type ViewOptions = {
  snapshot: ViewState; queryChanged: (query: string) => void; rootChanged: (root: string) => void;
  inspect: (path: string) => void; sync: () => void; retry: () => void;
};
const state = vi.hoisted(() => ({
  invoke: vi.fn(), options: null as ViewOptions | null, rendered: null as ViewState | null, raw: null as ViewState | null,
  updates: [] as ViewState[], foreground: true, listeners: new Set<() => void>(),
}));
vi.mock("./ipc", () => ({ invoke: state.invoke }));
vi.mock("./i18n", () => ({ t: (key: string) => key, current: () => "en", fromBack: (error: unknown) => String(error) }));
vi.mock("./background", () => ({
  currentOrDocument: () => state.foreground, foreground: (current: boolean) => current,
  subscribe: (listener: () => void) => { state.listeners.add(listener); return () => state.listeners.delete(listener); },
}));
vi.mock("./components/code-map", () => ({ codeMapView: (options: ViewOptions) => {
  state.options = options;
  const update = (value: ViewState) => {
    state.raw = value;
    state.rendered = structuredClone(value); state.updates.push(state.rendered);
  };
  update(options.snapshot);
  return { root: {} as HTMLElement, update };
} }));

function snapshot(root: string, generation: number, query = ""): CodeMapSnapshot {
  return {
    roots: ["first", "second"].map(id => ({ id, label: id, path: `/work/${id}`, relative: id })), root_id: root,
    status: { generation, last_indexed: generation, files: 1, symbols: 1, references: 0 },
    overview: { generation, files: [{ path: "source.ts", language: "typescript", symbols: 1 }], edges: [], truncated: false },
    result: query ? { generation, roots: [], files: [], edges: [], path: [], impact: [], affected_tests: [], ambiguous: false, truncated: false } : null,
  };
}
function deferred() {
  let resolve!: (value: CodeMapSnapshot) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<CodeMapSnapshot>((done, failed) => { resolve = done; reject = failed; });
  return { promise, resolve, reject };
}
const callbacks = () => state.options!;
let controller: WorkspaceCodeMap;
beforeEach(async () => {
  vi.useFakeTimers(); vi.stubGlobal("window", globalThis); vi.resetModules();
  state.invoke.mockReset(); state.invoke.mockResolvedValue(snapshot("first", 1));
  state.options = null; state.rendered = null; state.raw = null; state.updates = []; state.foreground = true; state.listeners.clear();
  controller = new (await import("./workspace-code-map")).WorkspaceCodeMap({ openSource: vi.fn(), expanded: vi.fn() });
});
afterEach(() => { controller.hide(); vi.useRealTimers(); vi.unstubAllGlobals(); });

it("coalesces changes during a cold read and discards superseded query results", async () => {
  const cold = deferred(), obsolete = deferred(), current = deferred();
  state.invoke.mockReturnValueOnce(cold.promise).mockReturnValueOnce(obsolete.promise).mockReturnValueOnce(current.promise);
  controller.show("workspace"); await vi.advanceTimersByTimeAsync(0);
  callbacks().queryChanged("search"); callbacks().queryChanged("session"); controller.refresh();
  await vi.advanceTimersByTimeAsync(300);
  expect(state.invoke).toHaveBeenCalledTimes(1);
  cold.resolve(snapshot("first", 10)); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenCalledTimes(2);
  expect(state.invoke).toHaveBeenLastCalledWith("workspace_code_map", { id: "workspace", root: undefined, query: "session", path: undefined, force: false });
  expect(state.rendered?.data).toBeNull();
  callbacks().queryChanged("codex"); await vi.advanceTimersByTimeAsync(250);
  obsolete.resolve(snapshot("first", 20, "session")); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenCalledTimes(3);
  expect(state.invoke.mock.lastCall?.[1].query).toBe("codex");
  current.resolve(snapshot("first", 30, "codex")); await vi.advanceTimersByTimeAsync(0);
  expect(state.rendered).toMatchObject({ query: "codex", loading: false, data: { status: { generation: 30 } } });
  expect(state.updates.some(value => value.data && [10, 20].includes(value.data.status.generation))).toBe(false);
});

it("invalidates old roots and workspaces while keeping only the latest pending request", async () => {
  controller.show("workspace-one"); await vi.advanceTimersByTimeAsync(0);
  const oldRoot = deferred(), newRoot = deferred(), newWorkspace = deferred();
  state.invoke.mockReturnValueOnce(oldRoot.promise).mockReturnValueOnce(newRoot.promise).mockReturnValueOnce(newWorkspace.promise);
  callbacks().inspect("source.ts"); await vi.advanceTimersByTimeAsync(0);
  callbacks().rootChanged("second"); await vi.advanceTimersByTimeAsync(0);
  expect(state.rendered?.data).toBeNull();
  oldRoot.resolve(snapshot("first", 11, "source.ts")); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenLastCalledWith("workspace_code_map", { id: "workspace-one", root: "second", query: "source.ts", path: undefined, force: false });
  controller.show("workspace-two"); await vi.advanceTimersByTimeAsync(0);
  newRoot.resolve(snapshot("second", 22, "source.ts")); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenLastCalledWith("workspace_code_map", { id: "workspace-two", root: undefined, query: "", path: undefined, force: false });
  newWorkspace.resolve(snapshot("first", 33)); await vi.advanceTimersByTimeAsync(0);
  expect(state.rendered).toMatchObject({ query: "", data: { status: { generation: 33 } } });
  expect(state.updates.some(value => value.data && [11, 22].includes(value.data.status.generation))).toBe(false);
});

it("keeps graph references during typing and unchanged refreshes but replaces changed query context", async () => {
  controller.show("workspace"); await vi.advanceTimersByTimeAsync(0);
  const overview = state.raw!.data!.overview;
  const completed = state.raw!.data;
  callbacks().queryChanged("s"); callbacks().queryChanged("source");
  expect(state.raw!.data).toBe(completed);
  state.invoke.mockResolvedValueOnce(snapshot("first", 1, "source"));
  await vi.advanceTimersByTimeAsync(250);
  const result = state.raw!.data!.result;
  expect(state.raw!.data!.overview).toBe(overview);
  state.invoke.mockResolvedValueOnce(snapshot("first", 1, "source"));
  controller.refresh(); await vi.advanceTimersByTimeAsync(0);
  expect(state.raw!.data!.result).toBe(result);
  const other = snapshot("first", 1, "another");
  state.invoke.mockResolvedValueOnce(other);
  callbacks().queryChanged("another"); await vi.advanceTimersByTimeAsync(250);
  expect(state.raw!.data!.result).toBe(other.result);
  expect(state.raw!.data!.result).not.toBe(result);
  expect(state.raw!.data!.overview).toBe(overview);
  state.invoke.mockResolvedValueOnce(snapshot("first", 2, "another"));
  controller.refresh(); await vi.advanceTimersByTimeAsync(0);
  expect(state.raw!.data!.overview).not.toBe(overview);
});

it("stops hidden requests and polling, ignores late data, and resumes only in the foreground", async () => {
  controller.show("workspace"); await vi.advanceTimersByTimeAsync(0);
  const polling = deferred(); state.invoke.mockReturnValueOnce(polling.promise);
  await vi.advanceTimersByTimeAsync(15_001);
  expect(state.invoke).toHaveBeenCalledTimes(2);
  controller.hide(); controller.refresh();
  for (const listener of state.listeners) listener();
  polling.resolve(snapshot("first", 99)); await vi.advanceTimersByTimeAsync(60_000);
  expect(state.invoke).toHaveBeenCalledTimes(2);
  expect(state.rendered?.data?.status.generation).toBe(1);
  controller.show("workspace"); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenCalledTimes(3);
  state.foreground = false; await vi.advanceTimersByTimeAsync(45_001);
  expect(state.invoke).toHaveBeenCalledTimes(3);
  state.foreground = true; for (const listener of state.listeners) listener();
  await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenCalledTimes(4);
});

it("forces reconciliation without losing the query or last good snapshot when it fails", async () => {
  controller.show("workspace"); await vi.advanceTimersByTimeAsync(0);
  state.invoke.mockResolvedValueOnce(snapshot("first", 2, "session"));
  callbacks().queryChanged(" session "); await vi.advanceTimersByTimeAsync(250);
  const good = structuredClone(state.rendered?.data);
  const forced = deferred(); state.invoke.mockReturnValueOnce(forced.promise);
  callbacks().sync(); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke).toHaveBeenLastCalledWith("workspace_code_map", { id: "workspace", root: "first", query: "session", path: undefined, force: true });
  expect(state.rendered).toMatchObject({ query: " session ", syncing: true, loading: true, data: good });
  forced.reject(new Error("index unavailable")); await vi.advanceTimersByTimeAsync(0);
  expect(state.rendered).toMatchObject({ query: " session ", syncing: false, loading: false, data: good, error: "Error: index unavailable" });
  callbacks().retry(); await vi.advanceTimersByTimeAsync(0);
  expect(state.invoke.mock.lastCall?.[1]).toMatchObject({ query: "session", force: false });
  expect(state.rendered?.error).toBeUndefined();
});
