// The browser demo's id for its repo, acme/web: one for each browser profile, kept in the
// browser's storage, as the core keeps an office's id. The UI keeps things per office under it
// (the request you were last on, the office's look in `codly.sims.<id>`), so after a reload
// the demo opens where you left it, and the storage holds one office's keys, not one per load.
import { uid } from './sim';

const KEY = 'codly.demo.workspace';
/** The key the office keeps its look under, before its id (shell/OfficeLayer.svelte). */
const LOOK = 'codly.sims.';
/** What a demo id looks like (`uid('ws-')`). */
const DEMO_ID = /^ws-[0-9a-f-]+$/;

export function demoWorkspaceId(): string {
  let id: string | null = null;
  try {
    id = localStorage.getItem(KEY);
    if (!id || !DEMO_ID.test(id)) {
      id = uid('ws-');
      localStorage.setItem(KEY, id);
    }
    // The look of the demo office under an id of an earlier load (one key was added per load).
    for (let i = localStorage.length - 1; i >= 0; i--) {
      const key = localStorage.key(i);
      if (key?.startsWith(`${LOOK}ws-`) && key !== `${LOOK}${id}`) localStorage.removeItem(key);
    }
    return id;
  } catch {
    // No storage here (a private window that refuses it, a test): an id for this page only.
    return id ?? uid('ws-');
  }
}
