import * as ui from "./primitives";
import { icon, iconNames, fileIcon, stageIcon, brand, avatar, avatars } from "./icons";
import { iconButton } from "./icon-button";
import { conversationBlock, workCard, errorCard } from "./chat/blocks";
import { requestCard } from "./chat/requests";
import { composer, attachmentChip } from "./chat/composer";
import { renderUserMessage, browserContextChip, inputView, contextPanel } from "./chat/content";
import { gitFileRow } from "./git/file-row";
import { gitGroup } from "./git/group";
import { commitForm } from "./git/commit-form";
import { diffView } from "./git/diff-view";
import { compositionExamples } from "../ui-compositions-gallery";
import { resourceExample } from "../resources/gallery";
import { md } from "./chat/markdown";
import { t } from "../i18n";
import { h } from "../util";
import type { ToolBlock, Ask } from "../timeline";
import type { RepoDiff } from "../types";

export type Story = { root: HTMLElement; destroy?: () => void };
type Factory = (state: string, report: (value: string) => void) => Story;
const wrap = (...nodes: HTMLElement[]): Story => {
  const root = h("div", "story-sample"); root.append(...nodes); return { root };
};
const tool = (state: string): ToolBlock => ({
  kind: "tool", id: "story-tool", name: state === "plan" ? "ExitPlanMode" : "Bash",
  input: state === "plan" ? { plan: "## Review plan\n\n1. Inspect changes\n2. Run focused checks" } : { command: "npm test", description: "Run focused checks" },
  json: "", result: state === "running" ? null : state === "error" ? "Process exited with code 1\nExpected a valid result" : "All checks passed",
  error: state === "error", done: state !== "running", background: state === "background",
});

export const stories: Record<string, Factory> = {
  button(state, report) {
    return wrap(...(["pri", "outline", "ghost", "danger"] as const).map(variant => {
      const control = ui.button(variant, () => report(variant), variant); control.disabled = state === "disabled"; return control;
    }));
  },
  fields(state) {
    const input = ui.input("Example"); input.disabled = state === "disabled";
    if (state === "error") input.setAttribute("aria-invalid", "true");
    const password = ui.password("example", { show: t("ui.showPassword"), hide: t("ui.hidePassword") });
    password.control.setAttribute("aria-label", t("ui.password"));
    password.control.disabled = state === "disabled";
    password.root.querySelector("button")!.disabled = state === "disabled";
    const instructions = ui.input("Example instructions", true); instructions.disabled = state === "disabled";
    return wrap(ui.field(t("actions.name"), input, state === "error" ? t("ui.errorExample") : ""),
      ui.field(t("actions.instructions"), instructions), password.root);
  },
  choices(state) {
    const choices = [ui.checkbox(t("actions.watch"), true), ui.toggle(t("notifications.enabled"), true),
      ui.radio(t("notifications.banner"), "story-notifications", "banner", true),
      ui.radio(t("notifications.notch"), "story-notifications", "notch", false)];
    choices.forEach(choice => { choice.control.disabled = state === "disabled"; });
    return wrap(...choices.map(choice => choice.label));
  },
  selection(state, report) {
    const select = ui.select("one", [["one", "Example one"], ["two", "Example two"]]);
    select.control.disabled = state === "disabled"; select.onchange = () => report(select.value);
    const menu = ui.menuButton(t("actions.more"), () => [{ label: t("actions.edit"), run: () => report("edit") }]);
    const picker = ui.button(t("ui.componentSearch"), () => ui.searchablePicker(picker, {
      label: t("ui.componentSearch"), searchPlaceholder: t("ui.componentSearch"), empty: t("settings.resourceNoResults"),
      items: Array.from({ length: 20 }, (_, i) => ({ key: String(i), label: `Example ${i + 1}` })), select: report,
    }));
    menu.disabled = picker.disabled = state === "disabled";
    return wrap(ui.field(t("actions.kind"), select.control), menu, picker);
  },
  dialog(state, report) {
    const open = ui.button(t("actions.edit"), () => {
      const dialog = ui.formDialog({ title: t("actions.profileEditor"), save: t("actions.save"), cancel: t("actions.cancel"),
        error: String, submit: async () => {
          if (state === "error") throw new Error(t("ui.errorExample")); report("save");
        },
      });
      const name = ui.input("Example"); name.required = true; dialog.body.append(ui.field(t("actions.name"), name)); dialog.open();
    });
    const confirm = ui.button(t("actions.remove"), () => {
      void ui.confirmDialog({ title: t("actions.remove"), message: t("ui.errorExample"), accept: t("actions.remove"), cancel: t("actions.cancel") })
        .then(value => report(String(value)));
    });
    return wrap(open, confirm);
  },
  feedback(state) {
    return wrap(ui.badge(t("ui.resourceReady")), ui.notice(t(state === "error" ? "ui.errorExample" : "ui.resourceReady"), state === "error" ? "error" : "success"),
      ui.card(t("actions.title"), ui.disclosure(t("actions.tools"), h("p", "", "Example content"))), ui.avatar());
  },
  icons(state) {
    const samples: [string, string][] = state === "files" ? ["index.ts", "main.rs", "README.md", "package.json", "photo.png"].map(name => [name, fileIcon(name)])
      : state === "stages" ? Array.from({ length: 4 }, (_, i) => [`stageIcon(${i}, 4)`, stageIcon(i, 4)])
      : state === "providers" ? ["claude", "codex", "antigravity"].map(name => [name, brand(name)])
      : iconNames.map(name => [name, icon(name)]);
    if (state === "default") samples.push(["avatar", avatar("Example")], ["avatars", avatars(["One", "Two"])]);
    const root = h("div", "story-icons");
    for (const [name, svg] of samples) {
      const item = h("div", "story-icon"); const mark = h("span", ""); mark.innerHTML = svg;
      item.append(mark, h("code", "", name)); root.append(item);
    }
    return { root };
  },
  "icon-button": (state, report) => wrap(iconButton({ label: t("git.refresh"), glyph: "rotate", disabled: state === "disabled", run: () => report("refresh") })),
  compositions: state => ({ root: compositionExamples(state) }),
  resources: state => ({ root: resourceExample(state) }),
  "chat-block"(state) {
    return wrap(conversationBlock(state === "text" ? { kind: "text", text: "## Review complete\n\nChanges look good.\n\n```ts\nconst ready = true;\n```" }
      : state === "thinking" ? { kind: "thinking", text: "Checking dependencies and preserving the existing behavior." } : tool(state), state === "running" || state === "thinking"));
  },
  "chat-work": (state, report) => wrap(workCard([{ block: tool(state), live: state === "running" }], false, open => report(String(open)))),
  "chat-error": state => wrap(errorCard(state === "short" ? "Example operation failed" : "Example operation failed\nProcess exited with code 1\nThe original input remains available.")),
  "chat-request"(state, report) {
    const ask: Ask = { kind: "ask", id: "story-request", ts: 0, toolUseId: null, answered: state === "answered",
      requestKind: state === "plan" ? "plan" : state === "permission" ? "approval" : "question", tool: "Example tool",
      input: { questions: [{ question: "Which files should be reviewed?", header: "Files", options: [
        { label: "Changed files", description: "Review the current changes" }, { label: "All files", description: "Review the entire project" },
      ] }] },
    };
    return wrap(requestCard(ask, { respond: value => report(JSON.stringify(value)), allowAlways: () => report("allowAlways"), feedbackOpen: false, feedbackChanged: open => report(String(open)) }));
  },
  composer(state, report) {
    const view = composer({ send: () => report(view.area.value), stop: () => report("stop"), addFile: () => report("addFile"),
      voice: () => report("voice"), quote: () => report("quote"), actions: () => report("actions"), voiceAvailable: true });
    view.area.placeholder = t("chat.placeholder"); view.area.setAttribute("aria-label", view.area.placeholder);
    view.addFile.hidden = false;
    view.stop.hidden = state !== "busy"; view.root.classList.toggle("busy", state === "busy");
    view.area.disabled = view.send.disabled = state === "offline";
    if (state === "attachments") {
      view.files.hidden = false;
      const chip = attachmentChip({ name: "example.ts", title: "src/example.ts", removeLabel: t("chat.attachment.remove"), remove: () => { chip.remove(); report("remove"); } });
      view.files.append(chip);
    }
    return wrap(view.root);
  },
  markdown(state) {
    const root = h("div", "md");
    root.innerHTML = md(state === "diff" ? "```diff\n-const value = 1;\n+const value = 2;\n```"
      : state === "untrusted" ? "<script>alert('example')</script>\n\n[unsafe](javascript:alert(1))"
      : "## Example output\n\n- Preserves **formatting**\n- Uses shared components\n\n```ts\nconst ready = true;\n```");
    return { root };
  },
  "chat-content"(state) {
    if (state === "input") return wrap(inputView("Edit", { file_path: "example.ts", old_string: "const value = 1;", new_string: "const value = 2;" }));
    if (state === "context") return wrap(contextPanel({ model: "Example model", used: "20k", total: "100k", pct: 20,
      categories: [{ name: "Messages", tokens: "20k", n: 20000, pct: 20 }], sections: [] }));
    if (state === "browser") return wrap(browserContextChip({ selection: { url: "https://example.com", selector: "main", tag: "MAIN", text: "Example page",
      html: "<main>Example page</main>", styles: { display: "block" }, rect: { x: 0, y: 0, width: 400, height: 200 }, viewport: { width: 800, height: 600 } } }));
    const bubble = h("div", "bubble"); renderUserMessage(bubble, "Review this change and preserve the current behavior."); return wrap(bubble);
  },
  "git-file"(state, report) {
    const status = state === "deleted" ? "D" : state === "new" ? "?" : state === "conflict" ? "U" : "M";
    return wrap(gitFileRow({ path: state === "long" ? "src/components/example/very-long-file-name-that-must-fit-in-the-current-panel.ts" : "src/example.ts", status,
      statusLabel: t(`git.status.${status}`), selected: true, select: () => report("select"), open: status === "D" ? undefined : () => report("open"),
      contextMenu: () => report("menu"), action: { label: t("git.stage.short"), description: t("git.stage"), disabled: state === "disabled", run: () => report("stage") } }));
  },
  "git-group"(state, report) {
    const group = gitGroup({ scope: "changes", title: t("diff.files.one", { n: 1 }), collapsed: state === "collapsed", changed: value => report(String(value)) });
    group.body.append(stories["git-file"]("modified", report).root); return { root: group.root };
  },
  "git-commit": (state, report) => wrap(commitForm({ message: state === "empty" ? "" : "refactor: reuse desktop components", label: t("git.message"),
    placeholder: t("git.message.placeholder"), submit: t("git.commit.files.one", { n: 1 }), hint: t("git.commit.hint"), title: t("git.commit.hint"),
    busy: state === "busy", disabled: state === "busy" || state === "disabled", changed: report, commit: () => report("commit") })),
  diff(state, report) {
    const root = h("div", "story-diffs");
    const readers: ReturnType<typeof diffView>[] = [];
    for (let i = 0; i < (state === "two-instances" ? 2 : 1); i++) {
      const seen = new Set<string>();
      const reader = diffView({ isSeen: (_id, repo, file) => seen.has(`${repo}/${file.path}`), setSeen: (_id, repo, file, on) => {
        const key = `${repo}/${file.path}`; if (on) seen.add(key); else seen.delete(key);
      } });
      readers.push(reader);
      const host = h("div", "dlist story-diff"); root.append(host);
      const repos: RepoDiff[] = [{ name: "example", base: "main", ahead: 1, unpushed: 1, dirty: 1, files: [{
        path: "src/example.ts", added: 1, removed: 1, new_file: false, deleted: false, dirty: true,
        patch: state === "binary" ? "" : "@@ -1,2 +1,2 @@\n-const value = 1;\n+const value = 2;\n export { value };",
      }] }];
      reader.render(host, { id: `story-${i}`, repos: state === "empty" ? [] : repos, layout: state === "split" ? "split" : "unified",
        empty: t("diff.clean"), onSeen: () => report("review"), onOpen: (_repo, path) => report(path) });
    }
    return { root, destroy: () => readers.forEach(reader => reader.destroy()) };
  },
};
