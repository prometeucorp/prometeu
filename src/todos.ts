import { listen } from "@tauri-apps/api/event";
import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import { button, input, select } from "./ui";
import { issueRefresh } from "./issues-controls";
import { toolbar } from "./components/compositions";
import { empty, h, template } from "./util";
import type { MachineView } from "./machines";
import { draftAfterCreate, followConnection, locationOptions, locationPhase, titleProblem, todoScreen, todoState, todoStatuses, type Todo, type WriteOutcome } from "./todos-model";

type Context = { say: (message: string, error?: boolean) => void };

/// Tasks > To-dos: the person's standalone list. It works without Linear or GitHub. The selector
/// shows which computer keeps the list; agents reach the same list through the Prometeu MCP.
export function todoList(host: HTMLElement, ctx: Context) {
  let visible = false;
  let stale = true;
  let machines: MachineView[] = [];
  const state = todoState(invoke, () => draw());

  const bar = h("div", "subbar ibar todo-bar");
  const titleField = template("label", "ifind todo-new", "");
  const title = input("", false, "todos.title");
  // No maxLength: it counts UTF-16 units, and the list counts Unicode characters (titleProblem).
  title.spellcheck = true;
  title.placeholder = t("todos.title.placeholder"); title.setAttribute("aria-label", t("todos.title.label"));
  titleField.append(title);
  const add = button(t("todos.add"), () => void create(), "outline", "todos.add");
  add.classList.add("sm");
  title.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); void create(); } });
  title.addEventListener("input", () => {
    if (titleProblem(title.value) === "title_too_long") title.setAttribute("aria-invalid", "true");
    else title.removeAttribute("aria-invalid");
    drawControls();
  });
  const location = select("", [["", t("todos.location.here")]], { track: "todos.location" });
  location.control.classList.add("task-filter");
  location.control.setAttribute("aria-label", t("todos.location.label"));
  location.control.title = t("todos.location.hint");
  location.onchange = () => void move(location.value || null);
  const meta = h("span", "imeta"); meta.setAttribute("role", "status");
  const refresh = issueRefresh(t("issues.refresh"), () => void state.load());
  refresh.dataset.track = "todos.refresh";
  bar.append(toolbar([titleField, add], [meta, location.control, refresh]));
  const list = h("div", "ilist todo-list"); list.id = "todo-list";
  list.setAttribute("aria-labelledby", "issues-provider-todos");
  host.append(bar, list);

  void listen("todos", () => { stale = true; if (visible) void state.load(); });
  // A connection change of the machine that keeps the list makes the shown list old: offline, it
  // must show the error; back online, it must show what host agents changed meanwhile.
  let phase: string | null = null;
  void listen<MachineView[]>("machines", ({ payload }) => {
    machines = payload;
    const follow = followConnection(phase, machines, state.view()?.machine ?? null);
    phase = follow.phase;
    if (follow.reload) { stale = !visible; if (visible) void state.load(); }
    drawControls();
  });

  function report(outcome: WriteOutcome) {
    if (outcome.kind === "failed") ctx.say(fromBack(outcome.error), true);
  }

  async function create() {
    const problem = titleProblem(title.value);
    if (problem) {
      title.setAttribute("aria-invalid", "true");
      ctx.say(t(`todos.err.${problem}`), true);
      title.focus();
      return;
    }
    // The input stays editable during a slow request. Clear only the draft that was sent.
    const submitted = title.value;
    const outcome = await state.create(submitted);
    report(outcome);
    title.value = draftAfterCreate(outcome, submitted, title.value); drawControls();
  }

  async function move(machine: string | null) {
    report(await state.locate(machine));
  }

  function drawControls() {
    const view = state.view();
    const busy = state.busy();
    const selected = view?.machine ?? null;
    location.setOptions(locationOptions(machines, selected, t("todos.location.here"), t("todos.location.removed")), selected ?? "");
    location.control.disabled = busy;
    // Writes need a list that answers; a failed read shows why in the list area.
    const writable = !!view?.items;
    add.disabled = busy || !writable || titleProblem(title.value) !== null;
    title.disabled = !writable;
    refresh.disabled = state.loading();
    meta.textContent = state.loading() || busy ? t("todos.loading") : "";
  }

  function draw() {
    if (!visible) return;
    drawControls();
    const screen = todoScreen(state.view());
    list.setAttribute("aria-busy", String(state.loading() || state.busy()));
    if (screen.kind === "loading") { list.replaceChildren(); return; }
    if (screen.kind === "error") {
      // Selecting the machine again also makes its host serve the list after another desktop
      // moved it away (todos_not_here); for this computer it only reads again.
      const machine = state.view()?.machine ?? null;
      list.replaceChildren(empty(t("todos.failed.title"), screen.message, [t("issues.failed.action"), () => void (machine ? move(machine) : state.load()), "todos.retry"]));
      return;
    }
    if (screen.kind === "empty") { list.replaceChildren(empty(t("todos.empty.title"), t("todos.empty.body"))); return; }
    const focused = (document.activeElement as HTMLElement | null)?.dataset.focus;
    list.replaceChildren(...screen.items.map(row));
    if (focused) list.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focused)}"]`)?.focus();
  }

  function row(todo: Todo) {
    const el = template("div", `irow todo-row ${todo.status}`, `<span class="ititle"><b></b></span><span class="iact"></span>`);
    el.querySelector("b")!.textContent = todo.title;
    el.title = todo.title;
    const status = select(todo.status, todoStatuses.map(key => [key, t(`todos.status.${key}`)] as [string, string]), { track: "todos.status" });
    status.control.classList.add("task-filter");
    status.control.setAttribute("aria-label", t("todos.status.label", { title: todo.title }));
    status.control.dataset.focus = todo.id;
    status.control.disabled = state.busy();
    status.onchange = async () => report(await state.setStatus(todo.id, status.value as Todo["status"]));
    el.querySelector(".iact")!.append(status.control);
    return el;
  }

  return {
    show() {
      visible = true; host.hidden = false;
      void invoke("machines").then(found => { machines = found; phase = locationPhase(machines, state.view()?.machine ?? null); drawControls(); }).catch(() => {});
      draw();
      if (stale || !state.view()) { stale = false; void state.load(); }
    },
    hide() { visible = false; host.hidden = true; },
    draw,
  };
}
