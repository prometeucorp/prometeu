/// Rules of the To-dos tab without the DOM. The backend owns the list (`docs/contracts/todos.md`);
/// this module orders it, checks a title before it is sent and keeps the newest answer only.

export type TodoStatus = "todo" | "doing" | "completed";
export const todoStatuses: readonly TodoStatus[] = ["todo", "doing", "completed"];
export type Todo = { id: string; title: string; status: TodoStatus; created_at: number; updated_at: number };
/// `machine` is null for this computer. `items` is null when the list could not be read.
export type TodosView = { machine: string | null; items: Todo[] | null; error: string | null };

export const MAX_TITLE = 200;

export type TitleProblem = "title_required" | "invalid_title" | "title_too_long";

/// The same rule and the same codes as `prometeu_core::todos::title`: one trimmed line of 1 to 200
/// Unicode scalar values. The input must not use `maxLength`, which counts UTF-16 units.
export function titleProblem(raw: string): TitleProblem | null {
  const title = raw.trim();
  if (!title) return "title_required";
  if (/[\u0000-\u001f\u007f-\u009f]/.test(title)) return "invalid_title";
  return [...title].length > MAX_TITLE ? "title_too_long" : null;
}

export function todoTitle(raw: string): string | null {
  return titleProblem(raw) ? null : raw.trim();
}

/// The phase of the machine that keeps the list, or null for this computer.
export function locationPhase(machines: readonly { id: string; phase: string }[], machine: string | null): string | null {
  return machine ? machines.find(item => item.id === machine)?.phase ?? "removed" : null;
}

/// The draft after a create ends. Only a completed create clears it, and only when the person did
/// not type a new draft while the request was pending.
export function draftAfterCreate(outcome: WriteOutcome, submitted: string, current: string): string {
  return outcome.kind === "done" && current === submitted ? "" : current;
}

/// Whether a `machines` update makes the shown list old. A machine that disconnects must show its
/// error instead of its last items, and a machine that comes back must show what its agents changed.
export function followConnection(previous: string | null, machines: readonly { id: string; phase: string }[], machine: string | null) {
  const phase = locationPhase(machines, machine);
  return { phase, reload: machine !== null && phase !== previous };
}

/// `ignored` means that another write was pending, so this one was not sent.
export type WriteOutcome = { kind: "done" } | { kind: "ignored" } | { kind: "failed"; error: unknown };

/// Work in progress first, then what waits, then what is done; the newest change first in each.
export function orderTodos(items: readonly Todo[]): Todo[] {
  const rank: Record<TodoStatus, number> = { doing: 0, todo: 1, completed: 2 };
  return [...items].sort((a, b) => rank[a.status] - rank[b.status] || b.updated_at - a.updated_at || b.created_at - a.created_at);
}

/// The selector shows this computer and each machine. A selected machine that was removed stays
/// visible, so the person sees where the list is and can move it back.
export function locationOptions(machines: readonly { id: string; name: string }[], selected: string | null, here: string, removed: string): [string, string][] {
  const options: [string, string][] = [["", here], ...machines.map(machine => [machine.id, machine.name] as [string, string])];
  if (selected && !machines.some(machine => machine.id === selected)) options.push([selected, removed]);
  return options;
}

export type TodoScreen =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "empty" }
  | { kind: "list"; items: Todo[] };

export function todoScreen(view: TodosView | null): TodoScreen {
  if (!view) return { kind: "loading" };
  if (!view.items) return { kind: "error", message: view.error ?? "" };
  return view.items.length ? { kind: "list", items: orderTodos(view.items) } : { kind: "empty" };
}

type Invoke = {
  (command: "todos"): Promise<TodosView>;
  (command: "todo_create", args: { title: string }): Promise<Todo>;
  (command: "todo_set_status", args: { id: string; status: TodoStatus }): Promise<Todo>;
  (command: "todos_locate", args: { machine: string | null }): Promise<TodosView>;
};

/// Requests and their order. A reload that an older request started never replaces a newer answer.
export function todoState(invoke: Invoke, changed: () => void) {
  let view: TodosView | null = null;
  let loading = false;
  let busy = false;
  let revision = 0;

  async function load() {
    const request = ++revision;
    loading = true; changed();
    try {
      const next = await invoke("todos");
      if (request === revision) view = next;
    } catch (cause) {
      if (request === revision) view = { machine: view?.machine ?? null, items: null, error: String(cause) };
    }
    if (request === revision) { loading = false; changed(); }
  }

  /// Writes go one at a time. A failure keeps the list and returns the error for the caller.
  async function write(run: () => Promise<unknown>): Promise<WriteOutcome> {
    if (busy) return { kind: "ignored" };
    busy = true; changed();
    try {
      await run();
      return { kind: "done" };
    } catch (error) {
      return { kind: "failed", error };
    } finally {
      busy = false;
      await load();
    }
  }

  return {
    view: () => view,
    loading: () => loading,
    busy: () => busy,
    load,
    create: (raw: string): Promise<WriteOutcome> => {
      const problem = titleProblem(raw);
      return problem ? Promise.resolve({ kind: "failed", error: problem }) : write(() => invoke("todo_create", { title: raw.trim() }));
    },
    setStatus: (id: string, status: TodoStatus) => write(() => invoke("todo_set_status", { id, status })),
    locate: (machine: string | null) => write(async () => { view = await invoke("todos_locate", { machine }); }),
  };
}
