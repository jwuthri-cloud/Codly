// Settings → Safety → Silent agents: the limits of the core's watchdog
// (orchestrator/watchdog.rs, `Settings.watchdog`), as the choices of three selects. All in
// seconds; 0 turns a check off.

import type { WatchdogSettings } from '../gen/WatchdogSettings';

type Limit = { key: keyof WatchdogSettings; title: string; body: string; choices: number[] };

/** One row per limit, in the order the watchdog steps in. */
export const WATCHDOG_LIMITS: Limit[] = [
  {
    key: 'quiet_secs',
    title: 'No output in a turn',
    body: 'How long a turn may give no output before Codly marks it as stalled. After twice this time Codly interrupts the turn and asks the agent to continue, once (a Lead is only interrupted: send your message again). If the agent is silent for this long again, Codly stops it: a worker’s task goes back in line (from its third start on it is blocked for you instead), a review ends without a verdict, and a Lead or a 1:1 agent starts again with your next message.',
    choices: [0, 300, 600, 900, 1800, 3600],
  },
  {
    key: 'quiet_tool_secs',
    title: 'No output while a tool runs',
    body: 'The same limit while a command or another tool call is open: a build or a test suite can be silent for a long time.',
    choices: [0, 1800, 3600, 7200, 14400],
  },
  {
    key: 'idle_secs',
    title: 'Idle on a running task',
    body: 'How long a task’s worker may sit between turns while its task is still running (after you interrupted it, for example) before Codly marks the task as idle. Codly does nothing else: what is next is yours to say.',
    choices: [0, 60, 120, 300, 600, 1800],
  },
];

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
