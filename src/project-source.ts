// Browser mirror of the project source rules in src-tauri/src/catalog/projects.rs. The browser mock
// uses it so that previews reject the same clone sources as the desktop.

const segment = /^[A-Za-z0-9][A-Za-z0-9_.-]*$/;

/** The canonical https or ssh URL of a project source, or null if the desktop rejects it. */
export function projectSource(source: string): string | null {
  if (!source || source.length > 4096 || /[\s\\%\u0000-\u001f\u007f-\u009f]/.test(source)) return null;
  let candidate = source;
  if (source.startsWith("git@")) {
    const rest = source.slice(4);
    const colon = rest.indexOf(":");
    if (colon < 0 || !/^[A-Za-z0-9][A-Za-z0-9.-]*$/.test(rest.slice(0, colon))) return null;
    candidate = `ssh://git@${rest.slice(0, colon)}/${rest.slice(colon + 1)}`;
  } else if (!source.includes("://")) {
    const short = source.startsWith("github.com/") ? source.slice("github.com/".length) : source;
    const parts = short.split("/");
    if (parts.length !== 2 || !parts.every(part => segment.test(part))) return null;
    candidate = `https://github.com/${short}`;
  }
  // Check raw segments before URL parsing can normalize a traversal away.
  if (candidate.split("/").some(part => part === "." || part === "..")) return null;
  let url: URL;
  try { url = new URL(candidate); } catch { return null; }
  const path = url.pathname.replace(/^\//, "");
  const ssh = url.protocol === "ssh:";
  if ((url.protocol !== "https:" && !ssh) || !url.hostname || url.password || url.search || url.hash
    || (ssh ? url.username !== "git" : url.username !== "")
    || !/^[A-Za-z0-9][A-Za-z0-9_./-]*$/.test(path)) return null;
  return url.href;
}

/** The folder name of a clone: the last path segment of the source, without `.git`. */
export function cloneName(source: string): string | null {
  const last = source.replace(/\/+$/, "").split(/[/:]/).pop() ?? "";
  const name = last.endsWith(".git") ? last.slice(0, -4) : last;
  return name && name.length <= 128 && segment.test(name) ? name : null;
}

/** Two sources name the same remote when their canonical URLs agree without `.git`. */
export function sameProjectRemote(first: string, second: string): boolean {
  const [a, b] = [projectSource(first), projectSource(second)];
  return a !== null && b !== null && a.replace(/\.git$/, "") === b.replace(/\.git$/, "");
}
