// Event → toast pointers. Toasts never carry the decision itself (approvals live inline in the
// conversation and in the agent's sheet); they only point at it, and only when you can't see it.
import type { EventRow } from '../gen/EventRow';
import { oneLine } from '../util/sanitize';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';
import { ui } from './ui.svelte';

/** Whether you are reading that request's conversation right now. */
const watching = (thread: string) => ui.room === 'home' && ui.mode !== 'office' && ui.mode !== 'map' && ui.thread === thread;

export function installNotifications(): () => void {
  const seenPlanStatus = new Map<string, string>();
  const approvalToast = new Map<string, number>();
  const planToast = new Map<string, number>();
  return store.onEvent((ev: EventRow) => {
    const k = ev.kind;
    switch (k.type) {
      case 'approval_resolved': {
        const id = approvalToast.get(k.approval_id);
        if (id) toasts.dismiss(id);
        approvalToast.delete(k.approval_id);
        break;
      }
      case 'approval_requested': {
        const thread = store.approvalThread(k.approval);
        if (watching(thread) || ui.selectedAgentId === k.approval.agent_id) return;
        const a = store.agentMap.get(k.approval.agent_id);
        const s = k.approval.subject;
        const id = k.approval.id;
        const tid = toast({
          kind: 'attention',
          title: s.kind === 'question' ? `${a?.name ?? 'An agent'} has a question` : `${a?.name ?? 'An agent'} needs your approval`,
          body: s.kind === 'command' ? oneLine(s.command, 80) : s.kind === 'question' ? 'Answer it in the conversation' : `Wants to ${s.kind.replace('_', ' ')}`,
          action: { label: 'Review', run: () => { ui.openRequest(thread); ui.revealApproval = id; } },
        });
        approvalToast.set(id, tid);
        break;
      }
      case 'plan_changed': {
        const prev = seenPlanStatus.get(k.plan.id);
        seenPlanStatus.set(k.plan.id, k.plan.status);
        if (prev === k.plan.status) return;
        const pt = planToast.get(k.plan.id);
        if (pt) toasts.dismiss(pt);
        planToast.delete(k.plan.id);
        const thread = k.plan.thread || 'lead';
        if (k.plan.status === 'awaiting_approval' && !watching(thread))
          planToast.set(k.plan.id, toast({ kind: 'info', title: 'A plan is ready for you', body: k.plan.summary, action: { label: 'Review', run: () => ui.openRequest(thread) } }));
        else if (k.plan.status === 'done' && !watching(thread))
          toast({ kind: 'success', title: 'Plan finished', body: `${k.plan.summary} — ready to merge`, action: { label: 'View changes', run: () => ui.openDiff({ kind: 'plan', id: k.plan.id }) } });
        else if (k.plan.status === 'failed') toast({ kind: 'error', title: 'Plan failed', body: k.plan.summary });
        break;
      }
      case 'task_changed':
        if (k.task.status === 'failed')
          toast({ kind: 'error', title: `${k.task.key} failed`, body: k.task.status_reason ?? k.task.title, action: { label: 'Details', run: () => ui.selectTask(k.task.id) } });
        break;
      case 'error':
        if (k.fatal) {
          const a = ev.agent_id ? store.agentMap.get(ev.agent_id) : null;
          toast({ kind: 'error', title: `${a?.name ?? 'An agent'} stopped`, body: oneLine(k.message, 160) });
        }
        break;
    }
  });
}
