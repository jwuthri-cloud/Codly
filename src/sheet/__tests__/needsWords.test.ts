// "Show it" brings this agent's card into view, but the keys answer the oldest waiting request on
// screen (keys.ts `keyTargetApproval`): the sheet says so when that is another agent's request.
import { describe, expect, it } from 'vitest';
import { keysAnswerOther, keysAnswerOtherLine } from '../needsWords';

describe('whose request the keys answer, in the sheet’s “Needs you”', () => {
  const shown = [{ id: 'ap-2' }, { id: 'ap-3' }];

  it('names the other agent when the keys answer a request the sheet does not show', () => {
    expect(keysAnswerOther({ id: 'ap-1', agent_id: 'ada' }, shown)).toBe('ada');
    expect(keysAnswerOtherLine('Ada', false)).toBe('The keys answer the oldest request first (Ada’s). Use the buttons here.');
    // The sheet shows only where the card is: its buttons are there.
    expect(keysAnswerOtherLine('Ada', true)).toBe('The keys answer the oldest request first (Ada’s). Use the buttons on its card in the conversation.');
  });

  it('says nothing when the keys answer one of the requests shown, or none waits', () => {
    expect(keysAnswerOther({ id: 'ap-3', agent_id: 'eli' }, shown)).toBeNull();
    expect(keysAnswerOther(null, shown)).toBeNull();
  });
});
