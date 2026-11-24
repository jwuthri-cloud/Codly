// The browser demo's id for its repo, acme/web: one for each browser profile, kept in the
// browser's storage, as the core keeps an office's id. The UI keeps the request you were last on
// under it (`codly.request.<id>`), so with `?requests=two` (both scripted requests there when the
// page loads) a reload opens on the request you left.
//
// The office's look (`codly.sims.<id>`) is kept under that id too: the demo's agents keep their
// ids from one load to the next (sim.ts `agentId`), so the look is theirs again. The look of any
// other demo id (one an older demo made at each load) goes when the page loads.
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
