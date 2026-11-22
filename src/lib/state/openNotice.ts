// A notification of your system names something: an approval, a plan, a task, an agent. A click
// on it brings Codly to the front, and the app then says what it named (src-tauri/src/notify.rs,
// the `open-notice` event). This opens it, in its office.
import { isTauriApp } from '../api';
import { openWorkspace } from './actions';
import { blockers } from './blockers.svelte';
import { offices } from './offices.svelte';
import { store } from './store.svelte';
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

/** Open what a notification names: switch to its office first when it is another one. */
export async function openNotice(t: NoticeTarget) {
  if (t.workspace_id && !store.isOpen(t.workspace_id)) {
    await offices.refresh();
    const office = offices.others.find((o) => o.id === t.workspace_id);
    if (!office) return; // an office you closed since: nowhere to take you
    await openWorkspace(office.repo_path);
    if (!store.isOpen(office.id)) return; // it did not open: that toast says why
  }
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
      ui.settingsTab = 'safety';
      ui.settingsOpen = true;
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
