// How a turn ended, for a row that only holds what the agent thought in it. Pure: read from the
// event log.
import type { EventRow } from '../lib/gen/EventRow';

/**
 * Whether the turn that holds the event `eventId` (a thought of a work row) ended before the
 * agent said anything: it failed or was interrupted, or the agent's process exited, with no
 * message of the agent in that turn. Then its thoughts were cut, not finished ("Was thinking
 * when the turn ended"). False while the turn runs, for a turn that ended well, and for an event
 * the log does not hold.
 */
export function endedBeforeAnswer(events: readonly EventRow[], eventId: number): boolean {
  const agentId = events.find((e) => e.id === eventId)?.agent_id;
  if (!agentId) return false;
  let holds = false;
  let said = false;
  for (const ev of events) {
    if (ev.agent_id !== agentId) continue;
    const k = ev.kind;
    if (k.type === 'turn_started') {
      if (holds) return false; // the event came between two turns: not a cut turn's thought
      said = false;
    } else if (k.type === 'message') said = true;
    if (ev.id === eventId) holds = true;
    if (!holds) continue;
    if (k.type === 'turn_completed') return (k.status === 'failed' || k.status === 'interrupted') && !said;
    if (k.type === 'exited') return !said;
  }
  return false;
}
