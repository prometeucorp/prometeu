import { expect, it, vi } from "vitest";
import { draftAfterCreate, followConnection, locationOptions, orderTodos, titleProblem, todoScreen, todoState, todoTitle, type Todo, type TodosView } from "./todos-model";
import { EN } from "./i18n.en";

const todo = (id: string, status: Todo["status"], updated_at = 1): Todo => ({ id, title: id, status, created_at: 1, updated_at });

it("accepts one trimmed line of 1 to 200 characters, as the backend does", () => {
  expect(todoTitle("  Ship the release  ")).toBe("Ship the release");
  expect(todoTitle("   ")).toBeNull();
  expect(todoTitle("one\ntwo")).toBeNull();
  expect(todoTitle("é".repeat(200))).toBe("é".repeat(200));
  expect(todoTitle("x".repeat(201))).toBeNull();
});

it("counts Unicode characters, not UTF-16 units, with the backend codes", () => {
  // 101 emoji are 202 UTF-16 units; an HTML maxLength of 200 would refuse them.
  expect(todoTitle("😀".repeat(101))).toBe("😀".repeat(101));
  expect(titleProblem("😀".repeat(200))).toBeNull();
  expect(titleProblem("😀".repeat(201))).toBe("title_too_long");
  expect(titleProblem(" ")).toBe("title_required");
  expect(titleProblem("a\tb")).toBe("invalid_title");
  for (const code of ["title_required", "invalid_title", "title_too_long"]) expect(EN[`todos.err.${code}` as keyof typeof EN]).toBeTruthy();
});

it("reloads a machine list when its connection changes, and never for this computer", () => {
  const machines = (phase: string) => [{ id: "vm", phase }];
  let follow = followConnection(null, machines("connected"), "vm");
  expect(follow).toEqual({ phase: "connected", reload: true });
  expect(followConnection(follow.phase, machines("connected"), "vm").reload).toBe(false);
  follow = followConnection(follow.phase, machines("connecting"), "vm");
  expect(follow.reload).toBe(true);
  expect(followConnection(follow.phase, machines("connected"), "vm").reload).toBe(true);
  expect(followConnection("connected", [], "vm")).toEqual({ phase: "removed", reload: true });
  expect(followConnection(null, machines("offline"), null)).toEqual({ phase: null, reload: false });
});

it("shows work in progress first, then waiting work, then completed work", () => {
  const ordered = orderTodos([todo("done", "completed", 9), todo("old", "todo", 1), todo("now", "doing"), todo("new", "todo", 5)]);
  expect(ordered.map(item => item.id)).toEqual(["now", "new", "old", "done"]);
});

it("separates loading, unavailable, empty and filled lists", () => {
  expect(todoScreen(null)).toEqual({ kind: "loading" });
  expect(todoScreen({ machine: "vm", items: null, error: "offline" })).toEqual({ kind: "error", message: "offline" });
  expect(todoScreen({ machine: null, items: [], error: null })).toEqual({ kind: "empty" });
  expect(todoScreen({ machine: null, items: [todo("a", "todo")], error: null }).kind).toBe("list");
});

it("names the computer that keeps the list, also after its machine was removed", () => {
  const machines = [{ id: "vm", name: "Build VM" }];
  expect(locationOptions(machines, null, "Here", "Removed")).toEqual([["", "Here"], ["vm", "Build VM"]]);
  expect(locationOptions([], "vm", "Here", "Removed")).toEqual([["", "Here"], ["vm", "Removed"]]);
});

it("has exactly three statuses with labels in the interface", () => {
  expect(["todo", "doing", "completed"].map(key => EN[`todos.status.${key}` as keyof typeof EN])).toEqual(["To do", "Doing", "Completed"]);
});

function backend() {
  let items: Todo[] = [];
  let machine: string | null = null;
  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    if (command === "todos") return { machine, items: [...items], error: null } satisfies TodosView;
    if (command === "todo_create") { const created = todo(String(args!.title), "todo"); items.push(created); return created; }
    if (command === "todo_set_status") {
      const found = items.find(item => item.id === args!.id);
      if (!found) throw "todo_not_found";
      found.status = args!.status as Todo["status"]; return found;
    }
    if (command === "todos_locate") {
      if (args!.machine === "offline") throw "machine_offline";
      machine = args!.machine as string | null; items = []; return { machine, items: [], error: null };
    }
    throw new Error(command);
  });
  return { invoke };
}

it("creates with status todo, changes status and reloads the list after each write", async () => {
  const { invoke } = backend();
  const state = todoState(invoke as never, () => {});
  await state.load();
  expect(todoScreen(state.view()).kind).toBe("empty");
  expect(await state.create("  Write docs ")).toEqual({ kind: "done" });
  expect(invoke).toHaveBeenCalledWith("todo_create", { title: "Write docs" });
  expect(state.view()!.items![0].status).toBe("todo");
  expect(await state.setStatus("Write docs", "completed")).toEqual({ kind: "done" });
  expect(state.view()!.items![0].status).toBe("completed");
  expect(await state.setStatus("missing", "doing")).toEqual({ kind: "failed", error: "todo_not_found" });
  expect(state.view()!.items).toHaveLength(1);
});

it("never sends an invalid title", async () => {
  const { invoke } = backend();
  const state = todoState(invoke as never, () => {});
  expect(await state.create("  ")).toEqual({ kind: "failed", error: "title_required" });
  expect(invoke).not.toHaveBeenCalledWith("todo_create", expect.anything());
});

it("keeps the current location when a machine cannot take the list", async () => {
  const { invoke } = backend();
  const state = todoState(invoke as never, () => {});
  await state.load();
  expect(await state.locate("offline")).toEqual({ kind: "failed", error: "machine_offline" });
  expect(state.view()!.machine).toBeNull();
  expect(await state.locate("vm")).toEqual({ kind: "done" });
  expect(state.view()!.machine).toBe("vm");
});

it("shows the newest answer when reloads overlap and keeps the failure visible", async () => {
  const answers: ((view: TodosView) => void)[] = [];
  const invoke = vi.fn(() => new Promise<TodosView>(done => answers.push(done)));
  const state = todoState(invoke as never, () => {});
  const first = state.load(), second = state.load();
  answers[1]({ machine: null, items: [todo("new", "todo")], error: null });
  answers[0]({ machine: null, items: [], error: null });
  await Promise.all([first, second]);
  expect(state.view()!.items!.map(item => item.id)).toEqual(["new"]);
  expect(state.loading()).toBe(false);
  const failing = todoState((async () => { throw "machine_offline"; }) as never, () => {});
  await failing.load();
  expect(todoScreen(failing.view())).toEqual({ kind: "error", message: "machine_offline" });
});

it("reports a second write during a pending one as ignored, not as done", async () => {
  let release = () => {};
  const invoke = vi.fn(async (command: string) => {
    if (command === "todo_create") await new Promise<void>(done => { release = done; });
    return { machine: null, items: [], error: null };
  });
  const state = todoState(invoke as never, () => {});
  const first = state.create("one");
  expect(state.busy()).toBe(true);
  expect(await state.create("two")).toEqual({ kind: "ignored" });
  release();
  expect(await first).toEqual({ kind: "done" });
  expect(invoke.mock.calls.filter(([command]) => command === "todo_create")).toHaveLength(1);
});

it("keeps a draft typed during a pending create, and any draft when the create did not complete", () => {
  expect(draftAfterCreate({ kind: "done" }, "first", "first")).toBe("");
  expect(draftAfterCreate({ kind: "done" }, "first", "second")).toBe("second");
  expect(draftAfterCreate({ kind: "ignored" }, "first", "first")).toBe("first");
  expect(draftAfterCreate({ kind: "failed", error: "machine_offline" }, "first", "first")).toBe("first");
});
