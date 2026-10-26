// The demo repo map for "acme/web" (~300 files): what `repo_map` returns in the browser demo.
// Deterministic (line counts are a hash of the path), shaped like a real Next.js app, and the
// files the demo story actually touches carry their real line counts. Files a task creates
// appear once that task is committed. Languages use the core's labels (index/lang.rs).
import type { RepoFile } from '../gen/RepoFile';
import { TASK_FILES, type RepoFile as DemoFile } from './repo';

const LANG: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', js: 'javascript', mjs: 'javascript', cjs: 'javascript',
  md: 'markdown', mdx: 'markdown', json: 'json', yml: 'yaml', yaml: 'yaml', css: 'css', sql: 'sql',
  sh: 'shell', html: 'html', toml: 'toml', txt: 'text', csv: 'data',
};

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return (h >>> 0) / 4294967296;
}

function lines(path: string): number {
  const r = hash(path);
  const span = (lo: number, hi: number) => Math.round(lo + (hi - lo) * r * r);
  if (/\/icons\//.test(path)) return span(9, 24);
  if (/i18n\/\w+\.json$/.test(path)) return span(380, 620);
  if (/migration\.sql$/.test(path)) return span(6, 48);
  if (/\.(test|spec)\.tsx?$/.test(path)) return span(28, 240);
  if (/^docs\//.test(path)) return span(40, 320);
  if (/\.css$/.test(path)) return span(24, 260);
  if (/page\.tsx$/.test(path)) return span(24, 190);
  if (/layout\.tsx$|loading\.tsx$|error\.tsx$/.test(path)) return span(12, 60);
  if (/route\.ts$/.test(path)) return span(30, 160);
  if (/^src\/components\//.test(path)) return span(30, 260);
  if (/\.(json|ya?ml|config\.\w+|toml)$/.test(path) || /^\.\w+/.test(path)) return span(8, 70);
  return span(18, 300);
}

const D = (dir: string, names: string[]) => names.map((n) => `${dir}/${n}`);

const ICONS = ['Arrow', 'Bell', 'Calendar', 'Check', 'ChevronDown', 'Close', 'Copy', 'Download', 'Edit', 'External', 'Eye', 'Filter', 'Folder', 'Gear', 'Globe', 'Heart', 'Home', 'Info', 'Link', 'Lock', 'Mail', 'Menu', 'Moon', 'Plus', 'Search', 'Star', 'Sun', 'Trash', 'Upload', 'User'];
const COMPONENTS = ['Avatar', 'Badge', 'Banner', 'Breadcrumbs', 'Button', 'Card', 'Checkbox', 'CodeBlock', 'Combobox', 'CommandMenu', 'DatePicker', 'Dialog', 'Drawer', 'Dropdown', 'EmptyState', 'Footer', 'Form', 'Header', 'Input', 'Kbd', 'Markdown', 'Nav', 'Pagination', 'Popover', 'Progress', 'Radio', 'Select', 'Sidebar', 'Skeleton', 'Spinner', 'Switch', 'Table', 'Tabs', 'Textarea', 'Toast', 'Tooltip'];
const SERVER_AREAS: Record<string, string[]> = {
  settings: ['route.ts', 'schema.ts', 'route.test.ts', 'schema.test.ts'],
  checkout: ['route.ts', 'webhook.ts', 'stripe.ts', 'prices.ts', 'route.test.ts', 'webhook.test.ts'],
  billing: ['route.ts', 'invoices.ts', 'portal.ts', 'plans.ts', 'route.test.ts'],
  projects: ['route.ts', 'schema.ts', 'members.ts', 'permissions.ts', 'archive.ts', 'route.test.ts', 'members.test.ts'],
  users: ['route.ts', 'schema.ts', 'avatar.ts', 'route.test.ts'],
  teams: ['route.ts', 'invites.ts', 'schema.ts', 'roles.ts', 'invites.test.ts'],
  notifications: ['route.ts', 'digest.ts', 'channels.ts', 'digest.test.ts'],
  search: ['route.ts', 'indexer.ts', 'query.ts', 'query.test.ts'],
  webhooks: ['github.ts', 'slack.ts', 'verify.ts', 'verify.test.ts'],
  audit: ['route.ts', 'log.ts', 'export.ts'],
};

const BASE: string[] = [
  '.github/CODEOWNERS', '.github/pull_request_template.md', '.github/workflows/ci.yml', '.github/workflows/e2e.yml', '.github/workflows/release.yml', '.github/dependabot.yml',
  '.env.example', '.eslintignore', '.prettierrc.json', '.nvmrc',
  'README.md', 'CONTRIBUTING.md', 'CHANGELOG.md', 'LICENSE.md', 'package.json', 'next.config.ts', 'tsconfig.json', 'eslint.config.js',
  'vitest.config.ts', 'playwright.config.ts', 'postcss.config.mjs', 'middleware.ts', 'instrumentation.ts',
  'prisma/schema.prisma', 'prisma/seed.ts',
  ...['20260112101500_init', '20260203140000_projects', '20260219091000_teams', '20260305120000_invites', '20260401083000_billing', '20260512170000_notifications', '20260620110000_audit_log', '20260818094500_user_settings', '20260927130000_digest_email']
    .map((m) => `prisma/migrations/${m}/migration.sql`),
  'public/robots.txt', 'public/logo.svg', 'public/illustrations/empty-projects.svg', 'public/illustrations/empty-inbox.svg', 'public/illustrations/welcome.svg',
  ...D('src/app', ['layout.tsx', 'page.tsx', 'error.tsx', 'not-found.tsx', 'loading.tsx', 'global-error.tsx', 'sitemap.ts', 'robots.ts']),
  ...D('src/app/(marketing)', ['layout.tsx', 'pricing/page.tsx', 'pricing/PlanTable.tsx', 'about/page.tsx', 'blog/page.tsx', 'blog/[slug]/page.tsx', 'changelog/page.tsx', 'contact/page.tsx', 'contact/ContactForm.tsx']),
  ...D('src/app/(auth)', ['layout.tsx', 'sign-in/page.tsx', 'sign-in/SignInForm.tsx', 'sign-up/page.tsx', 'sign-up/SignUpForm.tsx', 'reset/page.tsx', 'verify/page.tsx']),
  ...D('src/app/dashboard', ['layout.tsx', 'page.tsx', 'loading.tsx', 'ActivityFeed.tsx', 'StatCard.tsx', 'UsageChart.tsx', 'RecentProjects.tsx', 'Onboarding.tsx', 'QuickActions.tsx']),
  ...D('src/app/settings', ['layout.tsx', 'page.tsx', 'LocaleSelect.tsx', 'DigestSwitch.tsx', 'profile/page.tsx', 'profile/AvatarUpload.tsx', 'billing/page.tsx', 'billing/InvoiceList.tsx', 'team/page.tsx', 'team/InviteForm.tsx', 'team/MemberRow.tsx', 'notifications/page.tsx', 'security/page.tsx', 'security/SessionList.tsx', 'api-keys/page.tsx']),
  ...D('src/app/projects', ['page.tsx', 'loading.tsx', 'ProjectCard.tsx', 'ProjectGrid.tsx', 'new/page.tsx', 'new/NewProjectForm.tsx', '[id]/page.tsx', '[id]/layout.tsx', '[id]/ProjectHeader.tsx', '[id]/settings/page.tsx', '[id]/members/page.tsx', '[id]/activity/page.tsx']),
  ...D('src/app/checkout', ['page.tsx', 'CheckoutForm.tsx', 'success/page.tsx', 'cancelled/page.tsx']),
  ...D('src/app/admin', ['layout.tsx', 'page.tsx', 'users/page.tsx', 'audit/page.tsx', 'flags/page.tsx']),
  ...Object.keys(SERVER_AREAS).filter((a) => a !== 'webhooks').map((a) => `src/app/api/${a}/route.ts`),
  ...COMPONENTS.map((c) => `src/components/${c}.tsx`),
  ...['Button', 'Dialog', 'Select', 'Switch', 'Table', 'Tabs', 'Toast', 'Combobox'].map((c) => `src/components/${c}.test.tsx`),
  ...ICONS.map((i) => `src/components/icons/${i}.tsx`),
  'src/components/icons/index.ts',
  ...D('src/hooks', ['useMediaQuery.ts', 'useDebounce.ts', 'useLocalStorage.ts', 'useHotkeys.ts', 'useInterval.ts', 'useOnClickOutside.ts', 'useCopy.ts', 'useUser.ts', 'useProjects.ts', 'useInfiniteList.ts', 'useDebounce.test.ts', 'useHotkeys.test.ts']),
  ...D('src/lib', ['cn.ts', 'format.ts', 'dates.ts', 'money.ts', 'slug.ts', 'env.ts', 'analytics.ts', 'flags.ts', 'fetcher.ts', 'errors.ts', 'validators.ts', 'constants.ts', 'pagination.ts', 'retry.ts', 'format.test.ts', 'dates.test.ts', 'money.test.ts', 'slug.test.ts', 'retry.test.ts']),
  ...D('src/server', ['db.ts', 'auth.ts', 'session.ts', 'email.ts', 'rate-limit.ts', 'logger.ts', 'queue.ts', 'storage.ts', 'auth.test.ts', 'rate-limit.test.ts']),
  ...Object.entries(SERVER_AREAS).flatMap(([a, fs]) => fs.map((f) => `src/server/${a}/${f}`)),
  ...D('src/server/notifications/templates', ['Digest.tsx', 'Invite.tsx', 'Mention.tsx', 'Receipt.tsx', 'ResetPassword.tsx', 'Welcome.tsx']),
  ...D('src/styles', ['tokens.css', 'reset.css', 'typography.css', 'utilities.css', 'prose.css', 'animations.css']),
  ...D('src/i18n', ['index.ts', 'en.json', 'fr.json', 'de.json', 'es.json', 'ja.json', 'pt-BR.json']),
  ...D('src/types', ['global.d.ts', 'api.ts', 'models.ts', 'env.d.ts']),
  ...D('tests/e2e', ['checkout.spec.ts', 'auth.spec.ts', 'projects.spec.ts', 'settings.spec.ts', 'dashboard.spec.ts', 'team.spec.ts', 'billing.spec.ts', 'fixtures.ts', 'global-setup.ts']),
  ...D('tests/unit', ['setup.ts', 'msw.ts', 'factories.ts', 'render.tsx']),
  ...D('docs', ['architecture.md', 'deployment.md', 'testing.md', 'security.md', 'adr/0001-app-router.md', 'adr/0002-prisma.md', 'adr/0003-zod-everywhere.md', 'adr/0004-feature-flags.md', 'runbooks/incident.md', 'runbooks/stripe-webhooks.md', 'runbooks/rotate-secrets.md']),
  ...D('scripts', ['seed.ts', 'check-env.ts', 'migrate.sh', 'release.sh', 'generate-icons.ts']),
];

/** Real line counts for the files the story reads or writes. */
function count(text: string | null): number {
  return text ? text.split('\n').length - (text.endsWith('\n') ? 1 : 0) : 0;
}

function demoFiles(): Map<string, DemoFile> {
  const m = new Map<string, DemoFile>();
  for (const list of Object.values(TASK_FILES)) for (const f of list) m.set(f.path, f);
  return m;
}

/**
 * The repo as it stands. `committed` = task keys whose work is committed: their new files
 * exist and their edited files have their new length.
 */
export function repoMap(committed: Set<string>): RepoFile[] {
  const demo = demoFiles();
  const after = new Map<string, DemoFile>();
  for (const [key, list] of Object.entries(TASK_FILES)) if (committed.has(key)) for (const f of list) after.set(f.path, f);
  const paths = new Set(BASE);
  for (const p of after.keys()) paths.add(p);
  const out: RepoFile[] = [];
  for (const path of [...paths].sort()) {
    const d = demo.get(path);
    const done = after.get(path);
    let n: number;
    if (done) n = count(done.after);
    else if (d && d.before != null) n = count(d.before);
    else if (d && d.before == null) continue; // created by a task that has not landed yet
    else n = lines(path);
    if (done && done.after == null) continue; // deleted
    const ext = path.includes('.') ? path.slice(path.lastIndexOf('.') + 1).toLowerCase() : '';
    out.push({ path, lines: n, lang: LANG[ext] ?? null });
  }
  return out;
}
