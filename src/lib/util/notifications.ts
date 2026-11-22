// Settings → Notifications: the switches and what each one says. The app decides which event
// is of which kind and shows the notification (src-tauri/src/notify.rs).
import type { NotificationSettings } from '../gen/NotificationSettings';

export type NotificationSwitch = { key: keyof NotificationSettings; title: string; body: string };

/** When notifications show, whatever their kind. */
export const NOTIFY_WHEN: NotificationSwitch = {
  key: 'only_in_background',
  title: 'Only when Codly is not in front',
  body: 'While you look at Codly, the app itself shows what needs you. Turn this off to get the notifications then too.',
};

/** One switch per kind of notification. */
export const NOTIFY_KINDS: NotificationSwitch[] = [
  { key: 'approval', title: 'Needs your approval', body: 'An agent waits for your OK on a command or a change, or a plan waits for Run plan.' },
  { key: 'question', title: 'Asks you a question', body: 'An agent needs a decision from you to go on.' },
  { key: 'plan_done', title: 'Plan finished', body: 'A plan’s work is done and ready to merge.' },
  { key: 'plan_failed', title: 'Plan failed or stuck', body: 'A plan failed, or cannot go on by itself.' },
  { key: 'task_blocked', title: 'Task blocked', body: 'Codly blocked a task, or it failed. Not a task you stopped yourself.' },
  { key: 'agent_stalled', title: 'Agent stalled or idle', body: 'An agent gives no output or sits idle on its task, or the circuit breaker held it back or stopped it.' },
  { key: 'budget', title: 'Budget reached', body: 'A token or cost budget is used up: no new task starts.' },
  { key: 'merge_ready', title: 'Ready to merge', body: 'A solo agent ended a turn with changes on its branch.' },
];

/** The settings with one switch changed. */
export const withSwitch = (s: NotificationSettings, key: keyof NotificationSettings, on: boolean): NotificationSettings => ({ ...s, [key]: on });
