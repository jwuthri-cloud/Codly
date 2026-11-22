import { afterEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api';
import { demoIntake } from '../demo/intake';
import type { ChatMessage } from '../gen/ChatMessage';
import type { Intake } from '../gen/Intake';
import { intakeSummary, intakeThread, startFromGithub } from '../state/intake.svelte';
import { store } from '../state/store.svelte';
import { toasts } from '../state/toasts.svelte';
import { ui } from '../state/ui.svelte';

const issue: Intake = {
  source: { kind: 'issue', host: 'github.com', owner: 'acme', repo: 'web', number: 12, url: 'https://github.com/acme/web/issues/12', title: 'Dark mode resets on reload' },
  comments: 2,
  review_comments: 0,
  failing_checks: 0,
  notes: [],
};
const pull: Intake = {
  source: { ...issue.source, kind: 'pull_request', number: 7, url: 'https://github.com/acme/web/pull/7', title: 'Keep dark\nmode across reloads' },
  comments: 1,
  review_comments: 2,
  failing_checks: 1,
  notes: ['Codly could not read the checks: timeout.'],
};
const asked = (thread: string): ChatMessage => ({ id: `${thread}-1`, thread, author: 'user', text: 'Fix A', attachment: null, ts: 1 });
const said = () => toasts.list.map((t) => [t.kind, t.title, t.body]);

describe('From issue… / From pull request…', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    for (const t of toasts.list) toasts.dismiss(t.id);
    store.chats = [];
    ui.draftRequests = [];
    ui.openRequest('lead');
  });

  it('goes to the request you are on while nothing was asked in it, else to a new one', () => {
    expect(intakeThread('lead', () => false, () => 'lead:new')).toBe('lead');
    expect(intakeThread('lead', (t) => t === 'lead', () => 'lead:new')).toBe('lead:new');
  });

  it('says what the Lead got, and what was left out', () => {
    expect(intakeSummary(issue)).toEqual({ title: 'Issue #12 is with the Lead', body: '“Dark mode resets on reload”, with 2 comments.' });
    expect(intakeSummary({ ...issue, comments: 0 }).body).toBe('“Dark mode resets on reload”.');
    // The title comes from GitHub: it is shown on one line.
    expect(intakeSummary(pull)).toEqual({
      title: 'Pull request #7 is with the Lead',
      body: '“Keep dark mode across reloads”, with 1 comment, 2 comments on the code, 1 failing check. Codly could not read the checks: timeout.',
    });
  });

  it('sends the link and your note to the core, and opens the request', async () => {
    const core = vi.spyOn(api, 'startFromGithub').mockResolvedValue(issue);
    expect(await startFromGithub('issue', '#12', 'Only the cookie part.')).toBeNull();
    expect(core).toHaveBeenCalledWith('lead', 'issue', '#12', 'Only the cookie part.');
    expect(ui.request).toBe('lead');
    expect(said()).toEqual([['success', 'Issue #12 is with the Lead', '“Dark mode resets on reload”, with 2 comments.']]);
  });

  it('starts a new request when the one you are on already has a message', async () => {
    store.chats = [asked('lead')];
    const core = vi.spyOn(api, 'startFromGithub').mockResolvedValue(pull);
    expect(await startFromGithub('pull_request', '7', '')).toBeNull();
    const thread = core.mock.calls[0][0];
    expect(thread).toMatch(/^lead:.+/);
    expect(ui.request).toBe(thread);
    expect(ui.draftRequests.map((d) => d.thread)).toEqual([thread]);
    // Something was left out: the toast stays longer than a plain success.
    expect(said()[0][0]).toBe('info');
  });

  it('says why it could not be read, and opens nothing', async () => {
    store.chats = [asked('lead')];
    vi.spyOn(api, 'startFromGithub').mockRejectedValue('gh is not signed in to github.com. Run gh auth login in a terminal, then try again.');
    expect(await startFromGithub('issue', '12', '')).toBe('gh is not signed in to github.com. Run gh auth login in a terminal, then try again.');
    expect(ui.request).toBe('lead');
    expect(ui.draftRequests).toEqual([]);
    expect(said()).toEqual([]);
  });
});

describe('the demo reads a made-up issue', () => {
  const sent: [string, string][] = [];
  const say = (thread: string, text: string) => void sent.push([thread, text]);
  afterEach(() => {
    sent.length = 0;
  });

  it('a number is an issue or a pull request of acme/web, as asked', () => {
    const i = demoIntake({ thread: 'lead', kind: 'issue', reference: ' #12 ', note: 'Only the cookie part.' }, say);
    expect(i.source).toMatchObject({ kind: 'issue', owner: 'acme', repo: 'web', number: 12, url: 'https://github.com/acme/web/issues/12' });
    expect(sent).toEqual([['lead', 'Work on issue #12 of acme/web (https://github.com/acme/web/issues/12).\n\nOnly the cookie part.']]);
    const p = demoIntake({ thread: 'lead:x', kind: 'pull_request', reference: '7', note: '' }, say);
    expect(p.source.url).toBe('https://github.com/acme/web/pull/7');
    expect(sent[1]).toEqual(['lead:x', 'Work on pull request #7 of acme/web (https://github.com/acme/web/pull/7): address its review comments and failing checks.']);
  });

  it('a link says by itself what it is', () => {
    const p = demoIntake({ thread: 'lead', kind: 'issue', reference: 'https://github.com/octo/site/pull/31/files', note: '' }, say);
    expect(p.source).toMatchObject({ kind: 'pull_request', owner: 'octo', repo: 'site', number: 31 });
  });

  it('refuses what is not a link or a number, and a thread that is not a request', () => {
    expect(() => demoIntake({ thread: 'lead', kind: 'issue', reference: 'fix the bug', note: '' }, say)).toThrow("'fix the bug' is not a GitHub issue");
    expect(() => demoIntake({ thread: 'lead', kind: 'pull_request', reference: '0', note: '' }, say)).toThrow('is not a GitHub pull request');
    expect(() => demoIntake({ thread: 'agent:a1', kind: 'issue', reference: '12', note: '' }, say)).toThrow('agent:a1 is not a request');
    expect(sent).toEqual([]);
  });
});
