// Mock backend for the skills manager (`pnpm dev` and the site demo): a few installed skills in
// memory, and a catalog whose install/uninstall really move rows between the two tabs.
// Answers the same commands as src-tauri/src/commands.rs; MockTransport delegates to it.
import type { CatalogSkill } from '../gen/CatalogSkill';
import type { Skill } from '../gen/Skill';
import { HOME, REPO } from './sim';

const sk = (name: string, description: string, dir: string, p: Partial<Skill> = {}): Skill => ({
  name,
  description,
  path: `${dir}/${name}`,
  engine: 'claude_code',
  scope: 'user',
  origin: null,
  removable: true,
  ...p,
});

const U = `${HOME}/.claude/skills`;
const installed: Skill[] = [
  sk('release-notes', 'Draft release notes from merged PRs since the last tag, grouped by area.', `${REPO}/.claude/skills`, { scope: 'project' }),
  sk('acme-design-tokens', 'Use the acme/web design tokens instead of raw hex values and pixel sizes.', `${REPO}/.claude/skills`, { scope: 'project' }),
  sk('pdf', 'Read, fill, merge and split PDF files, and extract their text and tables.', U),
  sk('systematic-debugging', 'Find the root cause with a four-phase process before proposing a fix.', U),
  sk('review-hog-authoring', 'Author review perspectives for the PostHog review pipeline.', `${HOME}/.claude/plugins/cache/official/posthog/1.1.67/skills`, { removable: false, origin: 'plugin posthog' }),
  sk('imagegen', 'Generate and edit images from a prompt.', `${HOME}/.codex/skills/.system`, { engine: 'codex', removable: false, origin: 'Codex system set' }),
  sk('deploy-checklist', 'Walk the team’s pre-deploy checklist and report what is missing.', `${HOME}/.codex/skills`, { engine: 'codex' }),
  { ...sk('codly-team', 'How to work as one agent in a Codly team: board, search, memory and reports.', 'codly:bundled'), scope: 'bundled', removable: false, origin: 'built into Codly' },
];

const A = 'https://github.com/anthropics/skills.git';
const S = 'https://github.com/obra/superpowers.git';
const cat = (id: string, description: string, category: string, author: string, repo: string): CatalogSkill => ({
  id,
  description,
  category,
  author,
  repo,
  subdir: `skills/${id}`,
  installed: false,
});
const catalog: CatalogSkill[] = [
  cat('pdf', 'Read, fill, merge and split PDF files, and extract their text and tables.', 'Documents', 'Anthropic', A),
  cat('docx', 'Create and edit Word documents with tracked changes and comments.', 'Documents', 'Anthropic', A),
  cat('pptx', 'Create and edit PowerPoint decks from a brief or a template.', 'Documents', 'Anthropic', A),
  cat('xlsx', 'Build and analyse spreadsheets with formulas, charts and clean formatting.', 'Documents', 'Anthropic', A),
  cat('skill-creator', 'A guide to writing a new skill: structure, frontmatter and bundled resources.', 'Authoring', 'Anthropic', A),
  cat('mcp-builder', 'Build high-quality MCP servers that let an LLM use an external service.', 'Authoring', 'Anthropic', A),
  cat('webapp-testing', 'Test a local web app with Playwright: screenshots, console logs, UI checks.', 'Testing', 'Anthropic', A),
  cat('frontend-design', 'Distinctive, production-grade frontend design that avoids generic AI looks.', 'Design', 'Anthropic', A),
  cat('theme-factory', 'Style slides, docs and pages with one of ten ready-made themes.', 'Design', 'Anthropic', A),
  cat('brainstorming', 'Turn a rough idea into a design through questions, before any code is written.', 'Workflow', 'Jesse Vincent', S),
  cat('writing-plans', 'Write a step-by-step implementation plan from a spec.', 'Workflow', 'Jesse Vincent', S),
  cat('test-driven-development', 'Red, green, refactor: write the failing test before the code.', 'Testing', 'Jesse Vincent', S),
  cat('systematic-debugging', 'Find the root cause with a four-phase process before proposing a fix.', 'Debugging', 'Jesse Vincent', S),
  cat('using-git-worktrees', 'Create an isolated git worktree for a feature before touching code.', 'Git', 'Jesse Vincent', S),
];

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** undefined: not a skills command. */
export async function skillsMock(cmd: string, a: Record<string, unknown>): Promise<unknown> {
  switch (cmd) {
    case 'list_skills': {
      const project = a.project as string | null;
      return installed.filter((s) => s.scope !== 'project' || (project && s.path.startsWith(project)));
    }
    case 'skill_catalog':
      return catalog.map((c) => ({ ...c, installed: installed.some((s) => s.path === `${U}/${c.id}`) }));
    case 'install_skill': {
      const c = catalog.find((x) => x.id === a.id);
      if (!c) throw new Error(`${String(a.id)} is not in the catalog`);
      if (installed.some((s) => s.path === `${U}/${c.id}`)) throw new Error(`${U}/${c.id} already exists; remove it first`);
      await wait(1100); // a clone takes a moment
      const s = sk(c.id, c.description, U);
      installed.splice(2, 0, s);
      return s;
    }
    case 'uninstall_skill': {
      const i = installed.findIndex((s) => s.path === a.path);
      if (i < 0 || !installed[i].removable) throw new Error(`refusing to remove ${String(a.path)}`);
      installed.splice(i, 1);
      return null;
    }
    case 'open_skill_folder':
      return null;
    default:
      return undefined;
  }
}
