import { afterEach, describe, expect, it } from 'vitest';
import { sendState } from '../composerSend';
import { rewindLayer } from '../rewindLayer.svelte';

const typed = { text: 'Fix the header', files: 0, saving: false, unknown: false, inTerminal: null };

describe('may the composer send', () => {
  afterEach(() => rewindLayer.close());

  it('sends text or attached files', () => {
    expect(sendState(typed)).toEqual({ ok: true });
    expect(sendState({ ...typed, text: '', files: 1 })).toEqual({ ok: true });
  });

  it('sends nothing while the agent is in your terminal, and says why', () => {
    expect(sendState({ ...typed, inTerminal: 'Ada' })).toEqual({ ok: false, why: 'Ada is in your terminal: give it back to send' });
    expect(sendState({ ...typed, text: '', inTerminal: 'Ada' })).toEqual({ ok: false, why: 'Ada is in your terminal: give it back to send' });
  });

  it('sends no slash command Codly does not have, and no message while its files are attached', () => {
    expect(sendState({ ...typed, unknown: true })).toEqual({ ok: false, why: 'No such command: change the text to send it' });
    expect(sendState({ ...typed, saving: true })).toEqual({ ok: false, why: 'Wait until the files are attached' });
  });

  it('has nothing to send on an empty line, which needs no word', () => {
    expect(sendState({ ...typed, text: '  ' })).toEqual({ ok: false, why: null });
  });

  it('sends nothing while "Undo a turn" is open over the composer: a ↵ meant for the list starts no turn', () => {
    rewindLayer.open('ada');
    expect(sendState(typed)).toEqual({ ok: false, why: 'Close “Undo a turn” to send' });
    expect(sendState({ ...typed, text: '', files: 1 })).toEqual({ ok: false, why: 'Close “Undo a turn” to send' });
    rewindLayer.close();
    expect(sendState(typed)).toEqual({ ok: true });
  });
});
