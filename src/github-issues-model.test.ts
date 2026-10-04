import { expect, it } from "vitest";
import { githubIssue, githubWorkspace } from "./github-issues-model";
import type { GitHubItem, Workspace } from "./types";

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
