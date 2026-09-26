import { invoke } from "./ipc";
import { reviews } from "./review-store";
import { place, reviewBatch, type ReviewAnchor, type ReviewDraft, type ReviewNote, type ReviewScope, type ReviewEditorDraft } from "./review-comments";
import { encodeReviewContext, reviewContext, safeReviewPath } from "./review-context";
import { reviewSummary } from "./components/git/review-summary";
import { reviewEditor, reviewError, type ReviewEntry } from "./components/git/review-note";
import type { DiffReview } from "./components/git/diff-review";
import { formDialog } from "./ui";
import { fromBack, t } from "./i18n";
import { repoPath, type Change, type Workspace } from "./types";

type Snapshot = { workspace: string; repo: string; scope: ReviewScope; reference: string; files: Change[] };
const matches = (s: Snapshot, id: string, a: ReviewAnchor) => s.workspace === id && s.repo === a.repo && s.scope === a.scope && s.reference === a.reference;
export function workspaceReview(context: {
  workspace: () => Workspace | undefined; changed: () => void; say: (text: string, error?: boolean) => void;
  send: (at: HTMLElement, draft: ReviewDraft, close: () => void) => void;
  jump: (anchor: ReviewAnchor, note: string) => void;
}) {
  let snapshots: Snapshot[] = [];
  const drafts = new Map<string, { workspace: string; draft: ReviewEditorDraft }>();
  const draftsOf = (workspace: string) => [...drafts.values()].filter(d => d.workspace === workspace).map(d => d.draft);
  const draftKey = (workspace: string, a: ReviewAnchor) => JSON.stringify([workspace, a.repo, a.path, a.scope, a.reference]);
  function draftChanged(workspace: string, anchor: ReviewAnchor, body: string | null) {
    const key = draftKey(workspace, anchor), had = drafts.has(key);
    if (body === null) drafts.delete(key); else drafts.set(key, { workspace, draft: { anchor, body } });
    // Keystrokes retain writing outside the DOM without repeatedly redrawing the diff.
    if (had !== drafts.has(key)) refresh();
  }
  let summary: ReturnType<typeof reviewSummary> | undefined;
  let summaryWorkspace: string | undefined;
  let editorDialog: ReturnType<typeof formDialog> | undefined;
  let openVersion = 0;
  const writable = (ws: Workspace) => {
    const current = context.workspace();
    return current?.id === ws.id && !current.remote && !current.cleaned && !current.preparing && !current.failed;
  };
  function mutate(ws: Workspace, run: () => void) {
    if (!writable(ws)) return;
    try { run(); } catch (e) {
      context.say(reviewError(e), true);
      // A failed disk write already kept this edit in memory. Closing avoids creating it twice.
      if (!(e instanceof Error) || e.message !== "review.storage.write") throw e;
    }
  }
  function entries(ws: Workspace, snapshot?: Snapshot): ReviewEntry[] {
    return reviews.list(ws.id).map(note => {
      const source = snapshot ?? snapshots.find(s => matches(s, ws.id, note.anchor));
      const patch = source ? source.files.find(f => f.path === note.anchor.path)?.patch ?? null : undefined;
      const placement = patch === undefined ? { kind: "unknown" as const } : place(note.anchor, patch);
      const sentPlacement = note.sent && patch !== undefined ? place(note.sent.anchor, patch) : null;
      return { note, placement, changedSinceSent: note.state === "sent" && (sentPlacement?.kind === "changed" || (!!note.sent && !note.sent.anchor.match && sentPlacement?.kind === "attached" && sentPlacement.anchor.stamp !== note.sent.anchor.stamp)) };
    });
  }
  function edit(ws: Workspace, note: ReviewNote) {
    editorDialog?.close();
    const dialog = formDialog({ title: t("review.edit"), save: t("review.save"), cancel: t("review.cancel"), submit: async () => {}, error: reviewError });
    editorDialog = dialog;
    dialog.save.remove();
    const editor = reviewEditor(note.anchor, note.body, body => { mutate(ws, () => reviews.edit(ws.id, note.id, body)); dialog.close(); }, dialog.close);
    dialog.body.append(editor.root); dialog.open(); editor.focus();
  }
  const actions = (ws: Workspace) => ({
    edit: (note: ReviewNote) => edit(ws, note),
    remove: (note: ReviewNote) => mutate(ws, () => reviews.remove(ws.id, note.id)),
    state: (note: ReviewNote, state: "draft" | "resolved") => mutate(ws, () => reviews.state(ws.id, note.id, state)),
  });
  function batch(ws: Workspace, notes: ReviewNote[]) {
    const draft = reviewBatch(ws.id, crypto.randomUUID(), notes, a => {
      const repo = ws.repos.find(r => r.name === a.repo);
      if (!repo || (repo.worktree !== ws.worktree && !repo.worktree.startsWith(ws.worktree + "/"))) throw new Error("review.selection");
      const path = repoPath(ws, a.repo, a.path);
      if (!safeReviewPath(path)) throw new Error("review.selection");
      return path;
    });
    encodeReviewContext(reviewContext(draft)); return draft;
  }
  const refresh = () => {
    const ws = context.workspace(); if (summary && ws && ws.id === summaryWorkspace) summary.update(entries(ws), draftsOf(ws.id));
    context.changed();
  };
  reviews.subscribe(id => { if (id === context.workspace()?.id) refresh(); });
  return {
    count: (id: string, repo?: string, path?: string) => reviews.list(id).filter(n => n.state !== "resolved" && (repo === undefined || n.anchor.repo === repo) && (path === undefined || n.anchor.path === path)).length + draftsOf(id).filter(d => (repo === undefined || d.anchor.repo === repo) && (path === undefined || d.anchor.path === path)).length,
    snapshot(ws: Workspace, repo: string, scope: ReviewScope, reference: string, files: Change[]): DiffReview | undefined {
      if (!writable(ws) || reviews.blocked(ws.id)) return;
      const snapshot = { workspace: ws.id, repo, scope, reference, files };
      snapshots = snapshots.filter(s => !(s.workspace === ws.id && s.repo === repo && s.scope === scope && s.reference === reference));
      snapshots.push(snapshot);
      return { scope, reference, ...actions(ws), closed: context.changed,
        drafts: draftsOf(ws.id), draftChanged: (anchor, body) => draftChanged(ws.id, anchor, body),
        entries: entries(ws, snapshot).filter(e => e.note.anchor.repo === repo && (e.placement.kind === "attached" || matches(snapshot, ws.id, e.note.anchor))),
        add: (anchor, body) => mutate(ws, () => reviews.add(ws.id, anchor, body)),
      };
    },
    async open() {
      const ws = context.workspace(); if (!ws || !writable(ws)) return;
      summary?.close();
      const version = ++openVersion;
      const notes = reviews.list(ws.id), requests = new Map<string, ReviewAnchor>();
      for (const n of notes) requests.set(JSON.stringify([n.anchor.repo, n.anchor.scope, n.anchor.reference]), n.anchor);
      const results = await Promise.allSettled([...requests.values()].map(async a => {
        const repo = ws.repos.findIndex(r => r.name === a.repo);
        if (repo < 0) return { workspace: ws.id, repo: a.repo, scope: a.scope, reference: a.reference, files: [] };
        const diff = await invoke("workspace_git_diff", { id: ws.id, repo, scope: a.scope, path: null, reference: a.reference || null });
        return { workspace: ws.id, repo: a.repo, scope: a.scope, reference: a.reference, files: diff.files };
      }));
      if (!writable(ws) || version !== openVersion) return;
      snapshots = snapshots.filter(s => s.workspace !== ws.id);
      for (const result of results) {
        if (result.status === "fulfilled") snapshots.push(result.value); else context.say(fromBack(result.reason), true);
      }
      summaryWorkspace = ws.id;
      summary = reviewSummary({ ...actions(ws), entries: entries(ws), blocked: reviews.blocked(ws.id), drafts: draftsOf(ws.id),
        resume: draft => {
          summary?.close(); editorDialog?.close();
          const dialog = formDialog({ title: t("review.note"), save: t("review.save"), cancel: t("review.close"), submit: async () => {}, error: reviewError, closed: context.changed });
          editorDialog = dialog;
          dialog.save.remove();
          const editor = reviewEditor(draft.anchor, draft.body, body => {
            mutate(ws, () => reviews.add(ws.id, draft.anchor, body));
            draftChanged(ws.id, draft.anchor, null); dialog.close();
          }, () => { draftChanged(ws.id, draft.anchor, null); dialog.close(); }, body => draftChanged(ws.id, draft.anchor, body));
          dialog.body.append(editor.root); dialog.open(); editor.focus();
        },
        validate: notes => { if (!notes.length) return null; try { batch(ws, notes); return null; } catch (e) { return reviewError(e); } },
        send: (at, notes, close) => { if (writable(ws)) context.send(at, batch(ws, notes), close); },
        resolveChanged: notes => mutate(ws, () => { for (const note of notes) reviews.state(ws.id, note.id, "resolved"); }),
        jump: entry => { summary?.close(); context.jump(entry.placement.kind === "attached" ? entry.placement.anchor : entry.note.anchor, entry.note.id); },
        closed: () => { summary = undefined; summaryWorkspace = undefined; },
      });
    },
    leave() { openVersion++; summary?.close(); editorDialog?.close(); },
    forget(alive: Set<string>) {
      snapshots = snapshots.filter(s => alive.has(s.workspace));
      for (const [key, value] of drafts) if (!alive.has(value.workspace)) drafts.delete(key);
      try { reviews.prune(alive); } catch { context.say(t("review.storage.write"), true); }
    },
  };
}
