// Hire manifests as the add-agent wizard handles them: the constants the core enforces
// (crates/codly-core/src/hire.rs), the five briefing templates and the helpers around them.
// The rules themselves live in the core; the UI only mirrors the limits for counters.

import type { HireManifest } from '../gen/HireManifest';
import type { Provider } from '../gen/Provider';

export const HIRE_SCHEMA = 'codly/hire@1';
/** An exported agent: a hire manifest plus its traits, career and memory. */
export const AGENT_SCHEMA = 'codly/agent@1';
export const MAX_BYTES = 64 * 1024;
export const MAX_NAME = 40;
export const MAX_DESCRIPTION = 600;
export const MAX_GOAL = 2000;
/** Avatars the office can draw (an agent's `look_seed`). */
export const AVATARS = ['ember', 'moss', 'tide', 'dusk', 'sage', 'flint', 'pearl', 'copper'] as const;
/** The `provider` of a command Codly has no adapter for. */
export const CUSTOM = 'custom';

export type Engine = Provider | typeof CUSTOM;

export function blankManifest(provider: Provider = 'claude'): HireManifest {
  return {
    schema: HIRE_SCHEMA, name: '', avatar: AVATARS[0], color: 0, title: null, provider, model: null, command: null,
    isolate: true, resume_session: null, description: '', goal: '', traits: [],
  };
}

export interface HireTemplate {
  id: string;
  label: string;
  title: string;
  description: string;
  goal: string;
}

/** Briefing starting points: what the agent is for, and the goal it keeps coming back to. */
export const TEMPLATES: HireTemplate[] = [
  {
    id: 'janitor', label: 'Repo janitor', title: 'DevOps',
    description: 'Keeps the repository tidy: dead code, stale TODOs, unused dependencies, inconsistent formatting. Small, safe changes only.',
    goal: 'Leave the repo a little cleaner after every task. Remove only what you can prove is unused, run the tests after each change, and never touch behaviour.',
  },
  {
    id: 'docs', label: 'Docs writer', title: 'Docs',
    description: 'Writes and updates documentation: README, guides, API references and changelog entries, in the voice the repo already uses.',
    goal: 'Keep the docs true to the code. When asked about a change, update every doc that mentions it; when idle-checking, find places where docs and code disagree.',
  },
  {
    id: 'triager', label: 'Bug triager', title: 'QA',
    description: 'Reproduces reported bugs, finds the likely cause in the code and writes up a short, actionable triage note.',
    goal: 'For every bug: reproduce it first, name the file and cause you suspect, rate severity, and propose the smallest fix. Say plainly when you cannot reproduce.',
  },
  {
    id: 'research', label: 'Research assistant', title: 'Product manager',
    description: 'Reads the codebase and answers how things work, compares options and summarises trade-offs. Read-mostly.',
    goal: 'Answer with evidence: cite files and lines, separate what you verified from what you infer, and finish with a short recommendation.',
  },
  {
    id: 'release', label: 'Release manager', title: 'DevOps',
    description: 'Prepares releases: gathers merged changes, drafts the changelog, checks versions and the release checklist.',
    goal: 'Before any release, list what changed since the last tag, flag breaking changes and missing tests, draft release notes, and stop short of publishing anything.',
  },
];

/**
 * The manifest in a Lead reply: the first fenced block that holds the schema marker, else the first
 * balanced `{…}` that does. The core's review then judges it like an imported file.
 */
export function extractManifest(text: string): string | null {
  for (const m of text.matchAll(/```(?:json)?\s*([\s\S]*?)```/gi)) {
    if (m[1].includes(HIRE_SCHEMA)) return m[1].trim();
  }
  for (let start = text.indexOf('{'); start >= 0; start = text.indexOf('{', start + 1)) {
    let depth = 0;
    let inStr = false;
    for (let i = start; i < text.length; i++) {
      const c = text[i];
      if (inStr) {
        if (c === '\\') i++;
        else if (c === '"') inStr = false;
      } else if (c === '"') inStr = true;
      else if (c === '{') depth++;
      else if (c === '}' && --depth === 0) {
        const s = text.slice(start, i + 1);
        if (s.includes(HIRE_SCHEMA)) return s;
        break;
      }
    }
  }
  return null;
}

/** A manifest as the pretty JSON a person would share (null optionals left out). */
export function manifestJson(m: HireManifest): string {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(m)) if (v !== null && v !== '') out[k] = v;
  return JSON.stringify(out, null, 2) + '\n';
}

/** A file name for a saved hire ("mina.hire.json"). */
export const manifestFileName = (m: Pick<HireManifest, 'name'>): string =>
  `${m.name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'agent'}.hire.json`;
