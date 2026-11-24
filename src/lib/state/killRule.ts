// Whether an agent can be killed from Codly. While it is in your terminal ("Open in my
// terminal"), Codly has no process for it: the core stopped its own before the hand-over, and
// the mark stays until you give the agent back, because a terminal of your own can still have
// the session open (orchestrator/takeover.rs). A kill then ends nothing and changes nothing on
// screen, so Kill is off there, says why, and points at "Give it back". Pure, so the sheet,
// `stopAgent` and the tests agree.

/** Why `agent` cannot be killed now (`inTerminal`: its session is in your terminal), else null. */
export function killRefusal(agent: string, inTerminal: boolean): string | null {
  return inTerminal ? `${agent} is in your terminal: Codly runs nothing for it. Give it back to work with it here.` : null;
}
