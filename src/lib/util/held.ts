// A task that was in work when Codly last closed waits for your "Resume work": the core keeps it
// ready and says why in its reason (orchestrator/resume.rs `HELD`). Pure.
import type { Task } from '../gen/Task';

/** The core's reason on a held task, word for word (orchestrator/resume.rs `HELD`). */
export const HELD = "paused: Codly restarted. Nothing starts until you press 'Resume work' at the top of the window";

/** It waits for your "Resume work": nothing of it runs. */
export const isHeld = (t: Pick<Task, 'status' | 'status_reason'>): boolean => t.status === 'ready' && t.status_reason === HELD;

/** What a held task says in a row. */
export const HELD_LINE = 'On hold since Codly restarted · waits for “Resume work”';
