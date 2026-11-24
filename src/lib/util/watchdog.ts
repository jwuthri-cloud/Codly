// Settings → Safety → Silent agents: the limits of the core's watchdog
// (orchestrator/watchdog.rs, `Settings.watchdog`), as the choices of three selects. All in
// seconds; 0 turns a check off.

import type { WatchdogSettings } from '../gen/WatchdogSettings';

/** `more`: the rest of a long text, behind "More about this" under `body`. */
type Limit = { key: keyof WatchdogSettings; title: string; body: string; more?: string; choices: number[] };

/** One row per limit, in the order the watchdog steps in. Each says in two short sentences
 *  when Codly marks, interrupts and stops (watchdog.rs: flagged at the limit, interrupted at
 *  twice the limit, stopped one limit later); what follows a step is in `WATCHDOG_DETAIL`. */
export const WATCHDOG_LIMITS: Limit[] = [
  {
    key: 'quiet_secs',
    title: 'No output in a turn',
    body: 'Codly marks a turn as stalled after this long with no output. At twice this time it interrupts the turn, and when the agent stays silent this long once more, it stops the agent.',
    choices: [0, 300, 600, 900, 1800, 3600],
  },
  {
    key: 'quiet_tool_secs',
    title: 'No output while a tool runs',
    body: 'The limit for the same three steps while a tool call is open that gives no sign of life: a build or a test suite can be silent for a long time.',
    more: 'A Claude agent’s commands, MCP tools and helpers say every 30 s that they still run, so the limit above counts for them.',
    choices: [0, 1800, 3600, 7200, 14400],
  },
  {
    key: 'idle_secs',
    title: 'Idle on a running task',
    body: 'Codly marks a task as idle when its worker sits this long between turns while the task is still running. It does nothing else: what is next is yours to say.',
    choices: [0, 60, 120, 300, 600, 1800],
  },
];

/** What follows an interrupt and a stop, once for all the limits, under the section. */
export const WATCHDOG_DETAIL =
  'After an interrupt Codly asks the agent to continue, once. A Lead is only interrupted: send your message again. When Codly stops an agent, a worker’s task goes back in line (from its third start on it is blocked for you instead), a review ends without a verdict, and a Lead or a 1:1 agent starts again with your next message.';

/** The choices of one select, shortest first after "Off", with the saved value among them: a
 *  settings file can hold any number of seconds, and the select must show the one in use. */
export function limitChoices(choices: number[], saved: number): number[] {
  return [...new Set([...choices, saved])].sort((a, b) => a - b);
}

/** "Off", "45 s", "10 min", "2 h", "1 h 30 min". */
export function limitLabel(secs: number): string {
  if (secs === 0) return 'Off';
  if (secs % 60) return `${secs} s`;
  const hours = Math.floor(secs / 3600);
  const minutes = (secs % 3600) / 60;
  if (!hours) return `${minutes} min`;
  return minutes ? `${hours} h ${minutes} min` : `${hours} h`;
}

/** The watchdog settings with one limit changed. */
export function withLimit(w: WatchdogSettings, key: keyof WatchdogSettings, secs: number): WatchdogSettings {
  return { ...w, [key]: secs };
}
