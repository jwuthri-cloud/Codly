// The browser demo answers "Said in conversations" as the core does (Store::search_chats).
import { describe, expect, it } from 'vitest';
import type { ChatMessage } from '../gen/ChatMessage';
import { searchDemoChats } from '../demo/chatSearch';

const said = (id: string, thread: string, text: string, ts: number): ChatMessage => ({ id, thread, author: 'user', text, attachment: null, ts });

describe('the demo chat search', () => {
  const chats = [
    said('a', 'lead:a', 'Fix the Login page', 1),
    said('b', 'agent:ada', 'The login form posts twice.', 2),
    said('c', 'closed:lead:b:900', 'LOGIN works again.', 3),
    said('d', 'lead', 'Coverage is at 100% now', 3),
    said('e', 'lead', 'Coverage is at 1000 now', 4),
  ];

  it('finds the text in every kind of thread, newest first, whatever the ASCII case', () => {
    expect(searchDemoChats(chats, 'login').map((m) => m.id)).toEqual(['c', 'b', 'a']);
    expect(searchDemoChats(chats, 'login', 2).map((m) => m.id)).toEqual(['c', 'b']);
  });

  it('matches the text only, literally, and not for a query that short', () => {
    expect(searchDemoChats(chats, '100%').map((m) => m.id)).toEqual(['d']);
    expect(searchDemoChats(chats, 'user')).toEqual([]);
    expect(searchDemoChats(chats, ' l ')).toEqual([]);
  });

  it('two messages of the same millisecond: the one written last comes first', () => {
    const same = [said('x', 'lead', 'same words', 5), said('y', 'lead', 'same words', 5)];
    expect(searchDemoChats(same, 'same').map((m) => m.id)).toEqual(['y', 'x']);
  });
});
