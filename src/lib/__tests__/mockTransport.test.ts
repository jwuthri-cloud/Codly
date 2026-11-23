// The browser demo's transport (mock.ts) from the outside, as the UI uses it: a URL parameter
// of docs/DEMO.md starts it in its state, and the commands of that screen are answered.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LotSlot } from '../gen/LotSlot';
import type { MergeState } from '../gen/MergeState';
import type { Office } from '../gen/Office';
import type { Plan } from '../gen/Plan';
import type { Preview } from '../gen/Preview';
import type { RequiredChecks } from '../gen/RequiredChecks';
import type { Snapshot } from '../gen/Snapshot';
import { MockTransport } from '../mock';
import { intakeUi } from '../state/intake.svelte';

/** The demo, as the page with this query string starts it. */
function open(search: string): MockTransport {
  vi.stubGlobal('location', { search });
  return new MockTransport();
}

/** One command. The mock answers after a short wait, as a backend does. */
async function call<T>(t: MockTransport, cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const answer = t.call<T>(cmd, args);
  answer.catch(() => {}); // a refusal is read by the test, after the wait
  await vi.advanceTimersByTimeAsync(200);
  return answer;
}

async function snapshot(t: MockTransport): Promise<Snapshot> {
  const s = t.subscribe(() => {});
  await vi.advanceTimersByTimeAsync(50);
  return s;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] });
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
  // The story draws one picture on a canvas; a test has none.
  vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
});
afterEach(() => {
  intakeUi.dialog = null;
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('the demo transport', () => {
  it('without a parameter waits for you to open the repo, with nothing turned on', async () => {
    const t = open('');
    expect((await snapshot(t)).workspace).toBeNull();
    expect(intakeUi.dialog).toBeNull();
    expect((await call<RequiredChecks>(t, 'required_checks', { set: null })).on).toBe(false);
  });

  it('`?intake=pr&checks=on`: the dialog is open on the demo repo, your checks are on, and no scripted request starts', async () => {
    const t = open('?intake=pr&checks=on');
    expect(intakeUi.dialog).toBe('pull_request');
    await vi.advanceTimersByTimeAsync(10_000);
    const s = await snapshot(t);
    expect(s.workspace?.name).toBe('acme/web');
    expect(s.chats).toEqual([]);
    expect((await call<RequiredChecks>(t, 'required_checks', { set: null })).on).toBe(true);
  });

  it('`?merge=conflict&preview=running`: a finished plan is not merged until an agent resolved the conflict, and its preview runs', async () => {
    const t = open('?merge=conflict&preview=running&autoplay=1');
    let plan: Plan | undefined;
    for (let i = 0; i < 300 && !plan; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      plan = (await snapshot(t)).plans.find((p) => p.status === 'done');
    }
    expect(plan, 'a plan finished').toBeDefined();
    const source = { kind: 'plan', id: plan!.id };
    const merge = await call<MergeState>(t, 'merge_state', { source });
    expect(merge.conflicts).toHaveLength(1);
    await expect(call(t, 'merge_plan', { planId: plan!.id, into: null, unchecked: false })).rejects.toThrow(`merging into main conflicts in ${merge.conflicts[0]}`);
    expect((await call<Preview>(t, 'preview', { source, action: null, seen: null })).run).toMatchObject({ command: 'pnpm dev --port $CODLY_PORT_BASE', ended: null });

    let now = await call<MergeState>(t, 'resolve_merge_conflict', { source, into: null });
    expect(now.resolving).toBeTruthy();
    for (let i = 0; i < 120 && now.conflicts.length; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      now = await call<MergeState>(t, 'merge_state', { source });
    }
    expect(now).toMatchObject({ conflicts: [], resolving: null });
    // The plan is done again a moment after its last task.
    await vi.advanceTimersByTimeAsync(5000);
    expect(await call<string>(t, 'merge_plan', { planId: plan!.id, into: null, unchecked: false })).toMatch(/^[0-9a-f]{40}$/);
  });

  it('answers Close office: a quiet office closes and its plot is free, one with agents at work stays', async () => {
    const t = open('?ws=1');
    const docs = (await call<LotSlot[]>(t, 'lots')).find((l) => l.name === 'docs-site')!;
    await expect(call(t, 'close_office', { workspaceId: 'demo-office-billing' })).rejects.toThrow('is in the middle of a turn in billing-api');
    const after = await call<LotSlot[]>(t, 'close_office', { workspaceId: docs.workspace_id });
    expect(after.find((l) => l.id === docs.id)).toMatchObject({ owned: true, workspace_id: null, name: null });
    expect((await call<Office[]>(t, 'offices')).map((o) => o.name)).toEqual(['billing-api']);
  });
});
