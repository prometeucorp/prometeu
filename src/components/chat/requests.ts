import type { Ask } from "../../timeline";
import type { RequestResponse } from "../../conversation";
import { t } from "../../i18n";
import { icon } from "../icons";
import { button } from "../primitives";
import { h, template } from "../../util";
import { inputView, toolLabel } from "./content";

export type RequestActions = {
  respond: (response: RequestResponse) => void;
  allowAlways: () => void;
  feedbackOpen: boolean;
  feedbackChanged: (open: boolean) => void;
};

export function requestCard(ask: Ask, options: RequestActions): HTMLElement {
  if (ask.answered) {
    const el = template("div", "sys done", `${icon("check", 12)}<span></span>`);
    el.querySelector("span")!.textContent = t("chat.answered", { what: toolLabel(ask.tool) });
    return el;
  }
  const el = h("div", "ask");
  if (ask.requestKind === "plan") return planCard(el, ask);
  if (ask.requestKind === "question") return questionCard(el, ask);
  return permCard(el, ask);

  function planCard(el: HTMLElement, _ask: Ask): HTMLElement {
    el.classList.add("plan");
    el.append(h("h4", "", t("chat.plan.title")));
    const row = h("div", "row");
    const go = button(t("chat.plan.go"), undefined, "pri");
    go.title = t("chat.plan.go.title");
    go.addEventListener("click", () => {
      // Approving a plan also enables bypass before resuming so its first tool does not immediately ask again.
      options.allowAlways();
    });
    const asking = button(t("chat.plan.ask"));
    asking.title = t("chat.plan.ask.title");
    asking.addEventListener("click", () => options.respond({ outcome: "allow" }));
    const no = button(t("chat.plan.no"), undefined, "ghost");
    row.append(go, asking, no);
    el.append(row);
    // Send requested plan changes as denial feedback to the agent.
    const fb = template("div", "fb", `<textarea rows="3"></textarea><div class="row"><span class="spacer"></span><button class="pri md"></button></div>`);
    const area = fb.querySelector("textarea")!;
    area.placeholder = t("chat.plan.feedback");
    fb.querySelector("button")!.textContent = t("chat.plan.send");
    fb.hidden = !options.feedbackOpen;
    no.addEventListener("click", () => {
      options.feedbackChanged(true);
      fb.hidden = false;
      area.focus();
    });
    const send = () => {
      const text = area.value.trim();
      if (!text) return;
      options.feedbackChanged(false);
      options.respond({ outcome: "deny", message: text });
    };
    fb.querySelector("button")!.addEventListener("click", send);
    area.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        send();
      }
    });
    el.append(fb);
    return el;
  }

  /// Show one question per tab, advance to unanswered questions, and submit all answers together.
  function questionCard(el: HTMLElement, ask: Ask): HTMLElement {
    el.classList.add("question");
    type Q = { question: string; header?: string; multiSelect?: boolean; options: { label: string; description?: string }[] };
    const questions = (ask.input.questions as Q[]) ?? [];
    const answers: Record<string, string[]> = {};
    const other: Record<string, string> = {};
    let active = 0;
    const has = (q: Q) => (answers[q.question]?.length ?? 0) > 0 || !!other[q.question]?.trim();
    // Advance to the next unanswered question, wrap if needed, or remain when all are answered.
    const next = () => {
      const after = questions.findIndex((q, i) => i > active && !has(q));
      const any = questions.findIndex((q) => !has(q));
      active = after !== -1 ? after : any !== -1 ? any : active;
    };
    const tabs = h("div", "qtabs");
    const body = h("div", "qbody");
    const row = h("div", "row");
    const go = button(t("chat.ask.go"), undefined, "pri") as HTMLButtonElement;
    go.addEventListener("click", () => {
      const out: Record<string, string> = {};
      for (const q of questions) {
        const typed = other[q.question]?.trim();
        const picked = answers[q.question] ?? [];
        out[q.question] = [...picked, ...(typed ? [typed] : [])].join(", ");
      }
      options.respond({ outcome: "answer", answers: out });
    });
    row.append(h("span", "spacer"), go);

    const paint = () => {
      tabs.replaceChildren(
        ...questions.map((q, i) => {
          const b = template("button", "qtab" + (i === active ? " on" : "") + (has(q) ? " done" : ""), `<span></span>${icon("check", 11)}`);
          b.querySelector("span")!.textContent = q.header || t("chat.ask.n", { n: i + 1 });
          b.addEventListener("click", () => {
            active = i;
            paint();
          });
          return b;
        }),
      );
      tabs.hidden = questions.length < 2;
      const q = questions[active];
      body.replaceChildren();
      if (!q) return;
      body.append(h("p", "", q.question));
      const opts = h("div", "opts");
      for (const o of q.options ?? []) {
        const on = (answers[q.question] ?? []).includes(o.label);
        const b = template("button", "opt" + (on ? " on" : ""), `<b></b><span></span>`);
        b.querySelector("b")!.textContent = o.label;
        b.querySelector("span")!.textContent = o.description ?? "";
        b.addEventListener("click", () => {
          const list = answers[q.question] ?? [];
          if (q.multiSelect) {
            answers[q.question] = on ? list.filter((x) => x !== o.label) : [...list, o.label];
          } else {
            answers[q.question] = [o.label];
            next();
          }
          paint();
        });
        opts.append(b);
      }
      body.append(opts);
      const free = document.createElement("input");
      free.className = "field";
      free.placeholder = t("chat.ask.other");
      free.value = other[q.question] ?? "";
      free.addEventListener("input", () => {
        other[q.question] = free.value;
        paintGo();
        paintTabs();
      });
      free.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && free.value.trim()) {
          e.preventDefault();
          if (questions.every(has)) go.click();
          else {
            next();
            paint();
          }
        }
      });
      body.append(free);
      paintGo();
    };
    const paintGo = () => void (go.disabled = !questions.every(has));
    const paintTabs = () => {
      for (const [i, b] of [...tabs.children].entries()) b.classList.toggle("done", has(questions[i]));
    };
    el.append(tabs, body, row);
    paint();
    return el;
  }

  function permCard(el: HTMLElement, ask: Ask): HTMLElement {
    el.append(h("h4", "", t("chat.perm.title", { tool: toolLabel(ask.tool) })));
    el.append(inputView(ask.tool, ask.input));
    const row = h("div", "row");
    const yes = button(t("chat.perm.yes"), undefined, "pri");
    yes.addEventListener("click", () => options.respond({ outcome: "allow" }));
    const always = button(t("chat.perm.always"));
    always.addEventListener("click", () => {
      options.allowAlways();
    });
    const no = button(t("chat.perm.no"), undefined, "ghost");
    no.addEventListener("click", () => options.respond({ outcome: "deny", message: t("chat.perm.denied") }));
    row.append(yes, always, no);
    el.append(row);
    return el;
  }

}
