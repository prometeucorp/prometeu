import type { GitHubItem, Issue, Workspace } from "./types";

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
