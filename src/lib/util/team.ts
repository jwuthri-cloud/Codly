// Team members: presets, badge titles and the "model:effort" format shared with the core.

import type { Agent } from '../gen/Agent';
import type { Provider } from '../gen/Provider';
import type { TeamMember } from '../gen/TeamMember';

/** Each CLI's thinking-effort levels (Gemini and OpenCode take none through Codly). */
export const EFFORTS: Partial<Record<Provider, string[]>> = {
  claude: ['low', 'medium', 'high', 'xhigh', 'max'],
  codex: ['low', 'medium', 'high', 'xhigh'],
};
const ALL_EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];

/** "opus:high" → { name: "opus", effort: "high" }; a colon that is not an effort stays in the name. */
export function splitModel(m: string | null): { name: string; effort: string } {
  const s = m ?? '';
  const i = s.lastIndexOf(':');
  return i >= 0 && ALL_EFFORTS.includes(s.slice(i + 1)) ? { name: s.slice(0, i), effort: s.slice(i + 1) } : { name: s, effort: '' };
}

export function joinModel(name: string, effort: string): string | null {
  const n = name.trim();
  return n || effort ? `${n}${effort ? `:${effort}` : ''}` : null;
}

/** Another model, keeping the thinking effort it had ("sonnet:high" + "opus[1m]" → "opus[1m]:high"). */
export const withModel = (current: string | null, model: string): string | null => joinModel(model, splitModel(current).effort);

/** The first `n` models with every word of `q` in their name ("claude son" → anthropic/claude-sonnet-4),
 *  and how many matched in all. OpenRouter lists thousands. */
export function findModels(list: string[], q: string, n: number): { shown: string[]; total: number } {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hits = words.length ? list.filter((m) => words.every((w) => m.toLowerCase().includes(w))) : list;
  return { shown: hits.slice(0, n), total: hits.length };
}

/** The job titles an agent can have (its badge). Mirrors `ROLE_TITLES` in the core. */
export const ROLE_TITLES = ['Backend', 'Frontend', 'Full-stack', 'Mobile', 'Product manager', 'Designer', 'QA', 'DevOps', 'Data', 'Security', 'Docs', 'Reviewer', 'Researcher'];

/** Mirrors `TeamMember::title` in the core: its job title, else the role text before its first ':' / '—' / ' - '. */
export function memberTitle(m: Pick<TeamMember, 'role' | 'kind' | 'title'>): string {
  const t = ROLE_TITLES.find((x) => x.toLowerCase() === m.title.trim().toLowerCase());
  if (t) return t;
  const head = (m.role.split(/[:—\n]/)[0] ?? '').split(' - ')[0].trim();
  return head && head.length <= 24 ? head : m.kind === 'reviewer' ? 'Reviewer' : 'Teammate';
}

export const RESERVED = ['consultant', 'lead', 'any', 'codly'];

/** Why this list cannot be saved, or null. `others` = members it must not clash with. */
export function teamProblem(list: TeamMember[], others: TeamMember[] = []): string | null {
  const seen = new Set(others.map((m) => m.name.trim().toLowerCase()));
  for (const m of list) {
    const n = m.name.trim().toLowerCase();
    if (!n) return 'Every member needs a name.';
    if (RESERVED.includes(n)) return `“${m.name}” means something else to the Lead; pick another name.`;
    if (seen.has(n)) return `Two members are called “${m.name}”.`;
    seen.add(n);
  }
  return null;
}

type Preset = Omit<TeamMember, 'id' | 'enabled' | 'provider'> & { provider: Provider | null };

/** One-click starting points (provider null = the first usable CLI). */
export const PRESETS: Preset[] = [
  { name: 'Ana', provider: 'claude', model: 'sonnet', kind: 'worker', title: 'Backend', role: 'APIs, data and server code. Small, tested changes.' },
  { name: 'Leo', provider: 'claude', model: 'sonnet', kind: 'worker', title: 'Frontend', role: 'UI components, styles and accessibility.' },
  { name: 'Pam', provider: 'claude', model: 'opus', kind: 'worker', title: 'Product manager', role: 'Specs, acceptance criteria and docs for features.' },
  { name: 'Rex', provider: 'codex', model: null, kind: 'reviewer', title: 'Reviewer', role: 'Checks every change against its acceptance criteria.' },
  {
    name: 'Ada',
    provider: 'claude',
    model: 'opus',
    kind: 'worker',
    title: 'Researcher',
    role: 'AI safety research assistant (alignment, interpretability, evals, model organisms, control, scalable oversight). Start every reply with a 1-2 sentence answer in plain words (a voice assistant may read it aloud); details, tables and links follow. Rigor: for any claim or result, name the hypothesis, the baseline, the controls, the sample size and the uncertainty; keep what you read or measured apart from what you infer; say when evidence is weak or missing; challenge weak assumptions, the user’s included; if a cheaper test can answer the question first, say so. Literature: use WebSearch and WebFetch for current work, and the public APIs for papers (arXiv export.arxiv.org/api/query, OpenAlex api.openalex.org/works, Semantic Scholar api.semanticscholar.org/graph/v1). Prefer primary sources. Cite only sources you opened in this session (title, authors, year, venue or arXiv id, link); never invent a citation, a number or a quote. Summarize a paper as: claim, method, key numbers, limits, relevance to the question. Memory: check codly_memory_search before redoing work; save findings, reading notes and decisions with codly_memory_write. Write long outputs (literature reviews, experiment plans, comparisons) as Markdown files in your worktree and give their paths. Limits: no credentials or internal documents in files, commands or web requests; never launch compute jobs or anything that costs money: propose the exact command instead.',
  },
];

/** A preset as a new member on a CLI you can run (its own if usable). */
export function fromPreset(p: Preset, usable: Provider[], taken: string[]): TeamMember | null {
  const provider = p.provider && usable.includes(p.provider) ? p.provider : usable[0];
  if (!provider) return null;
  let name = p.name;
  for (let i = 2; taken.some((t) => t.toLowerCase() === name.toLowerCase()); i++) name = `${p.name} ${i}`;
  // A preset's model belongs to its own CLI; another CLI starts on its default.
  return { id: '', name, provider, model: provider === p.provider ? p.model : null, kind: p.kind, title: p.title, role: p.role, enabled: true };
}

/** What an agent is, for labels: "Lead", "Backend", "Frontend consultant", "Consultant", "Reviewer"; null for a plain worker. */
export function agentTitle(a: Pick<Agent, 'role' | 'title' | 'consultant'>): string | null {
  if (a.role === 'lead') return 'Lead';
  if (a.consultant) return a.title ? `${a.title} consultant` : 'Consultant';
  return a.title || (a.role === 'reviewer' ? 'Reviewer' : null);
}
