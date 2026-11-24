// three.js (and with it the character builder) must not be in the startup graph: the office is
// loaded lazily (shell/OfficeLayer.svelte) and so is the portrait drawing (office/portraits). A
// static import anywhere on the way from main.ts undoes both without any error, so this test
// walks the static imports of every .ts and .svelte source and fails with the offending chain.
// It reads imports however they are written: over several lines, with comments in the list.
import { describe, expect, it } from 'vitest';

const sources = import.meta.glob<string>(['/src/**/*.ts', '/src/**/*.svelte', '!/src/**/__tests__/**'], { query: '?raw', import: 'default', eager: true });

/**
 * `import x from '…'`, `import '…'`, `export … from '…'`; not `import type`, not `import('…')`.
 * What it imports is names, braces, commas and comments, over as many lines as it takes.
 */
const STATIC = /^\s*(?:import|export)\s+(?!type\b)(?:(?:[\w$*{},\s]|\/\/[^\n]*|\/\*[\s\S]*?\*\/)*?\bfrom\s+)?['"]([^'"]+)['"]/gm;

function resolve(from: string, spec: string): string | null {
  if (spec === 'three' || spec.startsWith('three/')) return 'three';
  if (!spec.startsWith('.')) return null;
  const parts = from.split('/').slice(0, -1);
  for (const p of spec.split('/')) {
    if (p === '..') parts.pop();
    else if (p !== '.') parts.push(p);
  }
  const base = parts.join('/');
  return [base, `${base}.ts`, `${base}/index.ts`].find((c) => c in sources) ?? null;
}

/** The modules loaded before anything is asked for lazily, each with the module that imports it. */
function startupGraph(entry: string): Map<string, string | null> {
  const from = new Map<string, string | null>([[entry, null]]);
  const todo = [entry];
  while (todo.length) {
    const file = todo.pop()!;
    for (const m of (sources[file] ?? '').matchAll(STATIC)) {
      const dep = resolve(file, m[1]);
      if (!dep || from.has(dep)) continue;
      from.set(dep, file);
      if (dep !== 'three') todo.push(dep);
    }
  }
  return from;
}

/** How `entry` reaches three.js through static imports ("three ← a ← b ← entry"), or null. */
function chainToThree(entry: string): string | null {
  const from = startupGraph(entry);
  if (!from.has('three')) return null;
  const chain: string[] = [];
  for (let at: string | null = 'three'; at; at = from.get(at) ?? null) chain.push(at);
  return chain.join(' ← ');
}

describe('the startup graph', () => {
  it('the pattern sees every way to write a static import, and nothing else', () => {
    const source = [
      "import a from './a';",
      "import './b.css';",
      "export { c } from './c';",
      "export * from './d';",
      'import {',
      '  e, // the e\'s; see f',
      '  /* g */ type G,',
      "} from './e';",
      "import x,\n  { y as z } from './f';",
      "import type { T } from './t';",
      "const lazy = import('./lazy');",
      "export const s = './not-an-import';",
    ].join('\n');
    expect([...source.matchAll(STATIC)].map((m) => m[1])).toEqual(['./a', './b.css', './c', './d', './e', './f']);
  });

  it('the walk works: it sees the app, follows .svelte and .svelte.ts imports, and finds three.js in the office', () => {
    const graph = startupGraph('/src/main.ts');
    expect(graph.has('/src/App.svelte')).toBe(true);
    expect(graph.has('/src/components/Portrait.svelte')).toBe(true);
    expect(graph.has('/src/office/portraits.svelte.ts')).toBe(true);
    expect(graph.size).toBeGreaterThan(100);
    expect(chainToThree('/src/office/OfficeView.svelte')).toContain('three');
    expect(chainToThree('/src/office/portrait.ts')).toContain('three');
  });

  it('has no three.js: the office and the portraits load it lazily', () => {
    expect(chainToThree('/src/main.ts')).toBeNull();
  });
});
