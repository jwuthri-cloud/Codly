import { describe, expect, it } from 'vitest';
import { addCopyButtons, codeToCopy } from '../codeblocks';

// The tests run without a browser: this is the small part of the DOM that addCopyButtons uses.
class El {
  parent: El | null = null;
  kids: El[] = [];
  className = '';
  type = '';
  textContent = '';
  attrs: Record<string, string> = {};
  ownerDocument = { createElement: (tag: string) => new El(tag) };
  constructor(readonly tag: string, kids: El[] = [], attrs: Record<string, string> = {}) {
    this.append(...kids);
    this.attrs = attrs;
  }
  setAttribute(name: string, value: string) {
    this.attrs[name] = value;
  }
  append(...kids: El[]) {
    for (const k of kids) {
      k.parent = this;
      this.kids.push(k);
    }
  }
  replaceWith(other: El) {
    const p = this.parent!;
    p.kids[p.kids.indexOf(this)] = other;
    other.parent = p;
    this.parent = null;
  }
  querySelectorAll(tag: string): El[] {
    return this.kids.flatMap((k) => [...(k.tag === tag ? [k] : []), ...k.querySelectorAll(tag)]);
  }
  /** The tree as text: `tag.class(children)`. */
  shape(): string {
    return `${this.tag}${this.className ? `.${this.className}` : ''}${this.kids.length ? `(${this.kids.map((k) => k.shape()).join(' ')})` : ''}`;
  }
}
const withButtons = (root: El): El => {
  addCopyButtons(root as unknown as DocumentFragment);
  return root;
};

describe('code block copy', () => {
  it('gives every code block its own Copy button and leaves the rest alone', () => {
    const root = withButtons(new El('root', [new El('p', [new El('code')]), new El('pre', [new El('code')]), new El('p'), new El('ul', [new El('li', [new El('pre', [new El('code')])])])]));
    expect(root.shape()).toBe('root(p(code) div.code(button.copy pre(code)) p ul(li(div.code(button.copy pre(code)))))');
    const button = root.kids[1].kids[0];
    expect([button.type, button.textContent, button.attrs['aria-label']]).toEqual(['button', 'Copy', 'Copy code']);
  });

  it('"<pre" inside an attribute is not a code block: only a pre element gets a button', () => {
    const cell = new El('td', [], { align: '<pre><form action=https://x>' });
    const root = withButtons(new El('root', [new El('table', [new El('tr', [cell])])]));
    expect(root.shape()).toBe('root(table(tr(td)))');
    expect(cell.attrs).toEqual({ align: '<pre><form action=https://x>' });
  });

  it('copies the code without the line break markdown adds at its end', () => {
    expect(codeToCopy('npm test\n')).toBe('npm test');
    expect(codeToCopy('a\n\nb\n')).toBe('a\n\nb');
    expect(codeToCopy('no break')).toBe('no break');
  });
});
