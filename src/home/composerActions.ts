// What `/` offers in the composer: Codly's own actions, and the controls of the agent you talk
// to, which a Claude Code or Codex user types as slash commands: compact, context, its way of
// working (ask, plan, edits), undo the last turn, continue in your terminal, its MCP servers.
// That agent is the Lead in a request and the agent of a 1:1 chat. Every name is listed in
// every conversation, so that a known name is never sent to the agent as a message; an action
// that cannot run here or right now says why instead (`off`), and one that only waits for the
// core's answer says that too (`wait`). The list is built here, away from the component, so it
// can be tested. The agent's controls (`agentCommands`, done by `agentDo`) are also the
// palette's group "This agent": one list, read from the same conversation (`slashContext`).
import type { Agent } from '../lib/gen/Agent';
import type { LeadMode } from '../lib/gen/LeadMode';
import type { PermissionMode } from '../lib/gen/PermissionMode';
import type { TakeoverState } from '../lib/gen/TakeoverState';
import { stopThread } from '../lib/state/actions';
import { PERMISSION_MODES, canCompact, compactAgent, compactHint, compactWait, setPermissionMode, showContext } from '../lib/state/control.svelte';
import { busy } from '../lib/state/queue';
import { mcpSummary, ofLiveRun, type McpServer } from '../lib/state/session';
import { store } from '../lib/state/store.svelte';
import { openInTerminal, showTerminal, takeovers } from '../lib/state/takeover.svelte';
import { isRequestThread, ui } from '../lib/state/ui.svelte';
import { agentTurns } from './agentTurns';
import type { SlashCommand } from './composerMenu';

/** The conversation the composer is in, as far as the actions need it. */
export type SlashContext = {
  /** A 1:1 chat with an agent; false: a request. */
  oneToOne: boolean;
  /** The agent of the conversation: the Lead of a request, the agent of a 1:1 chat. Null: there
   *  is none (a request whose Lead has not started yet, an agent that is gone). */
  agent: Agent | null;
  /** A turn runs in this conversation. */
  busy: boolean;
  /** The request's plan has changes to show. */
  changes: boolean;
  /** The modes of a request (`MODES` of LeadControls). */
  modes: { id: LeadMode; label: string; hint: string; icon: string }[];
  /** The MCP servers the agent's live run reports. */
  mcp: McpServer[];
  /** What the core said last about the agent and your terminal; null: it was not asked yet. */
  takeover: TakeoverState | null;
  /** What the core keeps of the agent's turns; null: it was not asked yet, or has not answered.
   *  `off`: why no turn of the agent can be undone (`noTurn`); null: Codly kept one. */
  turns: { off: string | null } | null;
};

/** What each action does: the component's part. */
export type SlashDo = {
  newRequest: () => void;
  fromIssue: () => void;
  fromPullRequest: () => void;
  stop: () => void;
  requestMode: (mode: LeadMode) => void;
  agentMode: (agent: Agent, mode: PermissionMode) => void;
  leadModels: () => void;
  agentModel: (agent: Agent) => void;
  compact: (agent: Agent) => void;
  context: (agent: Agent) => void;
  changes: () => void;
  /** The agent's sheet, where its checkpoints are: a restore asks first there. */
  undo: (agent: Agent) => void;
  terminal: (agent: Agent) => void;
  showTerminal: (agent: Agent) => void;
  /** The agent's sheet, at its MCP servers. */
  mcp: (agent: Agent) => void;
  clear: () => void;
  closed: () => void;
  settings: () => void;
  shortcuts: () => void;
};

/** What the agent's own controls do (`agentCommands`): the same from the composer and the palette. */
export type AgentDo = Pick<SlashDo, 'stop' | 'agentMode' | 'agentModel' | 'compact' | 'context' | 'undo' | 'terminal' | 'showTerminal' | 'mcp'>;

const IN_REQUEST = 'Only in a request';

/**
 * The names that set how a 1:1 Claude agent works with edits (`PERMISSION_MODES`), in its own
 * chat. `ask` and `plan` are also modes of a request: in a request they keep that meaning. The
 * hints are short, for a row of the menu; the agent's sheet says the rest.
 */
const WAYS: { name: string; id: PermissionMode; icon: string; hint: string }[] = [
  { name: 'ask', id: 'ask', icon: 'lock', hint: 'Every edit follows your permissions first' },
  { name: 'plan', id: 'plan', icon: 'plan', hint: 'It reads and plans first; you accept its plan' },
  { name: 'edits', id: 'accept_edits', icon: 'check', hint: 'Edits run without asking you: your permissions do not see them' },
];
const wayLabel = (id: PermissionMode): string => PERMISSION_MODES.find((m) => m.id === id)!.label;

/** The agent has a process that takes requests. */
const live = (a: Agent): boolean => !!a.run_id && a.status !== 'offline';

/** The agent took a turn: its CLI reported tokens. Before that its conversation is empty, and a
 *  compaction would sum up nothing (and still be put on its record). */
const talked = (a: Agent): boolean => a.usage.input_tokens + a.usage.output_tokens > 0;

/** The core's reason, as a row of the menu shows it. */
const capital = (text: string): string => `${text.slice(0, 1).toUpperCase()}${text.slice(1)}`;

/**
 * The controls of the conversation's agent, in the order the `/` menu lists them: stop it, in a
 * 1:1 chat its way of working and its model, then compact, context, undo, your terminal and its
 * MCP servers. With no agent each one is off and says so.
 */
export function agentCommands(c: SlashContext, on: AgentDo): SlashCommand[] {
  const name = c.agent?.name ?? 'the agent';
  const idle = c.agent && !live(c.agent) ? `${c.agent.name} isn’t running` : null;

  type Row = { name: string; label: string; icon: string };
  type State = { off: string; wait?: true } | { hint: string; run: () => void };
  const list: SlashCommand[] = [];
  /** A control of the conversation's agent: with no agent it is off, and says so. */
  const ofAgent = (row: Row, state: (agent: Agent) => State) => {
    list.push({ ...row, ...(c.agent ? state(c.agent) : { off: c.oneToOne ? 'This agent is gone' : 'This request has no Lead yet' }) });
  };

  list.push({ name: 'stop', label: `Stop ${name}`, icon: 'stop', ...(c.busy ? { hint: 'Ends the running turn', run: on.stop } : { off: 'Nothing is running' }) });

  if (c.oneToOne) {
    for (const w of WAYS) {
      ofAgent({ name: w.name, label: wayLabel(w.id), icon: w.icon }, (a) => {
        if (a.provider !== 'claude' || a.role !== 'solo') return { off: 'Only a 1:1 Claude agent has a mode to switch' };
        if (a.task_id) return { off: `${a.name} is on a task: its mode can change when the task is done` };
        return { hint: w.hint, run: () => on.agentMode(a, w.id) };
      });
    }
    const model = { name: 'model', label: 'Model', icon: 'sparkle' };
    const solo = c.agent?.role === 'solo' ? c.agent : null;
    if (solo) list.push({ ...model, hint: `${solo.name}’s model`, run: () => on.agentModel(solo) });
    else list.push({ ...model, off: 'Not for this agent' });
  }

  ofAgent({ name: 'compact', label: `Compact ${name}’s context`, icon: 'layers' }, (a) => {
    const why = canCompact(a) ? (idle ?? compactWait(a) ?? (talked(a) ? null : `Nothing to compact yet: ${a.name} has not taken a turn`)) : 'Only a Codex agent or a 1:1 Claude agent';
    return why ? { off: why } : { hint: compactHint(a.provider), run: () => on.compact(a) };
  });
  ofAgent({ name: 'context', label: `Check ${name}’s context`, icon: 'gauge' }, (a) => {
    const why = a.provider === 'claude' ? idle : 'Only Claude Code can be asked for it';
    return why ? { off: why } : { hint: 'How full its conversation is', run: () => on.context(a) };
  });

  // Only the core knows these two, and it is asked when `/` opens the menu: until it answers,
  // the row waits in its place (`wait`), so the letters of its name pick no other action.
  ofAgent({ name: 'undo', label: `Undo ${name}’s last turn…`, icon: 'arrow-left' }, (a) => {
    if (!c.turns) return { off: 'Reading what Codly kept of its turns…', wait: true };
    if (c.turns.off) return { off: c.turns.off };
    return { hint: 'Its sheet opens: put the files back under Git, at Checkpoints', run: () => on.undo(a) };
  });
  ofAgent({ name: 'terminal', label: 'Continue in your terminal', icon: 'terminal' }, (a) => {
    if (!c.takeover) return { off: 'Asking Codly if it can go…', wait: true };
    if (c.takeover.takeover) return { hint: `${a.name} is in your terminal: show it`, run: () => on.showTerminal(a) };
    if (c.takeover.blocked) return { off: capital(c.takeover.blocked) };
    return { hint: 'Codly stops its own process first, and sends it nothing until you give it back', run: () => on.terminal(a) };
  });
  ofAgent({ name: 'mcp', label: 'MCP servers', icon: 'globe' }, (a) => {
    const why = idle ?? (c.mcp.length ? null : `${a.name}’s run reports no MCP server`);
    return why ? { off: why } : { hint: mcpSummary(c.mcp), run: () => on.mcp(a) };
  });
  return list;
}

/** The actions of `/`, in the order the menu lists them: Codly's and the agent's (`agentCommands`). */
export function slashCommands(c: SlashContext, on: SlashDo): SlashCommand[] {
  const mine = agentCommands(c, on);
  /** The agent's control of that name, in its place in the menu. */
  const control = (...names: string[]) => names.flatMap((n) => mine.filter((r) => r.name === n));

  const list: SlashCommand[] = [
    { name: 'new', label: 'New request', hint: 'Its own Lead, in parallel', icon: 'plus', run: on.newRequest },
    { name: 'issue', label: 'From issue…', hint: 'A GitHub link or number', icon: 'link', run: on.fromIssue },
    { name: 'pr', label: 'From pull request…', hint: 'Its reviews and failing checks', icon: 'merge', run: on.fromPullRequest },
    ...control('stop'),
  ];

  // The modes of a request, and the ways a 1:1 Claude agent works. `ask` and `plan` name both:
  // each name is listed once, with the meaning it has in this conversation.
  if (!c.oneToOne) {
    for (const m of c.modes) list.push({ name: m.id, label: `${m.label} mode`, icon: m.icon, hint: m.hint, run: () => on.requestMode(m.id) });
    for (const w of WAYS) if (!c.modes.some((m) => m.id === w.name)) list.push({ name: w.name, label: wayLabel(w.id), icon: w.icon, off: 'Only in a 1:1 chat' });
    list.push({ name: 'model', label: 'Model', icon: 'sparkle', hint: 'The Lead’s CLI, model and thinking', run: on.leadModels });
  } else {
    list.push(...control(...WAYS.map((w) => w.name)));
    for (const m of c.modes) if (!WAYS.some((w) => w.name === m.id)) list.push({ name: m.id, label: `${m.label} mode`, icon: m.icon, off: IN_REQUEST });
    list.push(...control('model'));
  }

  const diff = { name: 'diff', label: 'View the changes', icon: 'diff' };
  if (c.oneToOne) list.push({ ...diff, off: IN_REQUEST });
  else if (c.changes) list.push({ ...diff, hint: 'What this request’s plan changed', run: on.changes });
  else list.push({ ...diff, off: 'This request has no changes yet' });

  list.push(
    { name: 'clear', label: 'Clear the draft', hint: 'The text and the attached files', icon: 'trash', run: on.clear },
    { name: 'closed', label: 'Closed requests', hint: 'Read them again', icon: 'clock', run: on.closed },
    { name: 'settings', label: 'Settings', hint: '⌘,', icon: 'settings', run: on.settings },
    { name: 'shortcuts', label: 'Keyboard shortcuts', hint: '?', icon: 'keyboard', run: on.shortcuts },
  );

  // The agent's controls come last: a name that was here before them keeps its place, so the
  // letters you are used to still pick it ("/c" and ↵ clears the draft, it does not compact).
  list.push(...control('compact', 'context', 'undo', 'terminal', 'mcp'));
  return list;
}

/** The conversation `thread`, as the actions read it (reactive in a component: it reads the store). */
export function slashContext(thread: string, modes: SlashContext['modes']): SlashContext {
  void store.rev;
  const a = store.threadAgent(thread);
  // The servers of its live run change with its events, which the store counts per agent.
  if (a) void [store.revBy.get(a.id), store.liveRevBy.get(a.id)];
  const oneToOne = !isRequestThread(thread);
  const plan = oneToOne ? null : store.planOf(thread);
  return {
    oneToOne,
    agent: a,
    busy: busy(thread),
    changes: !!plan?.integration_branch,
    modes,
    mcp: a ? ofLiveRun(store.live[a.id]?.mcp ?? [], a.run_id) : [],
    takeover: a ? takeovers.of(a.id) : null,
    turns: a ? agentTurns.of(a.id) : null,
  };
}

/** The agent's sheet, open at one of its sections. */
function openSheet(a: Agent, section: string) {
  ui.sheetFocus = section;
  ui.selectAgent(a.id);
}

/** What the agent's controls do, in the conversation `thread()` (the composer's changes). */
export function agentDo(thread: () => string): AgentDo {
  return {
    stop: () => void stopThread(thread()),
    agentMode: (a, mode) => void setPermissionMode(a.id, mode),
    // A block body: an assignment's value would be the object, not the state's proxy of it.
    agentModel: (a) => {
      ui.modelDialog = { provider: a.provider, agentId: a.id };
    },
    compact: (a) => void compactAgent(a.id, a.name),
    context: (a) => void showContext(a.id),
    // Its checkpoints are in the Git tab of the sheet's last section: the sheet opens there.
    undo: (a) => openSheet(a, 'tabs'),
    terminal: (a) => void openInTerminal(a.id),
    showTerminal: (a) => showTerminal(a.id),
    mcp: (a) => openSheet(a, 'mcp'),
  };
}
