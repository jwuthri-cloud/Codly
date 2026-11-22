// Demo backend for workspace trust: the demo repo has Claude Code project settings (a hook, two
// allow rules) and a project MCP server. It starts trusted, so the scripted demo is not
// interrupted; `?trust=ask` starts it undecided, to see the question. The choice is for the
// files shown, by their fingerprint, as in the core.

import type { TrustChoice } from '../gen/TrustChoice';
import type { TrustFile } from '../gen/TrustFile';
import type { TrustState } from '../gen/TrustState';
import type { WorkspaceTrust } from '../gen/WorkspaceTrust';

const FILES: TrustFile[] = [
  {
    path: '.claude/settings.json',
    cli: 'claude',
    what: ['runs commands by itself (hooks): pnpm exec prettier --write src', 'allows without asking: Bash(pnpm test:*), Bash(pnpm lint)'],
  },
  { path: '.mcp.json', cli: 'claude', what: ['starts MCP servers: docs (npx -y @acme/docs-mcp)'] },
];

/** Stands in for the core's SHA-256: one value per set of files. */
function fingerprint(files: TrustFile[]): string {
  const text = JSON.stringify(files);
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (Math.imul(h, 31) + text.charCodeAt(i)) | 0;
  return `demo-${(h >>> 0).toString(16)}`;
}

export class WorkspaceTrustDemo {
  /** The repo's own CLI configuration. A pull or a merged task can change it. */
  files = FILES;
  /** The fingerprints you trusted. */
  private trusted: string[] = [];
  /** The fingerprint you opened the repo without. */
  private without: string | null = null;

  /** `undecided`: start as a repo you have never been asked about. */
  constructor(undecided = false) {
    if (!undecided) this.trusted = [fingerprint(this.files)];
  }

  /** `workspace_trust`: the files and your choice; `set` trusts the files shown or opens the repo without them. */
  trust(set: TrustChoice | null): WorkspaceTrust {
    const now = fingerprint(this.files);
    if (set?.trust === null) {
      this.trusted = [];
      this.without = now;
    } else if (set) {
      if (set.trust !== now) throw new Error('the repo’s CLI configuration changed since you read it. Read it again, then choose');
      this.trusted = [...this.trusted.filter((t) => t !== now), now];
      this.without = null;
    }
    const state: TrustState = this.trusted.includes(now) ? 'trusted' : this.without === now ? 'without' : 'ask';
    const changed = state === 'ask' && (this.trusted.length > 0 || this.without !== null);
    return { state, changed, files: this.files, instructions: ['CLAUDE.md'], fingerprint: now, problem: null };
  }
}
