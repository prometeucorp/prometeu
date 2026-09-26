import { button } from "../primitives";
import { icon } from "../icons";
import { h, template } from "../../util";

export function gitGroup(options: {
  scope: string; title: string; collapsed: boolean; changed: (collapsed: boolean) => void;
  action?: { label: string; disabled: boolean; run: () => void };
}) {
  const root = h("section", "git-group"); root.dataset.scope = options.scope;
  const head = h("div", "git-group-head"), body = h("div", "git-group-files");
  body.id = `git-files-${crypto.randomUUID()}`; body.hidden = options.collapsed;
  const toggle = button("", () => {
    body.hidden = !body.hidden; toggle.setAttribute("aria-expanded", String(!body.hidden)); options.changed(body.hidden);
  }, "ghost");
  toggle.classList.add("git-group-toggle");
  toggle.setAttribute("aria-expanded", String(!body.hidden)); toggle.setAttribute("aria-controls", body.id);
  toggle.append(template("span", "", icon("chevron-down", 12)), h("strong", "", options.title));
  head.append(toggle, h("span", "spacer"));
  if (options.action) {
    const action = button(options.action.label, options.action.run, "ghost"); action.classList.add("git-file-action");
    action.disabled = options.action.disabled; action.title = options.action.label; head.append(action);
  }
  root.append(head, body); return { root, body };
}
