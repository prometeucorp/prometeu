import { invoke } from "./ipc";
import { current as locale, fromBack, t, tn } from "./i18n";
import type { Cleanable } from "./types";
import { checkbox, formDialog } from "./ui";
import { h, template } from "./util";

// Cleanup removes worktrees and workspace-owned branches, but keeps selected existing branches and workspace cards.
// Risky rows require an explicit selection before the backend receives force.
function size(kb: number): string {
  const mb = kb / 1024;
  if (mb >= 1024) return `${(mb / 1024).toLocaleString(locale(), { maximumFractionDigits: 1 })} GB`;
  return `${Math.round(mb).toLocaleString(locale())} MB`;
}

export function openCleanup(say: (text: string, isError?: boolean) => void, only?: string) {
  let rows: Cleanable[] = [];
  const marked = new Set<string>();
  let running = false;
  const dialog = formDialog({
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
      const choice = checkbox(row.title, marked.has(row.id));
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
