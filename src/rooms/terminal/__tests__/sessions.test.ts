// Terminal sessions outlive the Terminal room, so each one must be let go by hand: a shell that
// ended is disposed before its replacement opens, and a shell whose folder is no longer a place
// of the office (another office opened, an agent's worktree is gone) is closed.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { closeSessionsOutside, openSession, sessionKey, sessions, type Session } from '../sessions';

const fake = vi.hoisted(() => {
  class Terminal {
    disposed = 0;
    options: Record<string, unknown> = {};
    textarea = undefined;
    private onInput: (data: string) => void = () => {};
    loadAddon() {}
    open() {}
    write() {}
    onData(cb: (data: string) => void) {
      this.onInput = cb;
    }
    dispose() {
      this.disposed++;
    }
    /** The user types. */
    type(data: string) {
      this.onInput(data);
    }
  }
  let opened = 0;
  const pty = {
    ptyOpen: vi.fn(async (_cwd: string | null) => `pty-${++opened}`),
    ptyAttach: vi.fn(async () => {}),
    ptyWrite: vi.fn(async (_ptyId: string, _data: string) => {}),
    ptyClose: vi.fn(async (_ptyId: string) => {}),
  };
  return { Terminal, pty };
});

vi.mock('@xterm/xterm', () => ({ Terminal: fake.Terminal }));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class {} }));
vi.mock('../../../lib/api', () => ({ api: fake.pty }));
vi.mock('../../../lib/util/color', () => ({ tokenHex: () => '#000000' }));

vi.stubGlobal('document', { createElement: () => ({ style: {} }), documentElement: { dataset: {} } });

const term = (s: Session) => s.term as unknown as InstanceType<typeof fake.Terminal>;
const closed = () => fake.pty.ptyClose.mock.calls.map((c) => c[0]);

beforeEach(() => {
  sessions.clear();
  vi.clearAllMocks();
});

describe('openSession', () => {
  it('gives the same shell back while it runs', async () => {
    const first = await openSession('/repo', 'Repository');
    expect(await openSession('/repo', 'Repository')).toBe(first);
    expect(fake.pty.ptyOpen).toHaveBeenCalledTimes(1);
    expect(term(first).disposed).toBe(0);
  });

  it('disposes a shell that ended before it opens the next one in that folder', async () => {
    const first = await openSession('/repo', 'Repository');
    fake.pty.ptyWrite.mockRejectedValueOnce(new Error('no such terminal'));
    term(first).type('ls\r');
    await vi.waitFor(() => expect(first.exited).toBe(true));

    const second = await openSession('/repo', 'Repository');
    expect(second).not.toBe(first);
    expect(term(first).disposed).toBe(1);
    expect(closed()).toEqual([first.ptyId]);
    expect(sessions.get('/repo')).toBe(second);
    expect(sessions.size).toBe(1);
  });
});

describe('closeSessionsOutside', () => {
  it('closes the shells of folders that are no longer places, and keeps the others', async () => {
    const repo = await openSession('/repo', 'Repository');
    const ada = await openSession('/repo/.codly/ada', 'Ada’s worktree');
    const bo = await openSession('/repo/.codly/bo', 'Bo’s worktree');

    await closeSessionsOutside([sessionKey('/repo'), '/repo/.codly/bo']);
    expect([...sessions.keys()]).toEqual(['/repo', '/repo/.codly/bo']);
    expect(term(ada).disposed).toBe(1);
    expect(closed()).toEqual([ada.ptyId]);
    expect(term(repo).disposed + term(bo).disposed).toBe(0);
  });

  it('closes every shell of the office that was open before', async () => {
    const old = [await openSession('/old', 'Old'), await openSession('/old/.codly/ada', 'Ada’s worktree')];
    await closeSessionsOutside([sessionKey('/new')]);
    expect(sessions.size).toBe(0);
    expect(old.map((s) => term(s).disposed)).toEqual([1, 1]);
    expect(closed().sort()).toEqual(old.map((s) => s.ptyId).sort());
  });
});
