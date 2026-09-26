import { icon } from "../icons";
import { fromBack, t, tn } from "../../i18n";
import { diffHtml, isDiff } from "../../highlight";
import { md } from "../../markdown";
import { h, template } from "../../util";
import { summary, type Block, type ToolBlock } from "../../timeline";
import { capError, capLines, countTools, errorPeek, inputView, peek, tallyText, toolIcon, toolLabel, wantsCard } from "./content";

export type BlockPart = { block: Block; live: boolean };

export function errorCard(text: string): HTMLElement {
  const value = fromBack(text).trim() || t("chat.result.error");
  if (!value.includes("\n") && value.length <= 180) {
    const el = h("div", "sys err");
    el.textContent = value;
    return el;
  }
  const el = template(
    "details",
    "syserr",
    `<summary><span class="eic">${icon("x", 12)}</span><b></b><span class="prev"></span></summary><pre></pre>`,
  );
  el.querySelector("b")!.textContent = t("chat.error.title");
  el.querySelector(".prev")!.textContent = errorPeek(value);
  el.querySelector("pre")!.textContent = capError(value);
  return el;
}

export function workCard(parts: BlockPart[], opened: boolean, changed: (open: boolean) => void): HTMLElement {
  if (!wantsCard(parts)) {
    const el = h("div", "turn bot");
    for (const p of parts) el.append(conversationBlock(p.block, p.live));
    return el;
  }
  const el = h("div", "work" + (opened ? " open" : ""));
  const head = template("button", "whead", `<span class="wic"></span><b></b><span class="sum"></span><span class="st"></span>`);
  head.addEventListener("click", () => {
    const open = el.classList.toggle("open");
    head.setAttribute("aria-expanded", String(open));
    changed(open);
  });
  head.setAttribute("aria-expanded", String(opened));
  const body = h("div", "wbody");
  for (const p of parts) body.append(conversationBlock(p.block, p.live));
  el.append(head, body);
  paintWorkHead(el, parts);
  return el;
}

/// While running, the card header shows current activity; after completion, summarize step counts and affected work.
export function paintWorkHead(el: HTMLElement, parts: BlockPart[]) {
  const tools = parts.map((p) => p.block).filter((b): b is ToolBlock => b.kind === "tool");
  const last = parts[parts.length - 1];
  const running = parts.some((p) => p.live) || tools.some((b) => !b.done || b.background);
  const bad = tools.some((b) => b.error);
  el.classList.toggle("going", running);
  el.classList.toggle("bad", !running && bad);
  el.classList.toggle("ok", !running && !bad);

  const now = running && last.block.kind === "tool" ? last.block : null;
  const tally = countTools(tools);
  const name = now ? now.name : tally[0]?.[0] ?? "";
  const q = (sel: string) => el.querySelector(sel)!;
  q(".wic").innerHTML = icon(running && !now ? "sparkles" : toolIcon(name), 14);
  q("b").textContent = now ? toolLabel(now.name) : running ? t("chat.thinking") : tn(tools.length, "chat.work");
  q(".sum").textContent = now ? summary(now.name, now.input, now.json) : running ? "" : tallyText(tally);
  q(".st").innerHTML = running ? `<span class="spin"></span>` : icon(bad ? "x" : "check", 12);
}

export function conversationBlock(block: Block, live: boolean): HTMLElement {
  if (block.kind === "text") {
    const el = h("div", "md" + (live ? " typing" : ""));
    el.dataset.kind = "text";
    el.innerHTML = md(block.text);
    return el;
  }
  if (block.kind === "thinking") {
    // Without retained reasoning text, show only its label and no expansion control.
    const el = template(
      "details",
      "think" + (live ? " live" : "") + (block.text ? "" : " bare"),
      `<summary><span class="tic">${icon("brain", 14)}</span><b></b><span class="prev"></span></summary><div></div>`,
    );
    el.dataset.kind = "thinking";
    el.querySelector("b")!.textContent = t(live ? "chat.thinking" : "chat.thought");
    el.querySelector(".prev")!.textContent = peek(block.text);
    (el.lastElementChild as HTMLElement).textContent = block.text;
    return el;
  }
  // Collapsed tool rows reveal input and output when opened.
  const running = !block.done || block.background;
  const el = h("div", "tool" + (running ? " run" : block.error ? " bad" : " ok"));
  el.dataset.kind = "tool";
  el.dataset.tool = block.id;
  const head = template("button", "thead", `<span class="tic">${icon(toolIcon(block.name), 14)}</span><b></b><span class="sum"></span><span class="bgtag"></span><span class="st"></span>`);
  head.querySelector("b")!.textContent = toolLabel(block.name);
  head.querySelector(".sum")!.textContent = block.name === "ExitPlanMode" ? "" : summary(block.name, block.input, block.json);
  head.querySelector(".bgtag")!.textContent = block.background ? t("chat.bg.tag") : "";
  head.querySelector(".st")!.innerHTML = running ? `<span class="spin"></span>` : icon(block.error ? "x" : "check", 12);
  head.setAttribute("aria-expanded", "false");
  head.addEventListener("click", () => {
    head.setAttribute("aria-expanded", String(el.classList.toggle("open")));
  });
  el.append(head);
  const body = h("div", "tbody");
  if (block.name === "ExitPlanMode") {
    // Render plans directly as readable markdown.
    el.classList.add("open", "plan");
    head.setAttribute("aria-expanded", "true");
    const plan = h("div", "md");
    plan.innerHTML = md(String((block.input as { plan?: string })?.plan ?? ""));
    body.append(plan);
  } else if (block.error) {
    const failed = template("div", "tfail", `<span class="tic">${icon("x", 12)}</span><span></span>`);
    failed.lastElementChild!.textContent = t("chat.tool.failed");
    body.append(failed);

    const technical = h("div", "ttech");
    const input = inputView(block.name, block.input);
    if (input.childElementCount) technical.append(input);
    if (block.result !== null) {
      const out = h("pre", "tout");
      out.textContent = capError(fromBack(block.result));
      technical.append(out);
    }
    if (technical.childElementCount) {
      const details = template("details", "ttechnical", `<summary></summary>`);
      details.querySelector("summary")!.textContent = t("chat.tool.details");
      details.append(technical);
      body.append(details);
    }
  } else if (block.name === "Skill" && block.result) {
    // Collapse skill instructions while preserving readable markdown on expansion.
    const what = h("div", "md");
    what.innerHTML = md(capLines(block.result));
    body.append(what);
  } else {
    body.append(inputView(block.name, block.input));
    if (block.result !== null) {
      const out = h("pre", "tout");
      // Highlight unified diffs returned by tools like other edit diffs.
      if (isDiff(block.result)) {
        out.classList.add("tdiff");
        out.innerHTML = diffHtml(capLines(block.result));
      } else out.textContent = capLines(block.result);
      body.append(out);
    }
  }
  el.append(body);
  return el;
}
