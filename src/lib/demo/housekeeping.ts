// The browser-demo stand-in for the core's housekeeping commands (board archive + default MCP
// servers). The archive is seeded with a few older cards so the "Archived" filter has something
// to search; the sweep applies the same day rules as `housekeeping/archive.rs` to the story's
// real tasks (none is old enough in a short session, so nothing else archives); and the
// connection switches are kept in memory (keys are never stored: the demo only remembers that
// one was "added").
import type { ArchivedItem } from '../gen/ArchivedItem';
import type { ArchivePolicy } from '../gen/ArchivePolicy';
import type { BoardFlag } from '../gen/BoardFlag';
import type { Connection } from '../gen/Connection';
import type { ConnectionSecret } from '../gen/ConnectionSecret';
import type { SweepReport } from '../gen/SweepReport';
import type { Task } from '../gen/Task';

const DAY = 86_400_000;

export const DEFAULT_POLICY: ArchivePolicy = { done_days: 2, stale_flag_days: 14, stale_archive_days: 21, question_flag_days: 7 };

const reached = (days: number, threshold: number) => threshold > 0 && days >= threshold;

/** Mirror of `archive::judge` for tasks (the Rust one is the tested original). */
export function judgeTask(t: Pick<Task, 'status' | 'updated_at'>, now: number, p: ArchivePolicy): 'keep' | 'flag' | 'archive_done' | 'archive_stale' {
  const days = Math.floor(Math.max(0, now - t.updated_at) / DAY);
  if (['running', 'verifying', 'reviewing', 'merging'].includes(t.status)) return 'keep';
  if (t.status === 'done' || t.status === 'cancelled') return reached(days, p.done_days) ? 'archive_done' : 'keep';
  if (reached(days, p.stale_archive_days)) return 'archive_stale';
  return reached(days, p.stale_flag_days) ? 'flag' : 'keep';
}

const CATALOG: Omit<Connection, 'enabled' | 'active' | 'secrets'>[] = [
  { id: 'sequential-thinking', label: 'Sequential thinking', blurb: 'A scratchpad for step-by-step reasoning.', tier: 'safe_readonly', command: 'npx -y @modelcontextprotocol/server-sequential-thinking' },
  { id: 'time', label: 'Time', blurb: 'Current time and time-zone conversion.', tier: 'safe_readonly', command: 'uvx mcp-server-time' },
  { id: 'fetch', label: 'Fetch', blurb: 'Read a web page as text.', tier: 'safe_readonly', command: 'uvx mcp-server-fetch' },
  { id: 'context7', label: 'Context7', blurb: 'Up-to-date library documentation.', tier: 'safe_readonly', command: 'npx -y @upstash/context7-mcp' },
  { id: 'filesystem', label: 'Filesystem', blurb: 'Reads files, scoped to the agent’s own working folder.', tier: 'safe_readonly', command: 'npx -y @modelcontextprotocol/server-filesystem {cwd}' },
  { id: 'git', label: 'Git', blurb: 'Log, diff and blame, scoped to the agent’s repository.', tier: 'safe_readonly', command: 'uvx mcp-server-git --repository {cwd}' },
  { id: 'github', label: 'GitHub', blurb: 'Issues, pull requests and code search on GitHub.', tier: 'credentialed', command: 'npx -y @modelcontextprotocol/server-github' },
  { id: 'database', label: 'Database', blurb: 'Query a Postgres, MySQL or SQLite database.', tier: 'credentialed', command: 'npx -y @bytebase/dbhub --transport stdio' },
  { id: 'google-workspace', label: 'Email and calendar', blurb: 'Gmail and Google Calendar.', tier: 'credentialed', command: 'uvx workspace-mcp' },
];
const SLOTS: Record<string, { env: string; label: string }[]> = {
  github: [{ env: 'GITHUB_PERSONAL_ACCESS_TOKEN', label: 'Personal access token' }],
  database: [{ env: 'DSN', label: 'Connection string' }],
  'google-workspace': [
    { env: 'GOOGLE_OAUTH_CLIENT_ID', label: 'OAuth client ID' },
    { env: 'GOOGLE_OAUTH_CLIENT_SECRET', label: 'OAuth client secret' },
  ],
};

export class HousekeepingDemo {
  private policy: ArchivePolicy = { ...DEFAULT_POLICY };
  private archive: ArchivedItem[];
  private enabled = new Map<string, boolean>();
  private keys = new Set<string>();

  constructor(now: number) {
    const old = (n: number, id: string, key: string, title: string, text: string, status: string, reason: ArchivedItem['reason']): ArchivedItem => ({
      id,
      kind: 'task',
      key,
      title,
      text,
      status,
      reason,
      plan_id: null,
      touched_at: now - n * DAY,
      archived_at: now - (n - 2) * DAY,
    });
    this.archive = [
      old(5, 'demo-a1', 't2', 'Rename theme tokens to the new palette', 'Replace --bg-1..4 with --surface-1..4 across the app.\nResult: renamed in 41 files, build green.', 'done', 'done'),
      old(9, 'demo-a2', 't1', 'Add a rate limiter to the sign-in route', 'Token bucket per IP, 10 attempts a minute.\nResult: merged with tests for the burst case.', 'done', 'done'),
      old(26, 'demo-a3', 't4', 'Investigate flaky checkout test', 'Fails about one run in ten on CI.\nNote: nobody picked this up.', 'pending', 'stale'),
    ];
  }

  handle(cmd: string, a: Record<string, unknown>, tasks: Iterable<Task>, now: number): unknown {
    switch (cmd) {
      case 'board_policy':
        if (a.policy) {
          const p = a.policy as ArchivePolicy;
          this.policy = { ...p, stale_archive_days: p.stale_archive_days !== 0 && p.stale_flag_days > p.stale_archive_days ? p.stale_flag_days : p.stale_archive_days };
        }
        return this.policy;
      case 'board_sweep':
        return this.sweep(tasks, now);
      case 'board_archive': {
        const words = String(a.query ?? '').toLowerCase().split(/\s+/).filter(Boolean);
        return this.archive.filter((i) => words.every((w) => `${i.key} ${i.title} ${i.text}`.toLowerCase().includes(w))).sort((x, y) => y.archived_at - x.archived_at);
      }
      case 'list_connections':
        return this.connections();
      case 'update_connection': {
        const id = String(a.id);
        const s = a.secret as ConnectionSecret | null;
        if (s) {
          if (s.value?.trim()) this.keys.add(`${id}:${s.env}`);
          else this.keys.delete(`${id}:${s.env}`);
        }
        if (typeof a.enabled === 'boolean') this.enabled.set(id, a.enabled);
        return this.connections();
      }
    }
    throw new Error(`Unknown command ${cmd}`);
  }

  private sweep(tasks: Iterable<Task>, now: number): SweepReport {
    const flagged: BoardFlag[] = [];
    let fresh = 0;
    for (const t of tasks) {
      const v = judgeTask(t, now, this.policy);
      if (v === 'flag') flagged.push({ id: t.id, kind: 'task', reason: 'stale', days: Math.floor((now - t.updated_at) / DAY), title: t.title });
      if (v.startsWith('archive') && !this.archive.some((i) => i.id === t.id)) {
        const text = [t.goal, ...t.acceptance.map((x) => `- ${x}`), t.summary ? `Result: ${t.summary}` : ''].filter(Boolean).join('\n');
        this.archive.push({ id: t.id, kind: 'task', key: t.key, title: t.title, text, status: t.status, reason: v === 'archive_done' ? 'done' : 'stale', plan_id: t.plan_id, touched_at: t.updated_at, archived_at: now });
        fresh++;
      }
    }
    // Demo seed: a question that has waited a week, so the flag banner has something real-looking to show.
    flagged.push({ id: 'demo-question', kind: 'question', reason: 'question_waiting', days: 8, title: 'Should the export include archived rows?' });
    return { flagged, archived_ids: this.archive.filter((i) => i.kind === 'task').map((i) => i.id), newly_archived: fresh };
  }

  private connections(): Connection[] {
    return CATALOG.map((c) => {
      const enabled = this.enabled.get(c.id) ?? c.tier === 'safe_readonly';
      const secrets = (SLOTS[c.id] ?? []).map((s) => ({ ...s, set: this.keys.has(`${c.id}:${s.env}`) }));
      return { ...c, enabled, secrets, active: enabled && secrets.every((s) => s.set) };
    });
  }
}
