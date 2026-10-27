// The browser demo's stand-in for crates/codly-core/src/hire.rs: the same rules, enough to make
// the add-agent wizard honest in `pnpm dev` (the real app asks the core). Keep the two in step.

import type { Agent } from '../gen/Agent';
import type { HireManifest } from '../gen/HireManifest';
import type { HireReview } from '../gen/HireReview';
import type { LaunchPreview } from '../gen/LaunchPreview';
import type { Provider } from '../gen/Provider';
import { ROLE_TITLES } from '../util/team';
import type { Sim } from './sim';
import type { Story } from './story';
import { AVATARS, CUSTOM, HIRE_SCHEMA, MAX_BYTES, MAX_DESCRIPTION, MAX_GOAL, MAX_NAME } from '../util/hire';

const PROVIDERS: Provider[] = ['claude', 'codex', 'gemini', 'opencode', 'cursor', 'copilot'];
const FIELDS = ['schema', 'name', 'avatar', 'color', 'title', 'provider', 'model', 'command', 'isolate', 'resume_session', 'description', 'goal'];
const SHELL_META = ';&|<>$`(){}\\!#';
const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max'];

export function shellRisks(cmd: string): string[] {
  const out: string[] = [];
  for (const c of cmd) {
    const n = c === '\n' || c === '\r' ? 'a line break' : SHELL_META.includes(c) ? `\`${c}\`` : null;
    if (n && !out.includes(n)) out.push(n);
  }
  return out;
}

export function splitCommand(cmd: string): string[] | string {
  // eslint-disable-next-line no-control-regex
  if (/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(cmd)) return 'the command contains control characters';
  const out: string[] = [];
  let cur = '';
  let quote = '';
  let started = false;
  for (const c of cmd) {
    if (quote) {
      if (c === quote) quote = '';
      else cur += c;
    } else if (c === "'" || c === '"') {
      quote = c;
      started = true;
    } else if (/\s/.test(c)) {
      if (started) out.push(cur), (cur = ''), (started = false);
    } else (cur += c), (started = true);
  }
  if (quote) return 'the command has an unclosed quote';
  if (started) out.push(cur);
  return out.length ? out : 'the command is empty';
}

const charsetOk = (s: string, extra: string) => !s.startsWith('-') && [...s].every((c) => /[A-Za-z0-9]/.test(c) || extra.includes(c));

export function problems(m: HireManifest): string[] {
  const p: string[] = [];
  if (m.schema !== HIRE_SCHEMA) p.push(`schema must be "${HIRE_SCHEMA}"`);
  const name = m.name.trim();
  if (!name) p.push('the agent needs a name');
  else if ([...name].length > MAX_NAME || /[\x00-\x1f\x7f]/.test(name)) p.push(`the name must be one line of at most ${MAX_NAME} characters`);
  if (m.avatar != null && !(AVATARS as readonly string[]).includes(m.avatar)) p.push(`unknown avatar "${m.avatar}"`);
  if (m.color != null && !(Number.isInteger(m.color) && m.color >= 0 && m.color < 8)) p.push('color must be between 0 and 7');
  if (m.title != null && !ROLE_TITLES.some((t) => t.toLowerCase() === m.title!.trim().toLowerCase())) p.push(`unknown title "${m.title}"`);
  const known = m.provider === CUSTOM || (PROVIDERS as string[]).includes(m.provider);
  if (!known) p.push(`unknown provider "${m.provider}"`);
  if (m.model != null && (m.model.length > 100 || !charsetOk(m.model, '._:/@+-'))) p.push('the model name has characters a model name never has');
  if (m.resume_session != null && (!m.resume_session || m.resume_session.length > 200 || !charsetOk(m.resume_session, '._:-')))
    p.push('the session id may only contain letters, digits and . _ : -');
  if ([...m.description].length > MAX_DESCRIPTION) p.push(`the description is over ${MAX_DESCRIPTION} characters`);
  if ([...m.goal].length > MAX_GOAL) p.push(`the goal is over ${MAX_GOAL} characters`);
  const cmd = m.command?.trim();
  if (known && m.provider === CUSTOM) {
    if (!cmd) p.push('a custom agent needs a launch command');
    else if (cmd.length > 1000) p.push('the command is over 1000 characters');
    else {
      const s = splitCommand(cmd);
      if (typeof s === 'string') p.push(s);
    }
  } else if (known && cmd) p.push('only a custom agent has a launch command; the others use their own adapter');
  return p;
}

export function reviewHire(raw: string): HireReview {
  if (raw.length > MAX_BYTES) return { manifest: null, problems: [`the manifest is ${Math.ceil(raw.length / 1024)} KB; the limit is ${MAX_BYTES / 1024} KB`], shell_risks: [] };
  let v: unknown;
  try {
    v = JSON.parse(raw.replace(/^﻿/, ''));
  } catch (e) {
    return { manifest: null, problems: [`not a valid hire manifest: ${(e as Error).message}`], shell_risks: [] };
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) return { manifest: null, problems: ['not a valid hire manifest: expected a JSON object'], shell_risks: [] };
  const o = v as Record<string, unknown>;
  const unknown = Object.keys(o).filter((k) => !FIELDS.includes(k));
  if (unknown.length) return { manifest: null, problems: [`not a valid hire manifest: unknown field \`${unknown[0]}\``], shell_risks: [] };
  const str = (k: string) => (typeof o[k] === 'string' ? (o[k] as string) : null);
  if (str('schema') == null || str('name') == null || str('provider') == null) return { manifest: null, problems: ['not a valid hire manifest: missing schema, name or provider'], shell_risks: [] };
  const m: HireManifest = {
    schema: str('schema')!, name: str('name')!, avatar: str('avatar'), color: typeof o.color === 'number' ? o.color : null, title: str('title'),
    provider: str('provider')!, model: str('model'), command: str('command'), isolate: o.isolate === false ? false : true,
    resume_session: str('resume_session'), description: str('description') ?? '', goal: str('goal') ?? '',
  };
  const p = problems(m);
  return { manifest: p.length ? null : m, problems: p, shell_risks: m.command ? shellRisks(m.command) : [] };
}

/** The last check before an agent exists. */
export function validateHire(m: HireManifest, confirmShell: boolean): string[] {
  const p = problems(m);
  const risks = m.command ? shellRisks(m.command) : [];
  if (risks.length && !confirmShell) p.push(`the command contains ${risks.join(', ')} (Codly runs it without a shell); confirm to allow it`);
  return p;
}

export function launchPreview(provider: string, model: string | null, command: string | null): LaunchPreview {
  const base = model?.trim() ? model.trim() : null;
  const bare = base && EFFORTS.includes(base.split(':').pop() ?? '') ? base.slice(0, base.lastIndexOf(':')) : base;
  const withModel = (s: string, flag: string) => (bare ? `${s} ${flag} ${bare}` : s);
  const fixed: Record<string, string> = {
    claude: withModel('claude -p --input-format stream-json --output-format stream-json --verbose', '--model'),
    codex: 'codex app-server',
    gemini: withModel('gemini --acp --skip-trust', '-m'),
    opencode: 'opencode acp',
    cursor: 'cursor-agent acp',
    copilot: 'copilot --acp',
  };
  if (provider === CUSTOM) {
    const c = (command ?? '').trim();
    const s = c ? splitCommand(c) : 'type the command to run';
    return { command: c, editable: true, shell_risks: shellRisks(c), problems: typeof s === 'string' ? [s] : [] };
  }
  if (fixed[provider]) return { command: fixed[provider], editable: false, shell_risks: [], problems: [] };
  return { command: '', editable: false, shell_risks: [], problems: [`unknown provider "${provider}"`] };
}

export function draftPrompt(brief: string): string {
  return (
    'Draft a hire manifest for a new 1:1 coding agent. This is a one-off drafting request: do not submit a plan, do not hire or delegate ' +
    'to anyone, do not use tools. Reply with ONLY one JSON object in a ```json block, using exactly these fields and no others:\n' +
    `{"schema": "${HIRE_SCHEMA}", "name": "<short first name>", "avatar": one of ${JSON.stringify(AVATARS)}, "color": 0-7, "title": <optional job title>, ` +
    `"provider": one of ${JSON.stringify(PROVIDERS)}, "model": <optional>, "isolate": true, "description": "<role in 1-3 sentences, at most ${MAX_DESCRIPTION} characters>", ` +
    `"goal": "<the standing goal, at most ${MAX_GOAL} characters>"}\n` +
    'Leave out `command` and `resume_session`. The user reviews the draft before anything starts.\n' +
    `What the agent is for: ${brief.trim().slice(0, 1000)}`
  );
}

/** What the demo's Lead answers to a drafting request: a plausible manifest built from the brief. */
export function demoDraft(brief: string): string {
  const b = brief.toLowerCase();
  const pick = (re: RegExp) => re.test(b);
  const [name, title, avatar, color] = pick(/bug|triage|issue/) ? ['Tri', 'QA', 'moss', 3] : pick(/doc|readme|guide/) ? ['Dot', 'Docs', 'pearl', 2] : pick(/release|changelog|version/) ? ['Rel', 'DevOps', 'flint', 5] : ['Scout', 'Product manager', 'tide', 1];
  const m = {
    schema: HIRE_SCHEMA, name, avatar, color, title, provider: 'claude', isolate: true,
    description: `Works on: ${brief.trim().slice(0, 200)}`,
    goal: `${brief.trim().slice(0, 300)} Keep changes small, explain what you did, and ask before anything risky.`,
  };
  return `Here is a draft. Nothing starts until you review and confirm it.\n\n\`\`\`json\n${JSON.stringify(m, null, 2)}\n\`\`\``;
}

/** A "Generate with AI" request in the demo: answered by the Lead, a moment later, with a draft. */
export const isDraftRequest = (text: string): boolean => text.startsWith('Draft a hire manifest') && text.includes(HIRE_SCHEMA);

export function draftReply(sim: Sim, thread: string, text: string): void {
  sim.chat(thread, 'user', text);
  const brief = text.slice(text.lastIndexOf('What the agent is for:') + 'What the agent is for:'.length);
  void sim.sleep(1400).then(() => {
    const lead = [...sim.agents.values()].find((a) => a.role === 'lead' && (a.thread ?? 'lead') === thread);
    sim.chat(thread, lead?.id ?? 'codly', demoDraft(brief));
  });
}

/** The confirmed hire: the same checks as the core, then a 1:1 agent with the manifest's identity. */
export function hireFromManifest(sim: Sim, story: Story, m: HireManifest, confirmShell: boolean): Agent {
  const p = validateHire(m, confirmShell);
  if (p.length) throw new Error(p.join('; '));
  if (m.provider === CUSTOM) throw new Error('Codly can’t launch custom commands yet: pick one of its agent CLIs');
  const name = m.name.trim();
  if ([...sim.agents.values()].some((a) => !a.archived && a.name.toLowerCase() === name.toLowerCase())) throw new Error(`an agent called ${name} already exists`);
  const a = story.hire(m.provider as Provider, m.model?.trim() || null, m.title ?? null);
  return sim.patchAgent(a.id, {
    name, color: m.color ?? a.color, look_seed: m.avatar ?? a.look_seed, session_id: m.resume_session ?? a.session_id,
    worktree: m.isolate ? a.worktree : (sim.workspace?.repo_path ?? a.worktree),
  });
}
