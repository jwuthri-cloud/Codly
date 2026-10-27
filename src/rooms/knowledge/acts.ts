// The six message acts: label, edge colour (design tokens, so light and dark both work) and a
// one-line meaning. Same vocabulary as the core's `MsgAct`.
import type { MsgAct } from '../../lib/gen/MsgAct';

export const ACTS: { id: MsgAct; label: string; color: string; tip: string }[] = [
  { id: 'request', label: 'Request', color: 'var(--aurora-m)', tip: 'Asks someone to do something' },
  { id: 'query', label: 'Query', color: 'var(--amber)', tip: 'Asks a question' },
  { id: 'propose', label: 'Propose', color: 'var(--aurora-b)', tip: 'Suggests an approach' },
  { id: 'agree_done', label: 'Agree / done', color: 'var(--ok)', tip: 'Agrees, or reports something finished' },
  { id: 'refuse', label: 'Refuse', color: 'var(--bad)', tip: 'Declines or says it cannot' },
  { id: 'inform', label: 'Inform', color: 'var(--ink-3)', tip: 'Shares a fact, an answer or a note' },
];

export const ACT = Object.fromEntries(ACTS.map((a) => [a.id, a])) as Record<MsgAct, (typeof ACTS)[number]>;
