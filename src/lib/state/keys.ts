// Global keyboard map. ⌘K palette · ⌘F find in the conversation · ⌘1 full office · ⌘2 full work · ⌘. focus · ⌘, settings ·
// ⌘N new request · ⌘[ / ⌘] previous / next request · ⇧⌘N new task · ⌘O open repo · ⌘↵ the one
// primary action (approve / run plan) · ⌥⌘↵ allow for the session · ⌘⌫ deny · ⇧⌘⌫ deny with a
// note (both also from the empty composer; on a plan both open the note) · 1-9 pick an option of the question that waits · G-sequences to rooms · J/K lists · Esc (a toast that
// waits for you, close, deselect, then stop the running turn: the first press asks, the second stops) · ?.
// ponytail: hand-rolled instead of tinykeys — one listener, sequences included.
import type { Approval } from '../gen/Approval';
import { showsPlan } from '../util/planRequest';
import { openWorkspace, resolveApprovals, runPlan, stopThreadByEsc } from './actions';
import { twinsIn } from './approvalGroups';
import { digitPick, isTextField } from '../util/questionKeys';
import { denyKeyInField, escStep, openOne } from './esc';
import { requests } from './requests.svelte';
import { store } from './store.svelte';
import { toasts } from './toasts.svelte';
import { ROOMS, isClosedThread, isRequestThread, ui, type DiffTarget, type Room } from './ui.svelte';

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

/** The request the keyboard acts on: the oldest pending one you can see in full. A question
 *  is one too: ⌘↵ sends its answer and its digits pick (`registerQuestionKeys`). */
export function keyTargetApproval(): Approval | null {
  const sheetAgent = ui.selectedAgentId;
  const pending = store.pendingApprovals
    .filter((a) => inThread(a) || a.agent_id === sheetAgent)
    .sort((a, b) => a.created_at - b.created_at);
  return pending[0] ?? null;
}

/** The requests one key press answers: the key target and, in the conversation, the other
 *  agents' requests that share its card (`groupSame`). */
function keyTargetIds(ap: Approval): string[] {
  const twins = inThread(ap) ? twinsIn(store.pendingApprovals.filter(inThread), ap) : [];
  return [ap, ...twins].map((a) => a.id);
}

/**
 * What ⌘⌫ does to the request the keyboard acts on: say no at once, or open the note field
 * first. ⇧⌘⌫ opens the note for every request. A Claude agent's plan always gets the note: its
 * card has no plain "no", since without a note Claude Code shows the same plan again.
 */
export function denyKeyAction(ap: Approval, shift: boolean): 'deny' | 'note' {
  return shift || showsPlan(ap.subject, store.agentMap.get(ap.agent_id)?.provider) ? 'note' : 'deny';
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

/**
 * What G C shows. From a request: that request's plan, when it has changes to show; a request
 * without one (a draft) gets the room's own choice, not what you looked at last. From anywhere
 * else: what you looked at last.
 */
export function keyTargetChanges(): DiffTarget {
  if (ui.room !== 'home' || !isRequestThread(ui.thread)) return ui.diffTarget;
  const p = store.planOf(ui.thread);
  const shows = !!p && p.status !== 'cancelled' && store.tasks.some((t) => t.plan_id === p.id && (t.status === 'done' || !!t.branch));
  return p && shows ? { kind: 'plan', id: p.id } : null;
}

/** What a question card does with the keys: the card knows what you picked and typed. */
export type QuestionKeys = { options: () => number; pick: (n: number) => void; send: () => void };
const questionKeys = new Map<string, QuestionKeys[]>();

/** A question card takes its keys while it shows; the card shown last wins (the sheet over the conversation). `questionKeysOf` reads them. */
export function registerQuestionKeys(approvalId: string, k: QuestionKeys): () => void {
  questionKeys.set(approvalId, [...(questionKeys.get(approvalId) ?? []), k]);
  return () => {
    const left = (questionKeys.get(approvalId) ?? []).filter((x) => x !== k);
    if (left.length) questionKeys.set(approvalId, left);
    else questionKeys.delete(approvalId);
  };
}
export const questionKeysOf = (id: string) => questionKeys.get(id)?.at(-1) ?? null;

const workspaceOpen = () => !!store.workspace;

export function installKeys(): () => void {
  let gPending = 0;

  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return;
    const mod = e.metaKey || e.ctrlKey;
    const typing = isTyping(e.target);
    const k = e.key.toLowerCase();
    // A dialog that is closing is not an overlay any more: an Esc while it fades goes to what is under it.
    const overlay = ui.paletteOpen || ui.settingsOpen || ui.shortcutsOpen || openOne(document.querySelectorAll('[aria-modal="true"]'));

    // ── the primary action (approve / run plan) ──
    // Other text fields (a deny note…) are not the composer: ⌘↵ there must not approve.
    // A held key acts once: its repeats would answer the next approval, then run the plan.
    if (mod && e.key === 'Enter' && !e.repeat && !overlay && (!typing || !!(e.target as HTMLElement).closest?.('[aria-label="Composer"]'))) {
      const ap = keyTargetApproval();
      if (ap?.subject.kind === 'question') {
        const q = questionKeysOf(ap.id);
        if (q && !e.altKey && !e.shiftKey) {
          e.preventDefault();
          q.send();
        }
        return;
      }
      if (e.altKey) {
        if (ap && ap.choices.includes('allow_session')) {
          e.preventDefault();
          resolveApprovals(keyTargetIds(ap), 'allow_session');
        }
        return;
      }
      if (e.shiftKey) return;
      if (ap && ap.choices.includes('allow_once')) {
        e.preventDefault();
        resolveApprovals(keyTargetIds(ap), 'allow_once');
        return;
      }
      const plan = keyTargetPlan();
      if (plan) {
        e.preventDefault();
        runPlan(plan);
      }
      return;
    }
    // In a text field these keys delete text; in the empty composer there is nothing to delete.
    const field = e.target as HTMLTextAreaElement | null;
    const denyHere = !typing || denyKeyInField({ composer: !!field?.closest?.('[aria-label="Composer"]'), empty: !field?.value });
    if (mod && e.key === 'Backspace' && !e.repeat && !overlay && denyHere) {
      const ap = keyTargetApproval();
      if (!ap || !ap.choices.includes('deny')) return;
      e.preventDefault();
      if (denyKeyAction(ap, e.shiftKey) === 'note') ui.denyNoteFor = ap.id;
      else resolveApprovals(keyTargetIds(ap), 'deny');
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
      // Find in the conversation on screen (also while typing: the composer is where you are).
      if (k === 'f' && !overlay && workspaceOpen() && ui.canFind) {
        e.preventDefault();
        ui.openFind();
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
      // One step per press: dismiss the toast that waits for you, close, deselect, leave the full
      // office, close the find bar, leave a closed request, and only then stop the running turn.
      const waiting = toasts.waiting;
      const step = escStep({
        repeat: e.repeat,
        artifactOpen: !!ui.artifactOpen,
        selection: !!ui.selection,
        mode: ui.mode,
        room: ui.room,
        popupOpen: openOne(document.querySelectorAll('[role="dialog"], [role="menu"]')),
        inOffice: !!(e.target as HTMLElement).closest?.('[role="application"]'),
        findOpen: ui.findOpen,
        readingClosed: isClosedThread(ui.thread),
        toastWaits: waiting !== undefined,
      });
      if (step === 'dismiss-toast') toasts.dismiss(waiting!.id);
      else if (step === 'close-artifact') ui.artifactOpen = null;
      else if (step === 'deselect') ui.selection = null;
      else if (step === 'leave-office') ui.mode = 'blend';
      else if (step === 'close-find') ui.findOpen = false;
      else if (step === 'leave-closed') ui.openRequest(ui.request);
      else if (step === 'stop') stopThreadByEsc(isRequestThread(ui.thread) || store.threadAgent(ui.thread) ? ui.thread : ui.request);
      return;
    }

    // ── the options of the question that waits: on a radio or a checkbox too (it is no text field) ──
    if (!overlay && /^[1-9]$/.test(e.key)) {
      const ap = keyTargetApproval();
      const q = ap?.subject.kind === 'question' ? questionKeysOf(ap.id) : null;
      const n = q ? digitPick({ key: e.key, mod, alt: e.altKey, shift: e.shiftKey }, isTextField(e.target as Element | null), q.options()) : null;
      if (q && n !== null) {
        e.preventDefault();
        q.pick(n);
        return;
      }
    }

    if (typing || mod || e.altKey || overlay) return;

    // ── G-sequences ──
    if (gPending && Date.now() - gPending < 1200) {
      gPending = 0;
      const dest = G_MAP[k];
      if (dest) {
        e.preventDefault();
        if (dest === 'settings') ui.settingsOpen = true;
        else {
          if (dest === 'changes') ui.diffTarget = keyTargetChanges();
          ui.go(dest);
        }
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
      ['⌘F', 'Find in the conversation (↵ next · ⇧↵ previous)'],
      ['⌘1', 'Full office'],
      ['⌘2', 'Full work (hide the office)'],
      ['⌘3', 'Neighbourhood (all your lots)'],
      ['⌘.', 'Focus: dim the office'],
      ['⌘N', 'New request (its own Lead, in parallel)'],
      ['⌘[ ⌘]', 'Previous · next request'],
      ['⇧⌘N', 'New task'],
      ['⌘O', 'Open repository'],
      ['⌘,', 'Settings'],
      ['Esc', 'Dismiss a toast that waits · close · deselect · press twice to stop the running turn'],
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
      ['Esc Esc', 'Stop the running turn (empty line)'],
      ['↑', 'Bring back what you sent (empty line)'],
      ['⇧⇥', 'Next mode: Ask · Plan · Auto · Direct'],
      ['⇧⇥', 'In a 1:1 with Claude: its next mode (ask for edits · accept edits · plan first)'],
      ['⌫', 'Back to the request (empty line, 1:1)'],
      ['@', 'Talk to one agent · mention a file'],
      ['#', 'Mention a task'],
      ['/', 'Actions: new request, stop, mode, model…'],
      ['⌘↵', 'Approve once · send the answer · run the plan'],
      ['⌥⌘↵', 'Allow for this session'],
      ['⌘⌫', 'Deny'],
      ['⇧⌘⌫', 'Deny with a note'],
      ['1 … 9', 'Pick an option of the question that waits'],
      ['J K', 'Next · previous'],
      ['[ ]', 'Previous · next file (Changes)'],
    ],
  },
];
