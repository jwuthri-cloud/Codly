// Office tabs: the repos you keep at hand (remembered across launches), and how busy each one
// is. Every office with work keeps working in the core unless it is paused, so nothing works
// out of sight: an office that starts working gets its tab back, and closing a tab pauses it
// (its running tasks finish, nothing new starts). Reopening it resumes it.
import { api } from '../api';
import { updateSettings } from './actions';
import { store } from './store.svelte';
import type { Office } from '../gen/Office';
import type { Workspace } from '../gen/Workspace';

export interface OfficeTab {
  id: string;
  name: string;
  path: string;
}

const KEY = 'codly.officeTabs';

function load(): OfficeTab[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '[]') as OfficeTab[];
    return Array.isArray(v) ? v.filter((t) => t && t.id && t.path) : [];
  } catch {
    return [];
  }
}

class Offices {
  tabs = $state.raw<OfficeTab[]>(load());
  /** Your other offices (`api.offices` leaves the open one out), with their tasks in flight. */
  others = $state.raw<Office[]>([]);
  working = $derived(new Map(this.others.map((o) => [o.id, o.working])));

  private save() {
    try {
      localStorage.setItem(KEY, JSON.stringify(this.tabs));
    } catch {
      /* ignore */
    }
  }

  paused(id: string): boolean {
    return store.settings.paused || store.settings.paused_offices.includes(id);
  }

  /** Pause or resume one office (resuming also lifts the global pause). */
  pause(id: string, on: boolean) {
    const rest = store.settings.paused_offices.filter((x) => x !== id);
    return updateSettings(on ? { paused_offices: [...rest, id] } : { paused: false, paused_offices: rest });
  }

  /** The open repo always has a tab; reopening a closed one resumes it. */
  add(ws: Workspace) {
    const tab = { id: ws.id, name: ws.name, path: ws.repo_path };
    const i = this.tabs.findIndex((t) => t.id === ws.id || t.path === ws.repo_path);
    if (i < 0 && store.settings.paused_offices.includes(ws.id)) void this.pause(ws.id, false);
    if (i >= 0 && this.tabs[i].id === tab.id && this.tabs[i].name === tab.name && this.tabs[i].path === tab.path) return;
    this.tabs = i >= 0 ? this.tabs.map((t, j) => (j === i ? tab : t)) : [...this.tabs, tab];
    this.save();
  }

  close(id: string) {
    this.tabs = this.tabs.filter((t) => t.id !== id);
    this.save();
    if (!store.settings.paused_offices.includes(id)) void this.pause(id, true);
  }

  async refresh() {
    this.others = await api.offices().catch(() => this.others);
    // An office working without a tab (left open in an older version, or work that just came
    // in) gets its tab, so you see it and can pause it.
    const hidden = this.others.filter((o) => o.working > 0 && !this.paused(o.id) && !this.tabs.some((t) => t.id === o.id));
    if (hidden.length) {
      this.tabs = [...this.tabs, ...hidden.map((o) => ({ id: o.id, name: o.name, path: o.repo_path }))];
      this.save();
    }
  }
}

export const offices = new Offices();
