import { t } from "../../i18n";
import { button } from "../primitives";
import { h } from "../../util";

export type PendingActions = { send: () => void; edit: () => void; discard: () => void };

/** A teammate's message waiting above the owner's composer (ADR 0090), drawn as an attention card. The host holds the
 *  message, sends it, moves it into the draft or drops it; the text is shown as typed, never as markup. */
export function pendingCard(message: { name: string; text: string }, actions: PendingActions): HTMLElement {
  const root = h("section", "ask pending");
  const title = t("team.pending.title", { name: message.name });
  root.setAttribute("aria-label", title);
  const send = button(t("team.pending.send"), actions.send, "pri");
  send.title = t("team.pending.send.title", { name: message.name });
  const edit = button(t("team.pending.edit"), actions.edit);
  edit.title = t("team.pending.edit.title");
  const row = h("div", "row");
  row.append(h("span", "spacer"), button(t("team.pending.discard"), actions.discard, "ghost"), edit, send);
  root.append(h("h4", "", title), h("p", "ui-hint", t("team.pending.hint")), h("p", "pending-text", message.text), row);
  return root;
}
