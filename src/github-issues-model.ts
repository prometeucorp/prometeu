import type { GitHubItem, GitHubIssues, GitHubScope, Issue, Workspace } from "./types";

// Preserve the existing workspace reference format; namespace GitHub IDs at the adapter boundary.
export function githubIssue(item: GitHubItem): Issue {
  return {
    id: item.id, identifier: item.identifier, title: item.title, url: item.url,
    description: item.description, branch_name: `github-${item.number}-${item.title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 48) || "issue"}`,
    priority: 0, priority_label: "", state: { name: "", kind: "unstarted", color: "" },
    team: item.repository, project: item.repository,
    labels: item.labels.map(name => ({ name, color: "" })), updated_at: item.updated_at,
  };
}

export function githubWorkspace(item: GitHubItem, workspaces: Workspace[]): Workspace | undefined {
  return workspaces.find(workspace => !workspace.archived && !workspace.cleaned && (
    workspace.issue?.id === item.id || workspace.issue?.url === item.url
  ));
}

export const githubScopes = ["mine", "available", "authored", "reviews"] as const;

// Fetch the whole inbox together; a failed scope must not hide successful scopes or retain old data.
export async function loadGitHubInbox(fetch: (scope: GitHubScope) => Promise<GitHubIssues>) {
  const results = await Promise.allSettled(githubScopes.map(fetch));
  const lists = new Map<GitHubScope, GitHubIssues>();
  const errors = new Map<GitHubScope, unknown>();
  let account: GitHubIssues | undefined;
  for (const [index, result] of results.entries()) {
    const scope = githubScopes[index];
    if (result.status === "rejected") {
      // A lost connection invalidates the whole batch, including earlier successful queries.
      if (result.reason === 'i18n:{"code":"err.github.auth"}' || result.reason === 'i18n:{"code":"err.github.off"}') throw result.reason;
      errors.set(scope, result.reason); continue;
    }
    account ??= result.value;
    // Reconnecting as someone else during the batch must not combine two accounts' lists.
    if (result.value.login !== account.login) throw 'i18n:{"code":"err.github.accountChanged"}';
    lists.set(scope, result.value);
  }
  return { lists, errors, login: account?.login ?? "", repositories: account?.repositories ?? [] };
}
