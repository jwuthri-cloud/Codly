// The buttons under a request that waits, per kind of card, and what ⌘⌫ does to it.
import { afterEach, describe, expect, it } from 'vitest';
import { mkAgent } from '../../../office/demo/script';
import type { Approval } from '../../gen/Approval';
import type { ApprovalChoice } from '../../gen/ApprovalChoice';
import type { Provider } from '../../gen/Provider';
import { denyKeyAction } from '../../state/keys';
import { store } from '../../state/store.svelte';
import { acceptsEdits, footerButtons, noteGoes, PLAN_NOTE_HINT } from '../approvalFooter';

/** Label, key and what a press does: the answer it sends, or the field it opens. */
const shown = (plan: boolean, offered: ApprovalChoice[]) => footerButtons(plan, offered).map((b) => [b.label, b.kbd ?? '', b.act === 'choice' ? b.choice : b.act]);

describe('the buttons under a request', () => {
  it('a plan is accepted, changed, sent back with a note or stopped: it has no plain "no"', () => {
    // What Claude Code offers for a plan (2.1.311 recordings): allow, deny, and stop the turn.
    expect(shown(true, ['allow_once', 'deny', 'cancel'])).toEqual([
      ['Accept the plan', '⌘↵', 'allow_once'],
      ['Change the plan…', '', 'edit'],
      ['Send it back with a note…', '⌘⌫', 'note'],
      ['Stop the turn', '', 'cancel'],
    ]);
    // No button sends a bare deny, and "Reject the plan" is gone.
    expect(footerButtons(true, ['allow_once', 'deny', 'cancel']).some((b) => b.choice === 'deny' || /reject/i.test(b.label))).toBe(false);
    // Only what the CLI offered: no stop without "cancel", and nothing to change in a plan that cannot be accepted.
    expect(shown(true, ['allow_once', 'deny']).map((b) => b[0])).toEqual(['Accept the plan', 'Change the plan…', 'Send it back with a note…']);
    expect(shown(true, ['deny', 'cancel']).map((b) => b[0])).toEqual(['Send it back with a note…', 'Stop the turn']);
    expect(PLAN_NOTE_HINT).toBe('Say what to change: without a note the same plan comes back.');
  });

  it('a command shows every choice its CLI offered, in one order, then the note', () => {
    expect(shown(false, ['cancel', 'deny', 'allow_always', 'allow_session', 'allow_once'])).toEqual([
      ['Approve once', '⌘↵', 'allow_once'],
      ['Allow for this session', '⌥⌘↵', 'allow_session'],
      ['Always allow', '', 'allow_always'],
      ['Deny', '⌘⌫', 'deny'],
      ['Stop the turn', '', 'cancel'],
      ['Deny with a note…', '⇧⌘⌫', 'note'],
    ]);
  });

  it('a tool request with only allow and deny has those and the note; without deny there is no note', () => {
    expect(shown(false, ['allow_once', 'deny'])).toEqual([['Approve once', '⌘↵', 'allow_once'], ['Deny', '⌘⌫', 'deny'], ['Deny with a note…', '⇧⌘⌫', 'note']]);
    expect(shown(false, ['allow_once'])).toEqual([['Approve once', '⌘↵', 'allow_once']]);
    expect(shown(false, [])).toEqual([]);
  });
});

describe('⌘⌫ on the request the keyboard acts on', () => {
  afterEach(() => {
    store.agents = [];
  });
  const request = (agent: string, subject: Approval['subject']) =>
    ({ id: 'ap', agent_id: agent, run_id: 'r', task_id: null, call_id: null, subject, reason: null, choices: ['allow_once', 'deny', 'cancel'], risks: [], status: 'pending', decided_by: null, created_at: 0 }) as Approval;
  const plan: Approval['subject'] = { kind: 'tool', name: 'ExitPlanMode', input: { plan: '1. Add the line' } };
  const command: Approval['subject'] = { kind: 'command', command: 'npm install', cwd: null };

  it('opens the note field of a plan card, with or without ⇧', () => {
    store.agents = [mkAgent('eli', 'Eli', 'claude', 'solo', 0, 'waiting_approval'), mkAgent('bo', 'Bo', 'codex', 'solo', 1, 'waiting_approval')];
    expect(denyKeyAction(request('eli', plan), false)).toBe('note');
    expect(denyKeyAction(request('eli', plan), true)).toBe('note');
    // Every other request: ⌘⌫ says no at once, ⇧⌘⌫ opens the note.
    expect(denyKeyAction(request('eli', command), false)).toBe('deny');
    expect(denyKeyAction(request('eli', command), true)).toBe('note');
    // That request is Claude Code's own: from another CLI, or an agent Codly does not know, it is a tool like any other.
    expect(denyKeyAction(request('bo', plan), false)).toBe('deny');
    expect(denyKeyAction(request('gone', plan), false)).toBe('deny');
  });
});

describe('"Approve and accept edits from now on"', () => {
  const edit: Approval['subject'] = { kind: 'file_change', paths: ['src/theme.ts'], diff: null };
  const command: Approval['subject'] = { kind: 'command', command: 'npm install', cwd: null };
  // What Claude Code offers for an edit inside the worktree: its only suggestion is a mode switch, so no "for this session" (claude.rs `grantable`).
  const offered: ApprovalChoice[] = ['allow_once', 'deny', 'cancel'];
  const ada = mkAgent('ada', 'Ada', 'claude', 'solo', 0, 'waiting_approval');

  it('is offered on a file edit of a 1:1 Claude agent that asks for edits', () => {
    expect(acceptsEdits(edit, offered, ada, 'balanced')).toBe(true);
    expect(acceptsEdits(edit, offered, { ...ada, permission_mode: 'ask' }, 'balanced')).toBe(true);
  });

  it('is not offered in another mode, for a worker, a Lead, another CLI, an agent on a task, or another kind of request', () => {
    expect(acceptsEdits(edit, offered, { ...ada, permission_mode: 'accept_edits' }, 'balanced')).toBe(false);
    expect(acceptsEdits(edit, offered, { ...ada, permission_mode: 'plan' }, 'balanced')).toBe(false);
    expect(acceptsEdits(edit, offered, mkAgent('wes', 'Wes', 'claude', 'worker', 0, 'waiting_approval'), 'balanced')).toBe(false);
    expect(acceptsEdits(edit, offered, mkAgent('lea', 'Lea', 'claude', 'lead', 0, 'waiting_approval'), 'balanced')).toBe(false);
    expect(acceptsEdits(edit, offered, mkAgent('bo', 'Bo', 'codex', 'solo', 0, 'waiting_approval'), 'balanced')).toBe(false);
    expect(acceptsEdits(edit, offered, { ...ada, task_id: 't1' }, 'balanced')).toBe(false);
    expect(acceptsEdits(command, offered, ada, 'balanced')).toBe(false);
    expect(acceptsEdits(edit, offered, null, 'balanced')).toBe(false);
    // Only an edit that can be approved once.
    expect(acceptsEdits(edit, ['deny', 'cancel'], ada, 'balanced')).toBe(false);
  });

  it('is not offered under "Ask first" permissions, where every change asks you and the core refuses "Accept edits"', () => {
    expect(acceptsEdits(edit, offered, ada, 'strict')).toBe(false);
    expect(acceptsEdits(edit, offered, ada, 'autonomous')).toBe(true);
    expect(acceptsEdits(edit, offered, ada, 'full')).toBe(true);
  });

  it('comes after "Approve once", with the hint of the mode it switches to', () => {
    const hint = 'Edits run without asking you';
    const b = footerButtons(false, offered, false, { provider: 'claude', name: 'Ada' }, hint);
    expect(b.map((x) => [x.label, x.kbd ?? '', x.act])).toEqual([
      ['Approve once', '⌘↵', 'choice'],
      ['Approve and accept edits from now on', '', 'accept_edits'],
      ['Deny', '⌘⌫', 'choice'],
      ['Stop the turn', '', 'choice'],
      ['Deny with a note…', '⇧⌘⌫', 'note'],
    ]);
    expect(b[1].hint).toBe(hint);
    // No new key, and without the offer the buttons are as before.
    expect(footerButtons(false, offered, false, { provider: 'claude', name: 'Ada' }).map((x) => x.label)).not.toContain('Approve and accept edits from now on');
  });
});

describe('the note of a "no", by CLI', () => {
  it('says where it goes for every CLI but Claude Code', () => {
    const note = (provider: Provider) => footerButtons(false, ['allow_once', 'deny'], false, { provider, name: 'Kit' }).at(-1)!.label;
    expect(note('claude')).toBe('Deny with a note…');
    for (const p of ['codex', 'gemini', 'opencode', 'cursor', 'copilot'] as Provider[]) expect(note(p)).toBe('Deny and tell Kit why…');
    // An agent Codly does not know: as before.
    expect(footerButtons(false, ['allow_once', 'deny']).at(-1)!.label).toBe('Deny with a note…');
  });

  it('the line under the note field', () => {
    expect(noteGoes({ provider: 'codex', name: 'Kit' })).toBe('Codex takes no note with a deny. Codly sends yours to Kit as your next message.');
    expect(noteGoes({ provider: 'gemini', name: 'Gil' })).toBe('Gemini CLI takes no note with a deny. Codly sends yours to Gil as your next message.');
    expect(noteGoes({ provider: 'claude', name: 'Ada' })).toBeNull();
    expect(noteGoes(null)).toBeNull();
  });
});

describe('the buttons of a permissions request', () => {
  it('say how long an allow lasts, with the same keys', () => {
    const b = footerButtons(false, ['allow_once', 'allow_session', 'deny'], true).map((x) => [x.label, x.kbd ?? '']);
    expect(b).toEqual([['Allow for this turn', '⌘↵'], ['Allow for this session', '⌥⌘↵'], ['Deny', '⌘⌫'], ['Deny with a note…', '⇧⌘⌫']]);
    expect(footerButtons(false, ['allow_once'])[0].label).toBe('Approve once');
  });
});
