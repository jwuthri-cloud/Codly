// The command boundary, by name. The UI (api.ts), the app (`generate_handler!` in
// src-tauri/src/lib.rs) and the browser demo (mock.ts) each keep their own list of commands. A
// name missing from one of them shows only when someone clicks: "command not found" in the app,
// "unknown command" in the demo. (src-tauri/tests/acl.rs checks that the app also allows each
// command it has.)
import { describe, expect, it } from 'vitest';
import shell from '../../../src-tauri/src/lib.rs?raw';
import ui from '../api.ts?raw';
import skillsDemo from '../demo/skillsMock.ts?raw';
import demo from '../mock.ts?raw';

const names = (source: string, pattern: RegExp) => new Set([...source.matchAll(pattern)].map((m) => m[1]));
const without = (all: Set<string>, ...known: (Set<string> | string[])[]) => [...all].filter((name) => !known.some((k) => (Array.isArray(k) ? k.includes(name) : k.has(name)))).sort();

/** What api.ts sends: the first argument of `call`, `invoke` and `upload`. */
const called = names(ui, /\b(?:call|invoke|upload)\b[^'\n]*?\(\s*'([a-z0-9_]+)'/g);
/** What the app has: the last segment of each path in `generate_handler![…]`. */
const handlers = shell.slice(shell.indexOf('generate_handler!['));
const registered = names(handlers.slice(0, handlers.indexOf(']')), /([a-z0-9_]+)\s*,/g);
/** What the demo answers: each `case` of the mock transport and of the skills mock. */
const answered = names(demo + skillsDemo, /case '([a-z0-9_]+)'/g);
/** Sent by a transport method of its own (`subscribe`, `attachPty`, `upload`), which the mock must have to compile. */
const OWN_METHOD = ['ui_subscribe', 'pty_attach', 'save_attachment'];
/** Known holes in the demo: the button fails there with "unknown command". Take a name out with its fix. */
const DEMO_GAPS = ['close_office'];

describe('every command', () => {
  it('the UI calls is one the app has', () => {
    expect(called.size).toBeGreaterThan(100);
    expect(without(called, registered)).toEqual([]);
  });

  it('the app has is one the UI calls', () => {
    // A command nothing calls is a way into the app that nobody uses: take it out, or add its
    // caller to api.ts. (This also fails when a call in api.ts is written so that the pattern
    // above does not see it.)
    expect(registered.size).toBeGreaterThan(100);
    expect(without(registered, called)).toEqual([]);
  });

  it('the UI calls is answered by the browser demo', () => {
    expect(without(called, answered, OWN_METHOD, DEMO_GAPS)).toEqual([]);
  });
});
