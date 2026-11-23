// The buttons under a request that waits, per kind of card, and what ⌘⌫ does to it.
import { afterEach, describe, expect, it } from 'vitest';
import { mkAgent } from '../../../office/demo/script';
import type { Approval } from '../../gen/Approval';
import type { ApprovalChoice } from '../../gen/ApprovalChoice';
import { denyKeyAction } from '../../state/keys';
import { store } from '../../state/store.svelte';
import { footerButtons, PLAN_NOTE_HINT } from '../approvalFooter';

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
