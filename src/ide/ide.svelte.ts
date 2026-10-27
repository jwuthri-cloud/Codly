// State of the workspace IDE overlay. One agent at a time: closing the overlay only hides it
// (tabs and unsaved edits stay), opening another agent starts clean. Every file operation goes
// through the core, which confines it to the agent's worktree; nothing here touches a path
// except as a root-relative string.
import { api } from '../lib/api';
import type { IdeChange } from '../lib/gen/IdeChange';
import type { IdeEntry } from '../lib/gen/IdeEntry';
import type { IdeInfo } from '../lib/gen/IdeInfo';
import { toast } from '../lib/state/toasts.svelte';

export type Rail = 'explorer' | 'search' | 'source';
export type Tab =
  | { id: 'home'; kind: 'home' }
  | { id: string; kind: 'file'; path: string; content: string; saved: string; loading: boolean; error: string | null; saving: boolean; line: number | null }
  | { id: string; kind: 'diff'; path: string };

const HOME: Tab = { id: 'home', kind: 'home' };
const msg = (e: unknown) => (e instanceof Error ? e.message : typeof e === 'string' ? e : 'Unknown error');

class Ide {
  visible = $state(false);
  agentId = $state<string | null>(null);
  rail = $state<Rail>('explorer');
  info = $state<IdeInfo | null>(null);
  tabs = $state<Tab[]>([HOME]);
  active = $state('home');
  /** Loaded folders by root-relative path ("" = the root). Absent = not loaded yet. */
  dirs = $state<Record<string, IdeEntry[]>>({});
  expanded = $state<Record<string, boolean>>({});
  changes = $state<IdeChange[]>([]);
  changeOf = $derived(new Map(this.changes.map((c) => [c.path, c.kind])));
  /** Bumped to focus the search field (Home → Search in code). */
  searchFocus = $state(0);

  get tab(): Tab {
    return this.tabs.find((t) => t.id === this.active) ?? HOME;
  }

  /** Open the IDE on an agent's workspace. */
  async open(agentId: string) {
    if (agentId !== this.agentId) {
      this.agentId = agentId;
      this.info = null;
      this.tabs = [HOME];
      this.active = 'home';
      this.dirs = {};
      this.expanded = {};
      this.changes = [];
      this.rail = 'explorer';
    }
    this.visible = true;
    await Promise.all([this.refreshInfo(), this.loadDir(''), this.refreshChanges()]);
  }

  close() {
    this.visible = false;
  }

  isDirty(t: Tab): boolean {
    return t.kind === 'file' && !t.loading && !t.error && t.content !== t.saved;
  }

  get dirtyCount(): number {
    return this.tabs.filter((t) => this.isDirty(t)).length;
  }

  private fail(what: string, e: unknown) {
    toast({ kind: 'error', title: `Couldn’t ${what}`, body: msg(e) });
  }

  async refreshInfo() {
    const id = this.agentId;
    if (!id) return;
    try {
      const info = await api.ideInfo(id);
      if (this.agentId === id) this.info = info;
    } catch (e) {
      this.fail('open the workspace', e);
    }
  }

  async refreshChanges() {
    const id = this.agentId;
    if (!id) return;
    try {
      const changes = await api.ideChanges(id);
      if (this.agentId === id) {
        this.changes = changes;
        if (this.info) this.info = { ...this.info, changed: changes.length };
      }
    } catch {
      /* the Home tab still works without git; the Source rail says so */
    }
  }

  async loadDir(path: string, force = false) {
    const id = this.agentId;
    if (!id || (!force && this.dirs[path])) return;
    try {
      const entries = await api.ideList(id, path);
      if (this.agentId === id) this.dirs[path] = entries;
    } catch (e) {
      this.fail(`list ${path || 'the workspace'}`, e);
    }
  }

  toggleDir(path: string) {
    this.expanded[path] = !this.expanded[path];
    if (this.expanded[path]) void this.loadDir(path);
  }

  /** Expand every folder above `path` so the explorer shows it. */
  async reveal(path: string) {
    const parts = path.split('/').slice(0, -1);
    for (let i = 1; i <= parts.length; i++) {
      const dir = parts.slice(0, i).join('/');
      this.expanded[dir] = true;
      await this.loadDir(dir);
    }
  }

  /** Open (or focus) a file tab, optionally on a 1-based line. */
  async openFile(path: string, line: number | null = null) {
    const id = this.agentId;
    if (!id) return;
    const tabId = `file:${path}`;
    if (!this.tabs.some((t) => t.id === tabId)) {
      this.tabs.push({ id: tabId, kind: 'file', path, content: '', saved: '', loading: true, error: null, saving: false, line });
    }
    this.active = tabId;
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!tab || tab.kind !== 'file') return;
    if (line != null) tab.line = line;
    if (!tab.loading) return;
    try {
      const f = await api.ideRead(id, path);
      tab.content = tab.saved = f.content;
    } catch (e) {
      tab.error = msg(e);
    } finally {
      tab.loading = false;
    }
  }

  /** The editor's text changed (it never mutates the tab itself). */
  edit(tabId: string, content: string) {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (tab?.kind === 'file') tab.content = content;
  }

  /** The editor moved to the requested line. */
  clearLine(tabId: string) {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (tab?.kind === 'file') tab.line = null;
  }

  async openDiff(path: string) {
    const tabId = `diff:${path}`;
    if (!this.tabs.some((t) => t.id === tabId)) this.tabs.push({ id: tabId, kind: 'diff', path });
    this.active = tabId;
  }

  closeTab(tabId: string) {
    if (tabId === 'home') return;
    const i = this.tabs.findIndex((t) => t.id === tabId);
    if (i < 0) return;
    this.tabs.splice(i, 1);
    if (this.active === tabId) this.active = (this.tabs[i] ?? this.tabs[i - 1] ?? HOME).id;
  }

  async save(tabId: string) {
    const id = this.agentId;
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!id || !tab || tab.kind !== 'file' || tab.saving || !this.isDirty(tab)) return;
    tab.saving = true;
    const content = tab.content;
    try {
      await api.ideWrite(id, tab.path, content);
      tab.saved = content;
      void this.refreshChanges();
    } catch (e) {
      this.fail(`save ${tab.path}`, e);
    } finally {
      tab.saving = false;
    }
  }

  /** Create an empty file (folders as needed) and open it. */
  async create(path: string): Promise<boolean> {
    const id = this.agentId;
    const rel = path.trim().replace(/^\.?\/+/, '');
    if (!id || !rel) return false;
    try {
      await api.ideCreate(id, rel);
    } catch (e) {
      this.fail('create the file', e);
      return false;
    }
    const slash = rel.lastIndexOf('/');
    await this.reloadDirs(slash < 0 ? '' : rel.slice(0, slash));
    void this.refreshChanges();
    await this.reveal(rel);
    await this.openFile(rel);
    return true;
  }

  /** Re-read a folder and every folder above it (a new file may have created some). */
  private async reloadDirs(dir: string) {
    const parts = dir ? dir.split('/') : [];
    for (let i = 0; i <= parts.length; i++) await this.loadDir(parts.slice(0, i).join('/'), true);
  }
}

export const ide = new Ide();
