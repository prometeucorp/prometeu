import { mountCatalog } from "./components/gallery";
import { current, t } from "./i18n";
import { compositionExamples } from "./ui-compositions-gallery";
import { resourceExample } from "./resources/gallery";
import * as ui from "./ui";
import * as menu from "./menu";
import { h } from "./util";
import "./style.css";
import "./ui-gallery.css";

document.documentElement.lang = current();
const gallery = document.querySelector("#gallery")!;
if (!mountCatalog(gallery as HTMLElement)) {
gallery.append(h("h1", "", t("ui.gallery")), h("p", "ui-hint", t("ui.galleryHint")));
const companyGallery = h("a", "ui-link", "@prometeu/design-system");
companyGallery.setAttribute("href", "/packages/design-system/index.html");
gallery.append(companyGallery, compositionExamples(), resourceExample());

function section(title: string, ...content: HTMLElement[]) {
  const root = h("section", "ui-gallery-section");
  root.append(h("h2", "", title), ...content);
  gallery.append(root);
}

const colors = h("div", "ui-gallery-row");
for (const token of ["--bg", "--bg-side", "--bg-raised", "--fg", "--accent", "--err"]) {
  const sample = h("div", "ui-swatch");
  const color = h("span", ""); color.style.background = `var(${token})`;
  sample.append(color, h("code", "", token)); colors.append(sample);
}
section(t("ui.tokens"), colors);

const buttons = h("div", "ui-gallery-row");
const disabled = ui.button(t("ui.disabled"), () => {}); disabled.disabled = true;
buttons.append(ui.button(t("actions.save"), () => {}, "pri"), ui.button(t("actions.cancel"), () => {}), ui.button(t("actions.edit"), () => {}, "ghost"), disabled);
section(t("ui.buttons"), buttons);

const name = ui.input("Code review");
const provider = ui.select("claude", [["claude", "Claude"], ["codex", "Codex"]]);
const unavailable = ui.select("", [["", t("actions.default")]]); unavailable.control.disabled = true;
const invalid = ui.input(); invalid.setAttribute("aria-invalid", "true");
const invalidField = ui.field(t("actions.commandName"), invalid, t("ui.errorExample"));
invalidField.querySelector(".ui-hint")!.classList.add("ui-error");
section(t("ui.fields"), ui.field(t("actions.name"), name), ui.field(t("actions.provider"), provider.control), ui.field(t("ui.disabled"), unavailable.control), invalidField,
  ui.field(t("actions.instructions"), ui.input("", true)), ui.checkbox(t("actions.watch"), true).label);

const disclosure = ui.disclosure(t("actions.tools"), ui.checkbox(t("actions.inherit"), true).label);
section(t("notifications.title"), ui.toggle(t("notifications.enabled"), true).label,
  ui.radio(t("notifications.banner"), "gallery-notification-style", "banner", true).label,
  ui.radio(t("notifications.notch"), "gallery-notification-style", "notch", false).label);
const badge = ui.button(t("actions.title"), () => {
  const at = badge.getBoundingClientRect();
  menu.openAt({ x: at.left, y: at.bottom + 4 }, [{ label: "/review", badge: t("actions.origin"), hint: t("actions.agent"), run: () => {} }]);
});
section(t("ui.containers"), disclosure, badge, ui.button(t("actions.profileEditor"), () => {
  const simulate = ui.checkbox(t("ui.simulateError"), false);
  const dialog = ui.formDialog({
    title: t("actions.profileEditor"), save: t("actions.save"), cancel: t("actions.cancel"),
    error: cause => String(cause instanceof Error ? cause.message : cause),
    submit: async () => {
      await new Promise(resolve => setTimeout(resolve, 500));
      if (simulate.control.checked) throw new Error(t("ui.errorExample"));
    },
  });
  const required = ui.input(); required.required = true;
  const model = ui.select("claude", [["claude", "Claude"], ["codex", "Codex"]]);
  dialog.body.append(ui.field(t("actions.name"), required), ui.field(t("actions.provider"), model.control), simulate.label);
  dialog.open();
}));

}
