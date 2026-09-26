import { button, input, field } from "../primitives";
import { icon } from "../icons";
import { h, template } from "../../util";

/** The host decides Git availability. The component checks only the required message. */
export function commitForm(options: {
  message: string; label: string; placeholder: string; submit: string; hint: string; title: string;
  busy: boolean; disabled: boolean; changed: (message: string) => void; commit: () => void;
}) {
  const root = h("div", "git-composer");
  const area = input(options.message, true); area.id = "git-message";
  area.placeholder = options.placeholder; area.disabled = options.busy; area.rows = 3;
  const commit = button(options.submit, () => { if (!commit.disabled) options.commit(); }, "pri"); commit.id = "git-commit";
  commit.prepend(template("span", "", icon("check", 14))); commit.title = options.title;
  const paint = () => { commit.disabled = options.busy || options.disabled || !area.value.trim(); };
  area.oninput = () => { options.changed(area.value); paint(); };
  paint(); root.append(field(options.label, area), commit, h("p", "git-hint", options.hint));
  return root;
}
