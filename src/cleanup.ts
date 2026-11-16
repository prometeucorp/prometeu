import { invoke } from "./ipc";
import { current as locale, fromBack, t, tn } from "./i18n";
import type { Cleanable } from "./types";
import { checkbox, formDialog, toggle } from "./ui";
import { h, template } from "./util";

// Cleanup removes worktrees and workspace-owned branches, but keeps selected existing branches and workspace cards.
// Risky rows require an explicit selection before the backend receives force.
function size(kb: number): string {
  const mb = kb / 1024;
  if (mb >= 1024) return `${(mb / 1024).toLocaleString(locale(), { maximumFractionDigits: 1 })} GB`;
  return `${Math.round(mb).toLocaleString(locale())} MB`;
}

const AUTO_CLEANUP_KEY = "prometeu:auto-cleanup";

export function autoCleanupEnabled(): boolean {
  try { return localStorage.getItem(AUTO_CLEANUP_KEY) === "true"; }
  catch { return false; }
}

export function cleanupSettingsRow(say: (text: string, isError?: boolean) => void): HTMLElement {
  const row = h("div", "setrow");
  const copy = h("div", "txt");
  const hint = h("span", "", t("clean.autoHint"));
  hint.id = "auto-cleanup-description";
  copy.append(h("b", "", t("clean.auto")), hint);
  const choice = toggle(t("clean.auto"), autoCleanupEnabled(), "settings.cleanup.auto");
  choice.control.dataset.focus = "auto-cleanup";
  choice.control.setAttribute("aria-describedby", hint.id);
  choice.control.onchange = () => {
    try { localStorage.setItem(AUTO_CLEANUP_KEY, String(choice.control.checked)); }
    catch (error) {
      choice.control.checked = autoCleanupEnabled();
      say(fromBack(error), true);
    }
  };
  const action = h("div", "act");
  action.append(choice.label);
  row.append(copy, action);
  return row;
}

/// Apply the local preference only after a successful manual archive or finish.
export async function cleanupAfterArchive(say: (text: string, isError?: boolean) => void, id: string) {
  if (!autoCleanupEnabled()) { openCleanup(say, id); return; }
  try {
    const row = (await invoke("cleanup_list")).find(item => item.id === id);
    if (!row) return;
    if (row.blocked) { openCleanup(say, id); return; }
    // The backend rechecks safety; the preference never authorizes force.
    await invoke("cleanup_worktree", { id, force: false });
    say(tn(1, "clean.done", { size: size(row.sizeKb) }));
  } catch (error) { say(fromBack(error), true); }
}

export function openCleanup(say: (text: string, isError?: boolean) => void, only?: string) {
  let rows: Cleanable[] = [];
  const marked = new Set<string>();
  let running = false;
  const dialog = formDialog({ track: "cleanup.dialog", area: "cleanup",
    title: t(only ? "clean.offer" : "clean.title"), save: t("clean.goEmpty"), cancel: t(only ? "clean.keep" : "clean.cancel"), error: fromBack,
    submit: async () => {
      running = true;
      for (const control of list.querySelectorAll("input")) control.disabled = true;
      let done = 0;
      let freed = 0;
      // Serialize operations that kill processes and change Git state in the same clone.
      for (const row of rows.filter(item => marked.has(item.id))) {
        dialog.save.textContent = t("clean.doing", { name: row.title });
        try {
          await invoke("cleanup_worktree", { id: row.id, force: !!row.blocked });
          done++;
          freed += row.sizeKb;
        } catch (error) {
          say(fromBack(error), true);
        }
      }
      running = false;
      if (done) say(tn(done, "clean.done", { size: size(freed) }));
    },
  });
  dialog.root.classList.add("clean");
  dialog.save.id = "c-go";
  dialog.save.disabled = true;
  const sum = h("span", "sub");
  dialog.root.querySelector(".sheettop")!.append(h("span", "spacer"), sum);
  const list = h("div", "cleanlist");
  const hint = h("div", "cleanhint");
  dialog.body.append(list, hint);

  function drawFoot() {
    const chosen = rows.filter(row => marked.has(row.id));
    const total = chosen.reduce((n, row) => n + row.sizeKb, 0);
    const risky = chosen.some(row => row.blocked);
    dialog.save.textContent = marked.size ? t("clean.go", { n: marked.size, size: size(total) }) : t("clean.goEmpty");
    dialog.save.disabled = !marked.size || running;
    dialog.save.classList.toggle("risk", risky);
    hint.textContent = t(risky ? "clean.hint.force" : "clean.hint");
    hint.classList.toggle("risk", risky);
    sum.textContent = tn(rows.length, "clean.count");
  }

  function draw() {
    list.replaceChildren();
    if (!rows.length) list.append(h("div", "cleanempty", t("clean.none")));
    for (const row of rows) {
      const choice = checkbox(row.title, marked.has(row.id), "cleanup.workspace.select");
      choice.label.classList.add("cleanrow");
      choice.label.classList.toggle("risk", !!row.blocked);
      choice.label.classList.toggle("on", choice.control.checked);
      const text = template("div", "txt", `<b></b><span class="where"></span>`);
      text.querySelector("b")!.textContent = row.title;
      text.querySelector(".where")!.textContent = row.blocked
        ? fromBack(row.blocked)
        : `${row.repoName} · ${row.branch}`;
      choice.label.lastElementChild!.replaceWith(text);
      choice.label.append(h("span", "pr", row.pr ? `#${row.pr}` : ""), h("span", "sz", size(row.sizeKb)));
      if (row.blocked) choice.label.title = fromBack(row.blocked);
      choice.control.addEventListener("change", () => {
        if (choice.control.checked) marked.add(row.id);
        else marked.delete(row.id);
        choice.label.classList.toggle("on", choice.control.checked);
        drawFoot();
      });
      list.append(choice.label);
    }
    drawFoot();
  }

  list.append(h("div", "cleanempty", t("clean.loading")));
  dialog.open();
  // Loading scans Git and disk usage for every row; the dialog opens first.
  invoke("cleanup_list")
    .then(got => {
      if (!dialog.root.isConnected) return;
      rows = got.filter(row => !only || row.id === only).sort((a, b) => b.sizeKb - a.sizeKb);
      for (const row of rows) if (!row.blocked) marked.add(row.id);
      draw();
    })
    .catch(error => {
      if (!dialog.root.isConnected) return;
      say(fromBack(error), true);
      dialog.close();
    });
}
