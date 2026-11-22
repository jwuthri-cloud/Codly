import { describe, expect, it } from 'vitest';
import { codeToCopy, withCopyButtons } from '../codeblocks';

describe('code block copy', () => {
  it('gives every code block its own Copy button and leaves the rest alone', () => {
    const html = '<p>Run <code>ls</code>:</p><pre><code>ls -la\n</code></pre><p>then</p><pre><code>pwd\n</code></pre>';
    const out = withCopyButtons(html);
    expect(out.match(/<button class="copy"/g)).toHaveLength(2);
    expect(out).toContain('<div class="code"><button class="copy" type="button" aria-label="Copy code">Copy</button><pre><code>ls -la\n</code></pre></div>');
    expect(out.startsWith('<p>Run <code>ls</code>:</p>')).toBe(true);
    expect(withCopyButtons('<p>no code</p>')).toBe('<p>no code</p>');
  });

  it('escaped "<pre>" in the text is not a code block', () => {
    const html = '<p>the &lt;pre&gt; tag</p>';
    expect(withCopyButtons(html)).toBe(html);
  });

  it('copies the code without the line break markdown adds at its end', () => {
    expect(codeToCopy('npm test\n')).toBe('npm test');
    expect(codeToCopy('a\n\nb\n')).toBe('a\n\nb');
    expect(codeToCopy('no break')).toBe('no break');
  });
});
