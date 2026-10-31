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

/** The job titles an agent can have (its badge). Mirrors `ROLE_TITLES` in the core. */
export const ROLE_TITLES = ['Backend', 'Frontend', 'Full-stack', 'Mobile', 'Product manager', 'Designer', 'QA', 'DevOps', 'Data', 'Security', 'Docs', 'Reviewer'];

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
