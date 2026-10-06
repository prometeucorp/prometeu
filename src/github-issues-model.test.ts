import { expect, it } from "vitest";
import { githubIssue, githubScopes, githubWorkspace, loadGitHubInbox } from "./github-issues-model";
import type { GitHubItem, GitHubIssues, GitHubScope, Workspace } from "./types";

const item: GitHubItem = { id: "github:org/repo/issues/12", identifier: "org/repo#12", title: "Fix refs / names", url: "https://github.com/org/repo/issues/12", description: "Details", repository: "org/repo", number: 12, kind: "issue", author: "user", draft: false, updated_at: "2026-10-03T00:00:00Z", labels: ["bug"] };

it("keeps source identity and context while deriving a safe branch for the existing launcher", () => {
  const seed = githubIssue(item);
  expect(seed).toMatchObject({ id: item.id, url: item.url, description: "Details", branch_name: "github-12-fix-refs-names" });
  expect(githubIssue({ ...item, title: "////" }).branch_name).toBe("github-12-issue");
});

it("does not confuse equal issue numbers across providers or repositories", () => {
  const workspace = { id: "w", issue: { id: item.id, url: item.url }, archived: false, cleaned: false, repos: [] } as unknown as Workspace;
  expect(githubWorkspace(item, [workspace])).toBe(workspace);
  expect(githubWorkspace({ ...item, id: "github:other/repo/issues/12", url: "https://github.com/other/repo/issues/12" }, [workspace])).toBeUndefined();
  expect(githubWorkspace(item, [{ ...workspace, archived: true }])).toBeUndefined();
  expect(githubWorkspace(item, [{ ...workspace, cleaned: true }])).toBeUndefined();
  const pr = { ...item, kind: "pr" as const, id: "github:org/repo/pull/12", url: "https://github.com/org/repo/pull/12" };
  expect(githubWorkspace(pr, [{ ...workspace, issue: null, repo: "/clone", repos: [{ path: "/clone", pr: { number: 12 } }] } as unknown as Workspace])).toBeUndefined();
});

const inboxList = (login = "user"): GitHubIssues => ({ login, repositories: ["org/repo"], items: [item], fetched_at: 1, truncated: false });

it("starts all scopes before awaiting results and fills counts without selecting each tab", async () => {
  const pending = new Map<GitHubScope, (value: GitHubIssues) => void>();
  const loading = loadGitHubInbox(scope => new Promise(resolve => pending.set(scope, resolve)));
  expect([...pending.keys()]).toEqual([...githubScopes]);
  for (const [scope, resolve] of [...pending].reverse()) resolve({ ...inboxList(), items: scope === "available" ? [] : [item] });
  const result = await loading;
  expect([...result.lists].map(([scope, list]) => [scope, list.items.length])).toEqual([
    ["mine", 1], ["available", 0], ["authored", 1], ["reviews", 1],
  ]);
  expect(result.errors.size).toBe(0);
  expect(result.repositories).toEqual(["org/repo"]);
});

it("keeps available scopes when another fails and replaces failed results on retry", async () => {
  const error = new Error("Search unavailable");
  const result = await loadGitHubInbox(async scope => {
    if (scope === "available") throw error;
    return inboxList();
  });
  expect([...result.lists.keys()]).toEqual(["mine", "authored", "reviews"]);
  expect(result.errors.get("available")).toBe(error);
  const retry = await loadGitHubInbox(async () => inboxList());
  expect(retry.lists.size).toBe(4);
  expect(retry.errors.size).toBe(0);
  const offline = await loadGitHubInbox(async () => { throw error; });
  expect(offline.lists.size).toBe(0);
  expect(offline.login).toBe("");
  expect(offline.errors.size).toBe(4);
});

it("rejects a batch spanning a reconnection as another account", async () => {
  await expect(loadGitHubInbox(async scope => inboxList(scope === "reviews" ? "other-user" : "user")))
    .rejects.toBe('i18n:{"code":"err.github.accountChanged"}');
});

it("discards earlier successes if the connection is lost during the batch", async () => {
  for (const error of ['i18n:{"code":"err.github.auth"}', 'i18n:{"code":"err.github.off"}', 'i18n:{"code":"err.github.accountChanged"}']) {
    await expect(loadGitHubInbox(async scope => {
      if (scope === "reviews") throw error;
      return inboxList();
    })).rejects.toBe(error);
  }
});
