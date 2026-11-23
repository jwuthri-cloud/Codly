// A notification of your system names something: an approval, a plan, a task, an agent. A click
// on it brings Codly to the front, and the app then says what it named (src-tauri/src/notify.rs,
// the `open-notice` event). This opens it.
//
// The app does not see the click itself: it sees Codly come to the front while the notification
// is on screen. You may as well be back for your own reasons. So what is in the office you look
// at opens, and what is in another office only gets a toast whose Review takes you there: a
// guess must not move you out of the office you work in.
import { isTauriApp } from '../api';
import type { Office } from '../gen/Office';
import { openWorkspace } from './actions';
import { blockers } from './blockers.svelte';
import { budget } from './budget.svelte';
import { offices } from './offices.svelte';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';
import { ui } from './ui.svelte';

/** What a notification names (`Target` in src-tauri/src/notify.rs). */
export type NoticeTarget = {
  /** The office it is in (null: not in one office). */
  workspace_id: string | null;
} & (
  | { kind: 'approval'; id: string; agent_id: string }
  | { kind: 'questions' }
  | { kind: 'plan'; id: string }
  | { kind: 'task'; id: string }
  | { kind: 'agent'; id: string }
  | { kind: 'budgets' }
);

/** What the toast for another office says the notification named there, after the office's name. */
const ELSEWHERE: Record<NoticeTarget['kind'], string> = {
  approval: 'an approval waits for you',
  questions: 'a question waits for you',
  plan: 'a plan needs a look',
  task: 'a task stopped',
  agent: 'an agent needs a look',
  budgets: 'a budget is reached',
};

/** A toast on screen already offers the way to that office: this file's, or the one for what
 *  waits for you there (state/notify.ts). Both read "<office>: …" with a Review button. */
const pointedAt = (office: Office) => toasts.list.some((x) => x.action?.label === 'Review' && x.title.startsWith(`${office.name}: `));

/** Open what a notification names. In another office: offer the way there, and stay. */
export async function openNotice(t: NoticeTarget) {
  if (!t.workspace_id || store.isOpen(t.workspace_id)) return show(t);
  await offices.refresh();
  const office = offices.others.find((o) => o.id === t.workspace_id);
  if (!office) return; // an office you closed since: nowhere to take you
  if (pointedAt(office)) return;
  toast({
    kind: 'attention',
    title: `${office.name}: ${ELSEWHERE[t.kind]}`,
    action: {
      label: 'Review',
      run: async () => {
        await openWorkspace(office.repo_path);
        if (!store.isOpen(office.id)) return; // it did not open: that toast says why
        show(t); // after the switch, which ends on the first request
      },
    },
  });
}

/** Show it, in the office that is open. */
function show(t: NoticeTarget) {
  switch (t.kind) {
    case 'approval': {
      const approval = store.approvals.find((a) => a.id === t.id);
      ui.openThread(approval ? store.approvalThread(approval) : store.threadOf(t.agent_id));
      ui.revealApproval = t.id;
      break;
    }
    case 'questions':
      blockers.panel = true;
      break;
    case 'plan':
      ui.openRequest(store.planMap.get(t.id)?.thread || 'lead');
      break;
    case 'task':
      ui.selectTask(t.id);
      break;
    case 'agent':
      ui.selectAgent(t.id);
      break;
    case 'budgets':
      budget.openSettings();
      break;
  }
}

/** Listen for the app's `open-notice` event (the desktop app only: a browser shows no native notification). */
export function installNoticeOpener(): () => void {
  if (!isTauriApp()) return () => {};
  let off: (() => void) | null = null;
  let stopped = false;
  void import('@tauri-apps/api/event')
    .then(({ listen }) => listen<NoticeTarget>('open-notice', (e) => void openNotice(e.payload)))
    .then((unlisten) => {
      if (stopped) unlisten();
      else off = unlisten;
    })
    .catch((e) => console.error('[codly] a click on a notification will not open what it names', e));
  return () => {
    stopped = true;
    off?.();
  };
}
