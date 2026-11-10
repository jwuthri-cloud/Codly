// Global keyboard map. ⌘K palette · ⌘1 full office · ⌘2 full work · ⌘. focus · ⌘, settings ·
// ⌘N new request · ⌘[ / ⌘] previous / next request · ⇧⌘N new task · ⌘O open repo · ⌘↵ the one
// primary action (approve / run plan) · ⌥⌘↵ allow for the session · ⌘⌫ deny · ⇧⌘⌫ deny with a
// note · G-sequences to rooms · J/K lists · Esc · ?.
// ponytail: hand-rolled instead of tinykeys — one listener, sequences included.
import type { Approval } from '../gen/Approval';
import { openWorkspace, resolveApproval, runPlan } from './actions';
import { requests } from './requests.svelte';
import { store } from './store.svelte';
import { ROOMS, isRequestThread, ui, type Room } from './ui.svelte';

export type ListNav = { next: () => void; prev: () => void; open?: () => void };
let navStack: ListNav[] = [];

/** Rooms register J/K/↵ handlers while visible; the most recent registration wins. */
export function registerListNav(n: ListNav): () => void {
  navStack.push(n);
  return () => {
    navStack = navStack.filter((x) => x !== n);
  };
}

const G_MAP: Record<string, Room | 'settings'> = Object.fromEntries([
  ...ROOMS.map((r) => [r.key.toLowerCase(), r.id] as const),
  ['s', 'settings'] as const,
]);

const isTyping = (t: EventTarget | null) => {
  const el = t as HTMLElement | null;
  if (!el) return false;
  return el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || !!el.closest?.('.xterm');
};

/** Whether an approval is on screen in the conversation you are reading. */
function inThread(a: Approval): boolean {
  if (ui.room !== 'home' || ui.mode === 'office' || ui.mode === 'map') return false;
  return isRequestThread(ui.thread) ? store.approvalThread(a) === ui.thread : `agent:${a.agent_id}` === ui.thread;
}

/** The approval the keyboard acts on: the oldest pending one you can see in full. */
export function keyTargetApproval(): Approval | null {
  const sheetAgent = ui.selectedAgentId;
  const pending = store.pendingApprovals
    .filter((a) => a.subject.kind !== 'question')
    .filter((a) => inThread(a) || a.agent_id === sheetAgent)
    .sort((a, b) => a.created_at - b.created_at);
  return pending[0] ?? null;
}

/** The plan ⌘↵ runs when no approval is waiting: the one of the request you are on. */
export function keyTargetPlan(): string | null {
  if ((ui.mode === 'office' && ui.room === 'home') || ui.mode === 'map') return null;
  if (ui.room !== 'home' && ui.room !== 'plan') return null;
  if (ui.room === 'home' && !isRequestThread(ui.thread)) return null;
  const chosen = ui.room === 'plan' && ui.planId ? store.planMap.get(ui.planId) : null;
  const p = chosen && (chosen.thread || 'lead') === ui.request ? chosen : store.planOf(ui.request);
  return p && p.status === 'awaiting_approval' ? p.id : null;
}

const workspaceOpen = () => !!store.workspace;

export function installKeys(): () => void {
  let gPending = 0;

  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return;
    const mod = e.metaKey || e.ctrlKey;
    const typing = isTyping(e.target);
    const k = e.key.toLowerCase();
    const overlay = ui.paletteOpen || ui.settingsOpen || ui.shortcutsOpen || !!document.querySelector('[aria-modal="true"]');

    // ── the primary action (approve / run plan) ──
    // Other text fields (a deny note…) are not the composer: ⌘↵ there must not approve.
    if (mod && e.key === 'Enter' && !overlay && (!typing || !!(e.target as HTMLElement).closest?.('[aria-label="Composer"]'))) {
      const ap = keyTargetApproval();
      if (e.altKey) {
        if (ap && ap.choices.includes('allow_session')) {
          e.preventDefault();
          resolveApproval(ap.id, 'allow_session');
        }
        return;
      }
      if (e.shiftKey) return;
      if (ap && ap.choices.includes('allow_once')) {
        e.preventDefault();
        resolveApproval(ap.id, 'allow_once');
        return;
      }
      const plan = keyTargetPlan();
      if (plan) {
        e.preventDefault();
        runPlan(plan);
      }
      return;
    }
    if (mod && e.key === 'Backspace' && !overlay && !typing) {
      const ap = keyTargetApproval();
      if (!ap || !ap.choices.includes('deny')) return;
      e.preventDefault();
      if (e.shiftKey) ui.denyNoteFor = ap.id;
      else resolveApproval(ap.id, 'deny');
      return;
    }

    // ── requests: ⌘N new · ⌘[ / ⌘] switch (also while typing: the composer is where you are) ──
    if (mod && !e.altKey && !overlay && workspaceOpen()) {
      if (k === 'n' && !e.shiftKey) {
        e.preventDefault();
        ui.newRequest();
        return;
      }
      if ((e.key === '[' || e.key === ']') && !e.shiftKey) {
        e.preventDefault();
        if (ui.room !== 'home' || !isRequestThread(ui.thread)) ui.openRequest(ui.request);
        else requests.step(e.key === ']' ? 1 : -1);
        return;
      }
      if (k === 'n' && e.shiftKey && !typing) {
        e.preventDefault();
        ui.openPalette('+');
        return;
      }
    }

    // ── modifier shortcuts: work everywhere ──
    if (mod && !e.shiftKey && !e.altKey) {
      if (k === 'k') {
        e.preventDefault();
        if (ui.paletteOpen) ui.paletteOpen = false;
        else ui.openPalette();
        return;
      }
      if (k === '1' || k === '2' || k === '3') {
        e.preventDefault();
        ui.toggleMode(k === '1' ? 'office' : k === '2' ? 'work' : 'map');
        return;
      }
      if (k === '.') {
        e.preventDefault();
        ui.focus = !ui.focus;
        return;
      }
      if (k === ',') {
        e.preventDefault();
        ui.settingsOpen = true;
        return;
      }
      if (k === 'o' && !typing) {
        e.preventDefault();
        openWorkspace();
        return;
      }
    }

    if (e.key === 'Escape') {
      if (overlay) return; // overlays handle their own Esc
      if (typing) {
        (e.target as HTMLElement).blur();
        return;
      }
      if (ui.artifactOpen) ui.artifactOpen = null;
      else if (ui.selection) ui.selection = null;
      else if (ui.mode === 'office' || ui.mode === 'map') ui.mode = 'blend';
      return;
    }

    if (typing || mod || e.altKey || overlay) return;

    // ── G-sequences ──
    if (gPending && Date.now() - gPending < 1200) {
      gPending = 0;
      const dest = G_MAP[k];
      if (dest) {
        e.preventDefault();
        if (dest === 'settings') ui.settingsOpen = true;
        else ui.go(dest);
      }
      return;
    }
    if (k === 'g' && !e.shiftKey) {
      gPending = Date.now();
      return;
    }

    if (e.key === '?') {
      ui.shortcutsOpen = true;
      return;
    }
    const nav = navStack[navStack.length - 1];
    if (nav && (k === 'j' || e.key === 'ArrowDown')) {
      e.preventDefault();
      nav.next();
    } else if (nav && (k === 'k' || e.key === 'ArrowUp')) {
      e.preventDefault();
      nav.prev();
    } else if (nav?.open && e.key === 'Enter') {
      e.preventDefault();
      nav.open();
    }
  };

  window.addEventListener('keydown', onKey);
  return () => window.removeEventListener('keydown', onKey);
}

export const SHORTCUTS: { group: string; items: [string, string][] }[] = [
  {
    group: 'Everywhere',
    items: [
      ['⌘K', 'Search and commands'],
      ['⌘1', 'Full office'],
      ['⌘2', 'Full work (hide the office)'],
      ['⌘.', 'Focus: dim the office'],
      ['⌘N', 'New request (its own Lead, in parallel)'],
      ['⌘[ ⌘]', 'Previous · next request'],
      ['⇧⌘N', 'New task'],
      ['⌘O', 'Open repository'],
      ['⌘,', 'Settings'],
      ['Esc', 'Close · deselect'],
      ['?', 'This sheet'],
    ],
  },
  {
    group: 'Go to',
    items: ROOMS.map((r) => [`G ${r.key}`, r.label] as [string, string]),
  },
  {
    group: 'Conversation & approvals',
    items: [
      ['↵', 'Send (queues if the agent is mid-turn)'],
      ['⇧↵', 'New line'],
      ['@', 'Talk to one agent'],
      ['#', 'Mention a task'],
      ['⌘↵', 'Approve once · run the plan'],
      ['⌥⌘↵', 'Allow for this session'],
      ['⌘⌫', 'Deny'],
      ['⇧⌘⌫', 'Deny with a note'],
      ['J K', 'Next · previous'],
    ],
  },
];
