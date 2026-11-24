// The keyboard reaches the conversation in two Tab stops: the window's first stop is "Skip to the
// conversation", before the office, the rail and the top bar, and it puts the focus in the
// composer (from there ⇧⇥ goes into the thread). Read from the sources.
import { describe, expect, it } from 'vitest';

const sources = import.meta.glob<string>(['/src/shell/Shell.svelte', '/src/shell/SkipToConversation.svelte', '/src/home/Composer.svelte'], { query: '?raw', import: 'default', eager: true });
const markup = (file: string) => sources[file].replace(/<script\b[\s\S]*?<\/script>/, '');

describe('the skip link', () => {
  it('is the first thing of the shell, before the office, the rail and the top bar', () => {
    const shell = markup('/src/shell/Shell.svelte');
    const at = shell.indexOf('<SkipToConversation />');
    expect(at).toBeGreaterThan(0);
    for (const later of ['<OfficeLayer', '<Rail', '<TopBar', '<Home']) expect(shell.indexOf(later)).toBeGreaterThan(at);
  });

  it('focuses the composer’s text field, found by the composer’s own name', () => {
    const skip = sources['/src/shell/SkipToConversation.svelte'];
    expect(skip).toContain(`document.querySelector<HTMLTextAreaElement>('[aria-label="Composer"] textarea')?.focus()`);
    expect(markup('/src/home/Composer.svelte')).toMatch(/role="group" aria-label="Composer"/);
  });
});
