import { invoke } from "./ipc";
import { fromBack } from "./i18n";
import { codeMapLabels } from "./code-map-labels";
import * as background from "./background";
import { codeMapView, type CodeMapViewState } from "./components/code-map";
import type { CodeMapRoot } from "./code-map-types";

/** One visible explorer coalesces requests; stale roots and queries cannot replace its current snapshot. */
export class WorkspaceCodeMap {
  readonly view;
  readonly root: HTMLElement;
  private id: string | null = null;
  private visible = false;
  private rootId: string | undefined;
  private path: string | undefined;
  private loadedQuery = "";
  private loadedPath: string | undefined;
  private state: CodeMapViewState = { data: null, query: "" };
  private revision = 0;
  private running = false;
  private pending = false;
  private force = false;
  private timer = 0;

  constructor(private readonly options: {
    openSource: (root: CodeMapRoot, path: string, line: number) => void;
    expanded: (value: boolean) => void;
  }) {
    this.view = codeMapView({
      labels: codeMapLabels(), snapshot: this.state,
      queryChanged: query => {
        // Keep the completed graph while typing; pending input must not rebuild it on every key.
        this.state = { ...this.state, query };
        this.path = undefined; this.queue(250);
      },
      rootChanged: root => {
        this.rootId = root; this.path = undefined;
        this.state = { ...this.state, data: null }; this.queue();
      },
      inspect: path => { this.path = path; this.state.query = path; this.queue(); },
      sync: () => this.queue(0, true), retry: () => this.queue(),
      openSource: (path, line) => {
        const root = this.state.data?.roots.find(root => root.id === this.state.data?.root_id);
        if (root) { this.expand(false); this.options.openSource(root, path, line); }
      },
      expandedChanged: value => this.expand(value),
    });
    this.root = this.view.root;
    background.subscribe(() => { if (this.visible && background.foreground(background.currentOrDocument())) this.refresh(); });
  }

  show(id: string) {
    if (this.visible && this.id === id) return;
    if (this.id !== id) {
      this.rootId = undefined; this.path = undefined;
      this.state = { data: null, query: "" };
    }
    this.id = id; this.visible = true; this.queue();
  }

  hide() {
    this.visible = false; this.pending = false; this.force = false; this.revision++;
    clearTimeout(this.timer); this.expand(false);
  }

  refresh() { if (this.visible) this.queue(); }

  private expand(value: boolean) {
    this.state.expanded = value; this.options.expanded(value); this.view.update(this.state);
  }

  private queue(delay = 0, force = false) {
    clearTimeout(this.timer); this.revision++; this.pending = true; this.force ||= force;
    this.state = { ...this.state, loading: true, syncing: this.force, error: undefined };
    this.view.update(this.state);
    if (!this.visible) return;
    this.timer = window.setTimeout(() => void this.read(), delay);
  }

  private async read() {
    if (!this.visible || !this.id || this.running || !this.pending) return;
    this.running = true; this.pending = false;
    const mine = this.revision, force = this.force;
    const query = this.state.query.trim(), path = this.path;
    this.force = false;
    try {
      const data = await invoke("workspace_code_map", {
        id: this.id, root: this.rootId, query, path, force,
      });
      if (this.visible && mine === this.revision) {
        const previous = this.state.data;
        if (previous?.root_id === data.root_id) {
          if (previous.overview.generation === data.overview.generation) data.overview = previous.overview;
          if (this.loadedQuery === query && this.loadedPath === path && previous.result?.generation === data.result?.generation) {
            data.result = previous.result;
          }
        }
        this.loadedQuery = query; this.loadedPath = path;
        this.rootId = data.root_id;
        this.state = { ...this.state, data, loading: false, syncing: false, error: undefined };
      }
    } catch (error) {
      if (this.visible && mine === this.revision) this.state = { ...this.state, loading: false, syncing: false, error: fromBack(error) };
    } finally {
      this.running = false;
      if (this.visible) {
        this.view.update(this.state);
        if (this.pending) void this.read();
        else this.timer = window.setTimeout(() => {
          // Background windows retain the last snapshot without indexing repeatedly.
          if (background.foreground(background.currentOrDocument())) this.queue();
          else this.waitForForeground();
        }, 15_000);
      }
    }
  }

  private waitForForeground() {
    if (!this.visible) return;
    this.timer = window.setTimeout(() => {
      if (background.foreground(background.currentOrDocument())) this.queue();
      else this.waitForForeground();
    }, 15_000);
  }
}
