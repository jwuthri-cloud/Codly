// Office tabs: the repos you keep at hand (remembered across launches), and how busy each one
// is. Every office with work keeps working in the core; a tab only says where you look.
// Closing a tab hides it, nothing stops.
import { api } from '../api';
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

  /** The open repo always has a tab. */
  add(ws: Workspace) {
    const tab = { id: ws.id, name: ws.name, path: ws.repo_path };
    const i = this.tabs.findIndex((t) => t.id === ws.id || t.path === ws.repo_path);
    if (i >= 0 && this.tabs[i].id === tab.id && this.tabs[i].name === tab.name && this.tabs[i].path === tab.path) return;
    this.tabs = i >= 0 ? this.tabs.map((t, j) => (j === i ? tab : t)) : [...this.tabs, tab];
    this.save();
  }

  close(id: string) {
    this.tabs = this.tabs.filter((t) => t.id !== id);
    this.save();
  }

  async refresh() {
    this.others = await api.offices().catch(() => this.others);
  }
}

export const offices = new Offices();
