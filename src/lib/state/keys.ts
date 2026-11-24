// Global keyboard map. ⌘K palette · ⌘F find in the conversation · ⌘1 full office · ⌘2 full work · ⌘. focus · ⌘, settings ·
// ⌘N new request · ⌘[ / ⌘] previous / next request · ⇧⌘N new task · ⌘O open repo · ⌘↵ the one
// primary action (approve / run plan) · ⌥⌘↵ allow for the session · ⌘⌫ deny · ⇧⌘⌫ deny with a
// note (both also from the empty composer; on a plan both open the note) · 1-9 pick an option of the question that waits · G-sequences to rooms · J/K lists · Esc (a toast that
// waits for you, close, deselect, then stop the running turn: the first press asks, the second stops;
// in a 1:1 chat with an agent that rests, two presses open "Undo a turn") · ?.
// ponytail: hand-rolled instead of tinykeys — one listener, sequences included.
import type { Approval } from '../gen/Approval';
import { showsPlan } from '../util/planRequest';
import { openWorkspace, resolveApprovals, runPlan, stopThreadByEsc } from './actions';
import { twinsIn } from './approvalGroups';
import { keyActOf, restsAfterAnswer, type CardKey, type KeyAct } from '../util/approvalCard';
import { digitPick, isTextField } from '../util/questionKeys';
import { rewindLayer } from '../../home/rewindLayer.svelte';
import { denyKeyInField, escPair, escStep, mayRewind, openOne } from './esc';
import { busy } from './queue';
import { requests } from './requests.svelte';
import { store } from './store.svelte';
import { takeovers } from './takeover.svelte';
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

/** The request the keyboard acts on: the oldest pending one on screen. A key answers it only
 *  once a card of it has shown its title, what it asks and its buttons, uncovered and still, for
 *  a moment (`keyGate`); a key pressed before only brings it into view, or does nothing while the
 *  card settles. A question is one too: ⌘↵ sends its answer and its digits pick
 *  (`registerQuestionKeys`). */
export function keyTargetApproval(): Approval | null {
  const sheetAgent = ui.selectedAgentId;
  const pending = store.pendingApprovals
    .filter((a) => inThread(a) || a.agent_id === sheetAgent)
    .sort((a, b) => a.created_at - b.created_at);
  return pending[0] ?? null;
}

/** Each shown card of a request says what a key does to it now (`keyAct`), read at the press. */
const cardViews = new Map<string, (() => KeyAct)[]>();

/** A card registers while it shows, for its request and the twins it answers too. */
export function registerCardView(approvalIds: string[], act: () => KeyAct): () => void {
  for (const id of approvalIds) cardViews.set(id, [...(cardViews.get(id) ?? []), act]);
  return () => {
    for (const id of approvalIds) {
      const left = (cardViews.get(id) ?? []).filter((x) => x !== act);
      if (left.length) cardViews.set(id, left);
      else cardViews.delete(id);
    }
  };
}

/** What a key does to the request now, over the cards that show it (`keyActOf`). */
export const cardKeyAct = (approvalId: string): KeyAct => keyActOf((cardViews.get(approvalId) ?? []).map((act) => act()));

/** When an answer last went, by a key or by a click on a card (or a key ran a plan): the next key
 *  press, and a click on a card's buttons, does nothing for a moment (`restsAfterAnswer`). */
let answeredAt: number | null = null;

/** An answer went now: the keys and the cards' buttons rest for a moment. */
export function noteAnswer(): void {
  answeredAt = performance.now();
}
/** When the last answer went (`noteAnswer`); null: none yet. */
export const lastAnswerAt = (): number | null => answeredAt;
/** A key or a click answered a moment ago: no key acts on anything (`restsAfterAnswer`). */
const rests = (): boolean => restsAfterAnswer(performance.now(), answeredAt);

/** True: the key must not answer the request. An answer went a moment ago (the store may not
 *  know it yet, and the next card has not settled), it brought its card into view, or the card
 *  settles (then it does nothing). A digit and the send key of a question card ask the card
 *  whose question keys they use (`QuestionKeys.act`): they act on more than its three parts. */
function keyGate(ap: Approval, key: CardKey = 'other'): boolean {
  if (rests()) return true;
  const q = key !== 'other' ? questionKeysOf(ap.id) : null;
  const act = q ? q.act(key) : cardKeyAct(ap.id);
  if (act === 'reveal') ui.revealApproval = ap.id;
  return act !== 'answer';
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

/** A "Run plan" button on screen says what ⌘↵ does to its plan now (`keyAct`, its box as a
 *  card's part), and brings itself into view. */
export type PlanKey = { act: () => KeyAct; reveal: () => void };
const planKeys = new Map<string, PlanKey[]>();

/** A "Run plan" button registers while it shows (the plan card, the Plan room). */
export function registerPlanKey(planId: string, k: PlanKey): () => void {
  planKeys.set(planId, [...(planKeys.get(planId) ?? []), k]);
  return () => {
    const left = (planKeys.get(planId) ?? []).filter((x) => x !== k);
    if (left.length) planKeys.set(planId, left);
    else planKeys.delete(planId);
  };
}

/** True: ⌘↵ must not run the plan: no "Run plan" button of it is in view, uncovered and still
 *  (the press brings it into view), or one settles. No button on screen: it never runs. */
function planGate(planId: string): boolean {
  const ks = planKeys.get(planId) ?? [];
  const act = keyActOf(ks.map((k) => k.act()));
  if (act === 'reveal') for (const k of ks) k.reveal();
  return act !== 'answer';
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

/** What a question card does with the keys: the card knows what you picked and typed, and
 *  what a digit or the send key does to it now (`act`: what they act on is in view, still). */
export type QuestionKeys = { options: () => number; pick: (n: number) => void; send: () => void; act: (key: CardKey) => KeyAct };
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

/** The composer's text field. */
const composerField = (el: HTMLElement): boolean => el.tagName === 'TEXTAREA' && !!el.closest?.('[aria-label="Composer"]');

export function installKeys(): () => void {
  let gPending = 0;
  /** The Esc that may begin a pair (`escPair`): its thread and when. */
  let escFirst: { thread: string; at: number } | null = null;

  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) {
      // The composer or a field used this Esc (a menu closed, a toast went, a turn was asked to stop): it begins no pair.
      if (e.key === 'Escape') escFirst = null;
      return;
    }
    const mod = e.metaKey || e.ctrlKey;
    const typing = isTyping(e.target);
    const k = e.key.toLowerCase();
    // A dialog that is closing is not an overlay any more: an Esc while it fades goes to what is under it.
    const overlay = ui.paletteOpen || ui.settingsOpen || ui.shortcutsOpen || openOne(document.querySelectorAll('[aria-modal="true"]'));

    // ── the primary action (approve / run plan) ──
    // Other text fields (a deny note…) are not the composer: ⌘↵ there must not approve.
    // A held key acts once: its repeats would answer the next approval, then run the plan.
    if (mod && e.key === 'Enter' && !e.repeat && !overlay && (!typing || !!(e.target as HTMLElement).closest?.('[aria-label="Composer"]'))) {
      // A moment after an answer this key acts on nothing: not on the next card, not on a plan.
      if (rests()) {
        e.preventDefault();
        return;
      }
      const ap = keyTargetApproval();
      if (ap && keyGate(ap, ap.subject.kind === 'question' && !e.altKey && !e.shiftKey ? 'send' : 'other')) {
        e.preventDefault();
        return;
      }
      if (ap?.subject.kind === 'question') {
        const q = questionKeysOf(ap.id);
        if (q && !e.altKey && !e.shiftKey) {
          e.preventDefault();
          // The card notes the answer when one goes: with nothing to send, nothing rests.
          q.send();
        }
        return;
      }
      if (e.altKey) {
        if (ap && ap.choices.includes('allow_session')) {
          e.preventDefault();
          noteAnswer();
          resolveApprovals(keyTargetIds(ap), 'allow_session');
        }
        return;
      }
      if (e.shiftKey) return;
      if (ap && ap.choices.includes('allow_once')) {
        e.preventDefault();
        noteAnswer();
        resolveApprovals(keyTargetIds(ap), 'allow_once');
        return;
      }
      const plan = keyTargetPlan();
      if (plan) {
        e.preventDefault();
        if (planGate(plan)) return;
        noteAnswer();
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
      if (keyGate(ap)) return;
      if (denyKeyAction(ap, e.shiftKey) === 'note') ui.denyNoteFor = ap.id;
      else {
        noteAnswer();
        resolveApprovals(keyTargetIds(ap), 'deny');
      }
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
      // Every Esc ends a pair; only one that did nothing else may begin the next (`escPair`).
      const first = escFirst;
      escFirst = null;
      if (overlay) return; // overlays handle their own Esc
      const now = Date.now();
      if (typing) {
        const field = e.target as HTMLElement;
        if (!e.repeat && composerField(field) && !(field as HTMLTextAreaElement).value.trim()) escFirst = { thread: ui.thread, at: now };
        field.blur();
        return;
      }
      // One step per press: dismiss the toast that waits for you, close, deselect, leave the full
      // office, close the find bar, leave a closed request, and only then stop the running turn,
      // or, a second press in a 1:1 chat with an agent that rests, open "Undo a turn".
      const waiting = toasts.waiting;
      const agent = store.threadAgent(ui.thread);
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
        rewind: mayRewind({
          oneToOne: !isRequestThread(ui.thread),
          agent,
          turnRuns: busy(ui.thread),
          waits: !!agent && store.pendingApprovals.some((a) => a.agent_id === agent.id),
          inTerminal: !!agent && takeovers.has(agent.id),
          composerEmpty: [...document.querySelectorAll<HTMLTextAreaElement>('[aria-label="Composer"] textarea')].every((t) => !t.value.trim()),
        }),
        second: escPair(first, ui.thread, now),
      });
      if (step === 'dismiss-toast') {
        // A stop needs two presses of its own: the one before the toast's does not count.
        toasts.dismiss(waiting!.id);
        ui.escAsk = null;
      } else if (step === 'close-artifact') ui.artifactOpen = null;
      else if (step === 'deselect') ui.selection = null;
      else if (step === 'leave-office') ui.mode = 'blend';
      else if (step === 'close-find') ui.findOpen = false;
      else if (step === 'leave-closed') ui.openRequest(ui.request);
      else if (step === 'stop') {
        // Nothing ran to stop: this press may begin a pair.
        if (!stopThreadByEsc(isRequestThread(ui.thread) || agent ? ui.thread : ui.request)) escFirst = { thread: ui.thread, at: now };
      } else if (step === 'rewind') rewindLayer.open(agent!.id);
      return;
    }

    // ── the options of the question that waits: on a radio or a checkbox too (it is no text field) ──
    if (!overlay && /^[1-9]$/.test(e.key)) {
      const ap = keyTargetApproval();
      const q = ap?.subject.kind === 'question' ? questionKeysOf(ap.id) : null;
      const n = q ? digitPick({ key: e.key, mod, alt: e.altKey, shift: e.shiftKey }, isTextField(e.target as Element | null), q.options()) : null;
      if (ap && q && n !== null) {
        e.preventDefault();
        if (!keyGate(ap, { digit: n })) q.pick(n);
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
      ['Esc', 'Close what is open · press twice: stop the running turn, or undo a turn of an agent that rests'],
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
      ['Esc Esc', 'Stop the running turn, or undo a turn of an agent that rests (empty line)'],
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
