// The demo's "Said in conversations": the search of `Store::search_chats` over the demo's chat
// rows. The text alone is matched, ASCII case aside, `%` and `_` as plain characters; newest
// first; fewer than 2 characters find nothing.
import type { ChatMessage } from '../gen/ChatMessage';

const fold = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

/** `messages` in the order they were written. */
export function searchDemoChats(messages: ChatMessage[], query: string, limit = 40): ChatMessage[] {
  const q = fold(query.trim());
  if ([...q].length < 2) return [];
  return messages
    .map((m, i) => ({ m, i }))
    .filter(({ m }) => fold(m.text).includes(q))
    .sort((a, b) => b.m.ts - a.m.ts || b.i - a.i)
    .slice(0, limit)
    .map(({ m }) => m);
}
