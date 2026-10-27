// The IDE's browser backend (`pnpm dev`): a small in-memory acme/web worktree answering the
// ide_* commands. Files come from the scripted session's repo (demo/repo.ts); the first plan's
// files are "already edited", so the Source tab has changes and real diffs to show.
// Edits made in the overlay persist for the page's lifetime, like a real worktree would.
import type { IdeChange } from '../lib/gen/IdeChange';
import type { IdeEntry } from '../lib/gen/IdeEntry';
import type { IdeFile } from '../lib/gen/IdeFile';
import type { IdeInfo } from '../lib/gen/IdeInfo';
import type { IdeSearch } from '../lib/gen/IdeSearch';
import { REPO, WORKTREES } from '../lib/demo/sim';
import { TASK_FILES, unifiedDiff } from '../lib/demo/repo';

type MockFile = { base: string | null; content: string };

const README = `# acme/web

The marketing site and the customer dashboard.

\`\`\`bash
pnpm install
pnpm dev
\`\`\`
`;

let files: Map<string, MockFile> | null = null;

function seed(): Map<string, MockFile> {
  const m = new Map<string, MockFile>();
  m.set('README.md', { base: README, content: README });
  m.set('.gitignore', { base: 'node_modules\n.next\n', content: 'node_modules\n.next\n' });
  for (const [task, list] of Object.entries(TASK_FILES)) {
    for (const f of list) {
      // Task t1..t3 = the theme plan, edited in this worktree; the rest is committed as-is.
      const edited = ['t1', 't2', 't3'].includes(task);
      const content = (edited ? f.after : (f.before ?? f.after)) ?? '';
      if (!m.has(f.path) || edited) m.set(f.path, { base: edited ? f.before : (f.before ?? f.after), content });
    }
  }
  return m;
}

const fs = () => (files ??= seed());
const isText = (s: string) => !s.includes('\0');

function status(): IdeChange[] {
  const out: IdeChange[] = [];
  for (const [path, f] of fs()) {
    if (f.base === f.content) continue;
    out.push({ path, kind: f.base == null ? 'untracked' : 'modified' });
  }
  return out.sort((a, b) => a.path.localeCompare(b.path));
}

function list(dir: string): IdeEntry[] {
  const prefix = dir ? `${dir.replace(/\/$/, '')}/` : '';
  const seen = new Map<string, IdeEntry>();
  for (const [path, f] of fs()) {
    if (!path.startsWith(prefix)) continue;
    const rest = path.slice(prefix.length);
    const slash = rest.indexOf('/');
    const name = slash < 0 ? rest : rest.slice(0, slash);
    if (slash < 0) seen.set(name, { name, path, is_dir: false, size: f.content.length });
    else if (!seen.has(name)) seen.set(name, { name, path: prefix + name, is_dir: true, size: 0 });
  }
  if (!seen.size && dir) throw new Error(`${dir}: not a folder`);
  return [...seen.values()].sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
}

function search(query: string): IdeSearch {
  const q = query.trim().toLowerCase();
  const out: IdeSearch = { files: [], hits: [], truncated: false };
  if (q.length < 2) return out;
  for (const [path, f] of fs()) {
    if (path.toLowerCase().includes(q) && out.files.length < 50) out.files.push(path);
    if (!isText(f.content)) continue;
    f.content.split('\n').forEach((line, i) => {
      if (out.hits.length < 200 && line.toLowerCase().includes(q)) out.hits.push({ path, line: i + 1, text: line.trim().slice(0, 200) });
    });
  }
  return out;
}

/** Answers `ide_*` commands; undefined for anything else. */
export function mockIde(cmd: string, a: Record<string, unknown>): unknown {
  const path = String(a.path ?? '').replace(/^\.\//, '');
  // The core's `confine` in miniature: no absolute paths, `..` or `.git`.
  if (cmd.startsWith('ide_') && /^\/|(^|\/)\.\.(\/|$)|(^|\/)\.git(\/|$)/.test(path)) throw new Error(`${path}: outside the workspace`);
  switch (cmd) {
    case 'ide_info':
      return {
        agent_id: String(a.agentId),
        agent_name: 'Ada',
        root: `${WORKTREES}/${String(a.agentId).slice(0, 8)}`,
        project: REPO.split('/').pop() ?? 'acme-web',
        branch: 'codly/theme-toggle',
        changed: status().length,
      } satisfies IdeInfo;
    case 'ide_list':
      return list(path);
    case 'ide_read': {
      const f = fs().get(path);
      if (!f) throw new Error(`${path}: cannot read`);
      return { path, content: f.content, size: f.content.length } satisfies IdeFile;
    }
    case 'ide_write': {
      const f = fs().get(path);
      if (!f) throw new Error(`${path}: no such file (use New file to create it)`);
      f.content = String(a.content ?? '');
      return null;
    }
    case 'ide_create':
      if (!path || path.endsWith('/')) throw new Error('give the file a name');
      if (fs().has(path)) throw new Error(`${path} already exists`);
      fs().set(path, { base: null, content: '' });
      return null;
    case 'ide_search':
      return search(String(a.query ?? ''));
    case 'ide_changes':
      return status();
    case 'ide_diff': {
      const f = fs().get(path);
      return f && f.base !== f.content ? unifiedDiff({ path, before: f.base, after: f.content }) : '';
    }
    default:
      return undefined;
  }
}
