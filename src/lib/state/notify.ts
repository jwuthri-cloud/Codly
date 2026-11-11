// Event → toast pointers. Toasts never carry the decision itself (approvals live inline in the
// conversation and in the agent's sheet); they only point at it, and only when you can't see it.
import type { Approval } from '../gen/Approval';
import type { EventRow } from '../gen/EventRow';
import type { Office } from '../gen/Office';
import { oneLine } from '../util/sanitize';
import { openWorkspace } from './actions';
import { blockers } from './blockers.svelte';
import { offices } from './offices.svelte';
import { store } from './store.svelte';
import { toast, toasts } from './toasts.svelte';
import { ui } from './ui.svelte';

/** Whether you are reading that request's conversation right now. */
const watching = (thread: string) => ui.room === 'home' && ui.mode !== 'office' && ui.mode !== 'map' && ui.thread === thread;

const asks = (s: Approval['subject']) => (s.kind === 'command' ? oneLine(s.command, 80) : s.kind === 'question' ? 'Answer it in the conversation' : `Wants to ${s.kind.replace('_', ' ')}`);

/** Something that waits for you in an office you are not looking at, and how to get to it once there. */
type Need = { line: string; body: string; open: () => void };

export function installNotifications(): () => void {
  const seenPlanStatus = new Map<string, string>();
  const approvalToast = new Map<string, number>();
  const planToast = new Map<string, number>();
  const off = store.onEvent((ev: EventRow) => {
    arrived();
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
          body: asks(s),
          action: { label: 'Review', run: () => { ui.openThread(thread); ui.revealApproval = id; } },
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
        // Only the change to failed: bookkeeping on a task that already failed says nothing new.
        if (k.task.status === 'failed' && store.prevTaskStatus !== 'failed')
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

  // Your other offices keep working. What waits for you there (an approval, a blocking question,
  // a plan to approve) is pointed at too: one toast per office, and Review takes you there. Their
  // finished and failed work stays quiet: the tab's count and the office itself show it.
  const away = new Map<string, { toast: number; needs: Map<string, Need> }>();
  /** Their agents are not in this view: names come from their own `agent_changed` events. */
  const names = new Map<string, string>();
  let recount: ReturnType<typeof setTimeout> | undefined;
  const name = (agentId: string) => names.get(agentId) ?? 'An agent';
  /** The office you are in now: what waited there is in front of you, not behind a toast. */
  function arrived() {
    const w = away.size ? away.get(store.workspace?.id ?? '') : undefined;
    if (!w) return;
    toasts.dismiss(w.toast);
    away.delete(store.workspace?.id ?? '');
  }
  const card = (o: Office, needs: Map<string, Need>) => {
    const last = [...needs.values()][needs.size - 1];
    return {
      title: needs.size > 1 ? `${o.name}: ${needs.size} waiting for you` : `${o.name}: ${last.line}`,
      body: last.body,
      action: {
        label: 'Review',
        run: async () => {
          await openWorkspace(o.repo_path);
          if (!store.isOpen(o.id)) return; // it did not open: that toast says why
          arrived();
          last.open(); // after the switch, which ends on the first request
        },
      },
    };
  };
  const offElsewhere = store.onElsewhere((ev: EventRow) => {
    const k = ev.kind;
    const ws = ev.workspace_id ?? '';
    let key: string;
    let need: Need | null = null;
    switch (k.type) {
      case 'agent_changed':
        names.set(k.agent.id, k.agent.name);
        return;
      case 'approval_requested': {
        const id = k.approval.id;
        key = `a:${id}`;
        need = {
          line: `${name(k.approval.agent_id)} ${k.approval.subject.kind === 'question' ? 'has a question' : 'needs your approval'}`,
          body: asks(k.approval.subject),
          open: () => {
            const ap = store.approvals.find((a) => a.id === id);
            ui.openThread(ap ? store.approvalThread(ap) : 'lead');
            ui.revealApproval = id;
          },
        };
        break;
      }
      case 'approval_resolved':
        key = `a:${k.approval_id}`;
        break;
      case 'blocker_changed':
        key = `b:${k.blocker.id}`;
        if (k.blocker.status === 'open' && k.blocker.severity === 'blocking')
          need = { line: `${name(k.blocker.agent_id)} has a question`, body: k.blocker.title, open: () => (blockers.panel = true) };
        break;
      case 'plan_changed': {
        const thread = k.plan.thread || 'lead';
        key = `p:${k.plan.id}`;
        if (k.plan.status === 'awaiting_approval') need = { line: 'a plan is ready for you', body: k.plan.summary, open: () => ui.openRequest(thread) };
        break;
      }
      default:
        return;
    }
    // The tab's count follows now, not at its next poll.
    clearTimeout(recount);
    recount = setTimeout(() => void offices.refresh(), 400);
    arrived();
    const o = offices.others.find((x) => x.id === ws);
    let w = away.get(ws);
    if (need) {
      if (!o || w?.needs.has(key)) return; // nowhere to take you, or not new (a card or a plan changes more than once)
      if (w) toasts.dismiss(w.toast);
      else away.set(ws, (w = { toast: 0, needs: new Map() }));
      w.needs.set(key, need);
      w.toast = toast({ kind: 'attention', ...card(o, w.needs) });
    } else if (w?.needs.delete(key)) {
      if (!w.needs.size) {
        toasts.dismiss(w.toast);
        away.delete(ws);
      } else if (o) toasts.update(w.toast, card(o, w.needs));
    }
  });

  return () => {
    off();
    offElsewhere();
    clearTimeout(recount);
  };
}
