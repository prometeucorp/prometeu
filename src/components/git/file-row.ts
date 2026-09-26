import { button } from "../primitives";
import { icon } from "../icons";
import { h, template } from "../../util";

export function gitFileRow(options: {
  path: string; status: string; statusLabel: string; selected: boolean;
  select: () => void; open?: () => void;
  action?: { label: string; description: string; disabled: boolean; run: () => void };
  contextMenu: (event: MouseEvent) => void;
}) {
  const root = h("div", `git-file${options.selected ? " selected" : ""}`); root.dataset.path = options.path;
  const cut = options.path.lastIndexOf("/");
  const open = button("", options.select, "ghost"); open.classList.add("git-file-name"); open.title = options.path;
  open.setAttribute("aria-current", String(options.selected));
  open.append(template("span", "git-reviewed", icon("check", 12)), h("span", "", options.path.slice(cut + 1)), h("small", "", options.path.slice(0, cut + 1)));
  if (options.open) open.ondblclick = options.open;
  root.addEventListener("contextmenu", event => { event.preventDefault(); options.contextMenu(event); });
  const letter = h("span", `git-letter status-${options.status === "?" ? "new" : options.status}`, options.status === "?" ? "U" : options.status);
  letter.title = options.statusLabel; root.append(open, letter);
  if (options.action) {
    const action = button(options.action.label, options.action.run, "ghost");
    action.classList.add("git-file-action"); action.disabled = options.action.disabled;
    action.title = options.action.description; action.setAttribute("aria-label", options.action.description); root.append(action);
  }
  return root;
}
