// The fake "acme/web" repository behind the demo session: file contents before/after each task
// (diffs are generated from these, so line numbers are real), and a tiny search index.

export type RepoFile = { path: string; before: string | null; after: string | null };

const tokensBefore = `:root {
  --bg: #ffffff;
  --bg-subtle: #f6f6f4;
  --fg: #1b1b18;
  --fg-muted: #6b6b66;
  --border: #e4e4df;
  --accent: #2f6fea;
  --radius: 8px;
}

.dark {
  --bg: #121210;
  --bg-subtle: #1a1a17;
  --fg: #f2f2ee;
  --fg-muted: #9a9a93;
  --border: #2a2a26;
  --accent: #6f9bff;
}

body {
  background: var(--bg);
  color: var(--fg);
}
`;

const tokensAfter = `:root,
[data-theme='light'] {
  color-scheme: light;
  --bg: #ffffff;
  --bg-subtle: #f6f6f4;
  --fg: #1b1b18;
  --fg-muted: #6b6b66;
  --border: #e4e4df;
  --accent: #2f6fea;
  --radius: 8px;
}

[data-theme='dark'] {
  color-scheme: dark;
  --bg: #121210;
  --bg-subtle: #1a1a17;
  --fg: #f2f2ee;
  --fg-muted: #9a9a93;
  --border: #2a2a26;
  --accent: #6f9bff;
}

body {
  background: var(--bg);
  color: var(--fg);
  transition: background-color 120ms ease, color 120ms ease;
}
`;

const layoutBefore = `import type { Metadata } from 'next';
import { Inter } from 'next/font/google';
import '@/styles/tokens.css';
import { Nav } from '@/components/Nav';

const inter = Inter({ subsets: ['latin'] });

export const metadata: Metadata = {
  title: 'Acme',
  description: 'Acme web app',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className={inter.className}>
        <Nav />
        <main>{children}</main>
      </body>
    </html>
  );
}
`;

const layoutAfter = `import type { Metadata } from 'next';
import { Inter } from 'next/font/google';
import '@/styles/tokens.css';
import { Nav } from '@/components/Nav';
import { ThemeProvider } from '@/theme/ThemeProvider';

const inter = Inter({ subsets: ['latin'] });

export const metadata: Metadata = {
  title: 'Acme',
  description: 'Acme web app',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className={inter.className}>
        <ThemeProvider>
          <Nav />
          <main>{children}</main>
        </ThemeProvider>
      </body>
    </html>
  );
}
`;

const themeProvider = `'use client';

import { createContext, useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { useMediaQuery } from '@/hooks/useMediaQuery';

export type Theme = 'system' | 'light' | 'dark';
export type ResolvedTheme = Exclude<Theme, 'system'>;

type ThemeContextValue = {
  theme: Theme;
  resolved: ResolvedTheme;
  setTheme: (next: Theme) => void;
};

export const ThemeContext = createContext<ThemeContextValue | null>(null);

const STORAGE_KEY = 'acme.theme';

function readStored(): Theme {
  if (typeof window === 'undefined') return 'system';
  const v = window.localStorage.getItem(STORAGE_KEY);
  return v === 'light' || v === 'dark' ? v : 'system';
}

export function ThemeProvider({ initial, children }: { initial?: Theme; children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(() => initial ?? readStored());
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)');
  const resolved: ResolvedTheme = theme === 'system' ? (prefersDark ? 'dark' : 'light') : theme;

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    if (next === 'system') window.localStorage.removeItem(STORAGE_KEY);
    else window.localStorage.setItem(STORAGE_KEY, next);
  }, []);

  const value = useMemo(() => ({ theme, resolved, setTheme }), [theme, resolved, setTheme]);
  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}
`;

const useTheme = `import { useContext } from 'react';
import { ThemeContext } from './ThemeProvider';

/** Current preference, the resolved light/dark value, and a setter. */
export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error('useTheme() must be used inside <ThemeProvider>');
  return ctx;
}
`;

const themeTest = `import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from './ThemeProvider';
import { useTheme } from './useTheme';

function Probe() {
  const { theme, resolved, setTheme } = useTheme();
  return <button onClick={() => setTheme('dark')}>{theme}:{resolved}</button>;
}

const mockMedia = (dark: boolean) =>
  vi.stubGlobal('matchMedia', (q: string) => ({
    matches: dark && q.includes('dark'),
    addEventListener() {},
    removeEventListener() {},
  }));

describe('ThemeProvider', () => {
  beforeEach(() => localStorage.clear());

  it('follows the system by default', () => {
    mockMedia(true);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    expect(screen.getByRole('button').textContent).toBe('system:dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('persists an explicit choice', () => {
    mockMedia(false);
    render(<ThemeProvider><Probe /></ThemeProvider>);
    act(() => screen.getByRole('button').click());
    expect(localStorage.getItem('acme.theme')).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });
});
`;

const prismaBefore = `generator client {
  provider = "prisma-client-js"
}

datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

model User {
  id        String        @id @default(cuid())
  email     String        @unique
  name      String?
  settings  UserSettings?
  createdAt DateTime      @default(now())
}

model UserSettings {
  id          String   @id @default(cuid())
  userId      String   @unique
  user        User     @relation(fields: [userId], references: [id])
  locale      String   @default("en")
  emailDigest Boolean  @default(true)
  updatedAt   DateTime @updatedAt
}
`;

const prismaAfter = `generator client {
  provider = "prisma-client-js"
}

datasource db {
  provider = "postgresql"
  url      = env("DATABASE_URL")
}

model User {
  id        String        @id @default(cuid())
  email     String        @unique
  name      String?
  settings  UserSettings?
  createdAt DateTime      @default(now())
}

model UserSettings {
  id          String   @id @default(cuid())
  userId      String   @unique
  user        User     @relation(fields: [userId], references: [id])
  locale      String   @default("en")
  emailDigest Boolean  @default(true)
  theme       Theme    @default(system)
  updatedAt   DateTime @updatedAt
}

enum Theme {
  system
  light
  dark
}
`;

const migration = `-- CreateEnum
CREATE TYPE "Theme" AS ENUM ('system', 'light', 'dark');

-- AlterTable
ALTER TABLE "UserSettings" ADD COLUMN "theme" "Theme" NOT NULL DEFAULT 'system';
`;

const schemaBefore = `import { z } from 'zod';

export const settingsPatch = z
  .object({
    locale: z.string().min(2).max(10),
    emailDigest: z.boolean(),
  })
  .partial()
  .strict();

export type SettingsPatch = z.infer<typeof settingsPatch>;
`;

const schemaAfter = `import { z } from 'zod';

export const themeValues = ['system', 'light', 'dark'] as const;

export const settingsPatch = z
  .object({
    locale: z.string().min(2).max(10),
    emailDigest: z.boolean(),
    theme: z.enum(themeValues),
  })
  .partial()
  .strict();

export type SettingsPatch = z.infer<typeof settingsPatch>;
`;

const routeBefore = `import { NextResponse } from 'next/server';
import { db } from '@/server/db';
import { requireUser } from '@/server/auth';
import { settingsPatch } from './schema';

export async function GET() {
  const user = await requireUser();
  const settings = await db.userSettings.findUnique({ where: { userId: user.id } });
  return NextResponse.json({
    locale: settings?.locale ?? 'en',
    emailDigest: settings?.emailDigest ?? true,
  });
}

export async function PATCH(req: Request) {
  const user = await requireUser();
  const parsed = settingsPatch.safeParse(await req.json());
  if (!parsed.success) {
    return NextResponse.json({ error: parsed.error.flatten() }, { status: 400 });
  }
  const settings = await db.userSettings.upsert({
    where: { userId: user.id },
    update: parsed.data,
    create: { userId: user.id, ...parsed.data },
  });
  return NextResponse.json({ locale: settings.locale, emailDigest: settings.emailDigest });
}
`;

const routeAfter = `import { NextResponse } from 'next/server';
import type { UserSettings } from '@prisma/client';
import { db } from '@/server/db';
import { requireUser } from '@/server/auth';
import { settingsPatch } from './schema';

const toJson = (s: UserSettings | null) => ({
  locale: s?.locale ?? 'en',
  emailDigest: s?.emailDigest ?? true,
  theme: s?.theme ?? 'system',
});

export async function GET() {
  const user = await requireUser();
  const settings = await db.userSettings.findUnique({ where: { userId: user.id } });
  return NextResponse.json(toJson(settings));
}

export async function PATCH(req: Request) {
  const user = await requireUser();
  const parsed = settingsPatch.safeParse(await req.json());
  if (!parsed.success) {
    return NextResponse.json({ error: parsed.error.flatten() }, { status: 400 });
  }
  const settings = await db.userSettings.upsert({
    where: { userId: user.id },
    update: parsed.data,
    create: { userId: user.id, ...parsed.data },
  });
  return NextResponse.json(toJson(settings));
}
`;

const routeTestBefore = `import { describe, expect, it } from 'vitest';
import { GET, PATCH } from './route';
import { asUser, jsonRequest } from '@/test/helpers';

describe('/api/settings', () => {
  it('returns defaults for a new user', async () => {
    const res = await asUser('u1', () => GET());
    expect(await res.json()).toEqual({ locale: 'en', emailDigest: true });
  });

  it('rejects unknown fields', async () => {
    const res = await asUser('u1', () => PATCH(jsonRequest({ nope: 1 })));
    expect(res.status).toBe(400);
  });
});
`;

const routeTestAfter = `import { describe, expect, it } from 'vitest';
import { GET, PATCH } from './route';
import { asUser, jsonRequest } from '@/test/helpers';

describe('/api/settings', () => {
  it('returns defaults for a new user', async () => {
    const res = await asUser('u1', () => GET());
    expect(await res.json()).toEqual({ locale: 'en', emailDigest: true, theme: 'system' });
  });

  it('rejects unknown fields', async () => {
    const res = await asUser('u1', () => PATCH(jsonRequest({ nope: 1 })));
    expect(res.status).toBe(400);
  });

  it('saves a theme', async () => {
    const res = await asUser('u1', () => PATCH(jsonRequest({ theme: 'dark' })));
    expect((await res.json()).theme).toBe('dark');
  });

  it('rejects an unknown theme', async () => {
    const res = await asUser('u1', () => PATCH(jsonRequest({ theme: 'sepia' })));
    expect(res.status).toBe(400);
  });
});
`;

const toggle = `'use client';

import { useState } from 'react';
import { useTheme } from '@/theme/useTheme';
import type { Theme } from '@/theme/ThemeProvider';

const OPTIONS: { value: Theme; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
];

export function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const [saving, setSaving] = useState(false);

  async function choose(next: Theme) {
    if (next === theme || saving) return;
    setSaving(true);
    setTheme(next);
    try {
      // Await the write so a reload right after the click keeps the choice.
      const res = await fetch('/api/settings', {
        method: 'PATCH',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ theme: next }),
      });
      if (!res.ok) throw new Error('PATCH /api/settings failed: ' + res.status);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div role="radiogroup" aria-label="Theme" className="segmented">
      {OPTIONS.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={theme === o.value}
          disabled={saving}
          onClick={() => choose(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
`;

const pageBefore = `import { requireUser } from '@/server/auth';
import { LocaleSelect } from './LocaleSelect';
import { DigestSwitch } from './DigestSwitch';

export default async function SettingsPage() {
  await requireUser();
  return (
    <section className="settings">
      <h1>Settings</h1>
      <div className="row">
        <label>Language</label>
        <LocaleSelect />
      </div>
      <div className="row">
        <label>Weekly email digest</label>
        <DigestSwitch />
      </div>
    </section>
  );
}
`;

const pageAfter = `import { requireUser } from '@/server/auth';
import { LocaleSelect } from './LocaleSelect';
import { DigestSwitch } from './DigestSwitch';
import { ThemeToggle } from './ThemeToggle';

export default async function SettingsPage() {
  await requireUser();
  return (
    <section className="settings">
      <h1>Settings</h1>
      <div className="row">
        <label>Appearance</label>
        <ThemeToggle />
      </div>
      <div className="row">
        <label>Language</label>
        <LocaleSelect />
      </div>
      <div className="row">
        <label>Weekly email digest</label>
        <DigestSwitch />
      </div>
    </section>
  );
}
`;

const e2e = `import { expect, test } from '@playwright/test';

test.describe('settings · theme', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/settings');
  });

  test('defaults to the system theme', async ({ page }) => {
    await expect(page.getByRole('radio', { name: 'System' })).toHaveAttribute('aria-checked', 'true');
  });

  test('switching applies immediately', async ({ page }) => {
    await page.getByRole('radio', { name: 'Dark' }).click();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  });

  test('dark mode persists after reload', async ({ page }) => {
    await page.getByRole('radio', { name: 'Dark' }).click();
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await expect(page.getByRole('radio', { name: 'Dark' })).toHaveAttribute('aria-checked', 'true');
  });
});
`;

const pkgBefore = `{
  "name": "acme-web",
  "version": "2.4.0",
  "private": true,
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "test": "vitest run",
    "lint": "eslint ."
  },
  "devDependencies": {
    "eslint": "^9.38.0",
    "eslint-config-next": "16.1.2",
    "vitest": "^4.1.0"
  }
}
`;

const pkgAfter = `{
  "name": "acme-web",
  "version": "2.4.0",
  "private": true,
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "test": "vitest run",
    "lint": "eslint . --max-warnings 0"
  },
  "devDependencies": {
    "eslint": "^10.2.0",
    "eslint-config-next": "16.1.2",
    "vitest": "^4.1.0"
  }
}
`;

const nextBefore = `{
  "name": "acme-web",
  "version": "2.4.0",
  "private": true,
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "test": "vitest run",
    "lint": "next lint"
  },
  "dependencies": {
    "next": "16.0.4",
    "react": "19.2.0",
    "react-dom": "19.2.0"
  },
  "devDependencies": {
    "eslint": "^9.38.0",
    "eslint-config-next": "16.0.4",
    "vitest": "^4.1.0"
  }
}
`;

/** The finished ad-hoc task a3 from an earlier session (Next.js bump). */
export const NEXT_BUMP: RepoFile[] = [
  { path: 'package.json', before: nextBefore, after: nextBefore.replace(/16\.0\.4/g, '16.1.2').replace('"lint": "next lint"', '"lint": "eslint ."') },
];

// ───────────── the second request: sign-in lockout ─────────────

const rateLimitBefore = `// Fixed-window limits for the public API, kept in Redis.
import { redis } from './queue';

export async function hit(key: string, limit: number, windowSec: number): Promise<boolean> {
  const n = await redis.incr(key);
  if (n === 1) await redis.expire(key, windowSec);
  return n <= limit;
}
`;

const rateLimitAfter = `// Fixed-window limits for the public API, kept in Redis.
import { redis } from './queue';

export async function hit(key: string, limit: number, windowSec: number): Promise<boolean> {
  const n = await redis.incr(key);
  if (n === 1) await redis.expire(key, windowSec);
  return n <= limit;
}

/** Sign-in lockout: 5 failures within 15 minutes lock the email for 15 minutes. */
export const LOCKOUT = { attempts: 5, windowSec: 15 * 60 } as const;
const failKey = (email: string) => \`signin:fail:\${email.trim().toLowerCase()}\`;

export async function lockedFor(email: string): Promise<number> {
  const n = Number(await redis.get(failKey(email))) || 0;
  if (n < LOCKOUT.attempts) return 0;
  return Math.max(1, await redis.ttl(failKey(email)));
}

export async function recordFailure(email: string): Promise<void> {
  const n = await redis.incr(failKey(email));
  if (n === 1) await redis.expire(failKey(email), LOCKOUT.windowSec);
}

export async function clearFailures(email: string): Promise<void> {
  await redis.del(failKey(email));
}
`;

const authBefore = `import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import { db } from './db';
import { verifyPassword } from './password';
import { createSession, readSession } from './session';

export async function requireUser() {
  const session = await readSession(await cookies());
  if (!session) redirect('/sign-in');
  return session.user;
}

export type SignInResult = { ok: true } | { ok: false; error: 'INVALID' };

export async function signIn(email: string, password: string): Promise<SignInResult> {
  const user = await db.user.findUnique({ where: { email } });
  if (!user || !(await verifyPassword(password, user.passwordHash))) return { ok: false, error: 'INVALID' };
  await createSession(user.id);
  return { ok: true };
}
`;

const authAfter = `import { cookies } from 'next/headers';
import { redirect } from 'next/navigation';
import { db } from './db';
import { verifyPassword } from './password';
import { clearFailures, lockedFor, recordFailure } from './rate-limit';
import { createSession, readSession } from './session';

export async function requireUser() {
  const session = await readSession(await cookies());
  if (!session) redirect('/sign-in');
  return session.user;
}

export type SignInResult = { ok: true } | { ok: false; error: 'INVALID' } | { ok: false; error: 'LOCKED'; retryInSec: number };

export async function signIn(email: string, password: string): Promise<SignInResult> {
  const wait = await lockedFor(email);
  if (wait) return { ok: false, error: 'LOCKED', retryInSec: wait };
  const user = await db.user.findUnique({ where: { email } });
  if (!user || !(await verifyPassword(password, user.passwordHash))) {
    await recordFailure(email);
    return { ok: false, error: 'INVALID' };
  }
  await clearFailures(email);
  await createSession(user.id);
  return { ok: true };
}
`;

const rlTestBefore = `import { describe, expect, it } from 'vitest';
import { hit } from './rate-limit';

describe('hit', () => {
  it('allows up to the limit', async () => {
    for (let i = 0; i < 3; i++) expect(await hit('t:a', 3, 60)).toBe(true);
    expect(await hit('t:a', 3, 60)).toBe(false);
  });
});
`;

const rlTestAfter = `import { describe, expect, it, vi } from 'vitest';
import { clearFailures, hit, LOCKOUT, lockedFor, recordFailure } from './rate-limit';

describe('hit', () => {
  it('allows up to the limit', async () => {
    for (let i = 0; i < 3; i++) expect(await hit('t:a', 3, 60)).toBe(true);
    expect(await hit('t:a', 3, 60)).toBe(false);
  });
});

describe('sign-in lockout', () => {
  it('locks after 5 failures', async () => {
    for (let i = 0; i < LOCKOUT.attempts; i++) await recordFailure('a@acme.dev');
    expect(await lockedFor('a@acme.dev')).toBeGreaterThan(0);
  });
  it('is case-insensitive on the email', async () => {
    expect(await lockedFor(' A@acme.dev ')).toBeGreaterThan(0);
  });
  it('clears on success', async () => {
    await clearFailures('a@acme.dev');
    expect(await lockedFor('a@acme.dev')).toBe(0);
  });
  it('lifts after the window', async () => {
    vi.useFakeTimers();
    for (let i = 0; i < LOCKOUT.attempts; i++) await recordFailure('b@acme.dev');
    vi.advanceTimersByTime(LOCKOUT.windowSec * 1000 + 1);
    expect(await lockedFor('b@acme.dev')).toBe(0);
  });
});
`;

const signInBefore = `'use client';
import { useState } from 'react';
import { signInAction } from './actions';

export function SignInForm() {
  const [error, setError] = useState<string | null>(null);
  async function onSubmit(form: FormData) {
    const r = await signInAction(form);
    if (!r.ok) setError('Wrong email or password.');
  }
  return (
    <form action={onSubmit} className="stack">
      <input name="email" type="email" autoComplete="email" required />
      <input name="password" type="password" autoComplete="current-password" required />
      {error && <p role="alert" className="error">{error}</p>}
      <button type="submit">Sign in</button>
    </form>
  );
}
`;

const signInAfter = `'use client';
import { useState } from 'react';
import { signInAction } from './actions';

const minutes = (sec: number) => Math.max(1, Math.ceil(sec / 60));

export function SignInForm() {
  const [error, setError] = useState<string | null>(null);
  const [locked, setLocked] = useState(false);
  async function onSubmit(form: FormData) {
    const r = await signInAction(form);
    if (r.ok) return;
    setLocked(r.error === 'LOCKED');
    setError(
      r.error === 'LOCKED'
        ? \`Too many attempts. Try again in \${minutes(r.retryInSec)} minutes, or reset your password.\`
        : 'Wrong email or password.',
    );
  }
  return (
    <form action={onSubmit} className="stack">
      <input name="email" type="email" autoComplete="email" required />
      <input name="password" type="password" autoComplete="current-password" required disabled={locked} />
      {error && <p role="alert" className="error">{error}</p>}
      <button type="submit" disabled={locked}>Sign in</button>
      {locked && <a href="/reset">Reset your password</a>}
    </form>
  );
}
`;

const lockoutE2e = `import { expect, test } from '@playwright/test';

test('five wrong passwords lock the account for 15 minutes', async ({ page }) => {
  await page.goto('/sign-in');
  for (let i = 0; i < 5; i++) {
    await page.getByLabel('Email').fill('locked@acme.dev');
    await page.getByLabel('Password').fill('wrong-' + i);
    await page.getByRole('button', { name: 'Sign in' }).click();
  }
  await expect(page.getByRole('alert')).toContainText('Too many attempts');
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeDisabled();
});

test('a correct password before the limit still works', async ({ page }) => {
  await page.goto('/sign-in');
  await page.getByLabel('Email').fill('ok@acme.dev');
  await page.getByLabel('Password').fill('correct horse');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await expect(page).toHaveURL('/');
});
`;

/** Files each planned task touches, in the order the worker edits them. */
export const TASK_FILES: Record<string, RepoFile[]> = {
  t1: [
    { path: 'src/theme/ThemeProvider.tsx', before: null, after: themeProvider },
    { path: 'src/theme/useTheme.ts', before: null, after: useTheme },
    { path: 'src/styles/tokens.css', before: tokensBefore, after: tokensAfter },
    { path: 'src/app/layout.tsx', before: layoutBefore, after: layoutAfter },
    { path: 'src/theme/ThemeProvider.test.tsx', before: null, after: themeTest },
  ],
  t2: [
    { path: 'prisma/schema.prisma', before: prismaBefore, after: prismaAfter },
    { path: 'prisma/migrations/20261025093000_add_theme/migration.sql', before: null, after: migration },
    { path: 'src/server/settings/schema.ts', before: schemaBefore, after: schemaAfter },
    { path: 'src/server/settings/route.ts', before: routeBefore, after: routeAfter },
    { path: 'src/server/settings/route.test.ts', before: routeTestBefore, after: routeTestAfter },
  ],
  t3: [
    { path: 'src/app/settings/ThemeToggle.tsx', before: null, after: toggle },
    { path: 'src/app/settings/page.tsx', before: pageBefore, after: pageAfter },
    { path: 'tests/e2e/settings-theme.spec.ts', before: null, after: e2e },
  ],
  // the sign-in lockout plan (its tasks are t1/t2 of their own plan; files keyed l1/l2)
  l1: [
    { path: 'src/server/rate-limit.ts', before: rateLimitBefore, after: rateLimitAfter },
    { path: 'src/server/auth.ts', before: authBefore, after: authAfter },
    { path: 'src/server/rate-limit.test.ts', before: rlTestBefore, after: rlTestAfter },
  ],
  l2: [
    { path: 'src/app/(auth)/sign-in/SignInForm.tsx', before: signInBefore, after: signInAfter },
    { path: 'tests/e2e/auth-lockout.spec.ts', before: null, after: lockoutE2e },
  ],
  adhoc: [{ path: 'package.json', before: pkgBefore, after: pkgAfter }],
};

// ───────────── unified diff generation (LCS, 3 lines of context) ─────────────

type Op = { op: ' ' | '-' | '+'; line: string };

function lcs(a: string[], b: string[]): Op[] {
  const n = a.length;
  const m = b.length;
  const dp: Uint16Array[] = Array.from({ length: n + 1 }, () => new Uint16Array(m + 1));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--) dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: Op[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ op: ' ', line: a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) out.push({ op: '-', line: a[i++] });
    else out.push({ op: '+', line: b[j++] });
  }
  while (i < n) out.push({ op: '-', line: a[i++] });
  while (j < m) out.push({ op: '+', line: b[j++] });
  return out;
}

const lines = (s: string | null) => (s == null ? [] : s.replace(/\n$/, '').split('\n'));
const hash = (s: string) => {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return (h >>> 0).toString(16).padStart(8, '0').slice(0, 7);
};

export function unifiedDiff(f: RepoFile): string {
  const ops = lcs(lines(f.before), lines(f.after));
  const head = [`diff --git a/${f.path} b/${f.path}`];
  if (f.before == null) head.push('new file mode 100644');
  if (f.after == null) head.push('deleted file mode 100644');
  head.push(`index ${f.before == null ? '0000000' : hash(f.before)}..${f.after == null ? '0000000' : hash(f.after)}${f.before && f.after ? ' 100644' : ''}`);
  head.push(f.before == null ? '--- /dev/null' : `--- a/${f.path}`);
  head.push(f.after == null ? '+++ /dev/null' : `+++ b/${f.path}`);

  // number every op
  let o = 1;
  let n = 1;
  const numbered = ops.map((x) => {
    const r = { ...x, o, n };
    if (x.op !== '+') o++;
    if (x.op !== '-') n++;
    return r;
  });
  const changed = numbered.map((x, i) => (x.op !== ' ' ? i : -1)).filter((i) => i >= 0);
  if (!changed.length) return '';
  const ranges: [number, number][] = [];
  for (const i of changed) {
    const s = Math.max(0, i - 3);
    const e = Math.min(numbered.length - 1, i + 3);
    const last = ranges[ranges.length - 1];
    if (last && s <= last[1] + 1) last[1] = Math.max(last[1], e);
    else ranges.push([s, e]);
  }
  const body: string[] = [];
  for (const [s, e] of ranges) {
    const chunk = numbered.slice(s, e + 1);
    const oldCount = chunk.filter((x) => x.op !== '+').length;
    const newCount = chunk.filter((x) => x.op !== '-').length;
    const oldStart = oldCount ? chunk.find((x) => x.op !== '+')!.o : Math.max(0, chunk[0].o - 1);
    const newStart = newCount ? chunk.find((x) => x.op !== '-')!.n : Math.max(0, chunk[0].n - 1);
    // section heading: nearest earlier unchanged line that looks like a declaration
    let section = '';
    for (let i = s - 1; i >= 0; i--) {
      const l = numbered[i].line;
      if (/^(export |async |function |const |model |enum |describe|\s*it\(|[.:\[][\w-]|\w.*\{$)/.test(l) && numbered[i].op !== '+') {
        section = ' ' + l.trim().slice(0, 60);
        break;
      }
    }
    body.push(`@@ -${oldStart},${oldCount} +${newStart},${newCount} @@${section}`);
    for (const x of chunk) body.push(x.op + x.line);
  }
  return [...head, ...body].join('\n');
}

export const diffOf = (files: RepoFile[]) => files.map(unifiedDiff).filter(Boolean).join('\n') + '\n';

// ───────────── search index ─────────────

export type IndexEntry = { path: string; start: number; end: number; symbol: string | null; text: string };

export const INDEX: IndexEntry[] = [
  { path: 'README.md', start: 1, end: 38, symbol: null, text: 'acme web next.js app router prisma postgres vitest playwright setup scripts' },
  { path: 'package.json', start: 1, end: 42, symbol: null, text: 'scripts dev build test lint dependencies next react prisma zod vitest eslint' },
  { path: 'src/app/layout.tsx', start: 1, end: 22, symbol: 'RootLayout', text: 'root layout html body inter font nav main tokens.css metadata' },
  { path: 'src/app/page.tsx', start: 1, end: 48, symbol: 'HomePage', text: 'home page hero dashboard cards recent activity' },
  { path: 'src/app/settings/page.tsx', start: 1, end: 21, symbol: 'SettingsPage', text: 'settings page language weekly email digest requireUser rows' },
  { path: 'src/app/settings/LocaleSelect.tsx', start: 1, end: 34, symbol: 'LocaleSelect', text: 'locale select language dropdown patch settings' },
  { path: 'src/app/settings/DigestSwitch.tsx', start: 1, end: 29, symbol: 'DigestSwitch', text: 'email digest switch toggle patch settings' },
  { path: 'src/components/Nav.tsx', start: 1, end: 57, symbol: 'Nav', text: 'navigation bar links avatar menu sign out' },
  { path: 'src/components/Button.tsx', start: 1, end: 40, symbol: 'Button', text: 'button variants primary ghost danger size' },
  { path: 'src/hooks/useMediaQuery.ts', start: 1, end: 19, symbol: 'useMediaQuery', text: 'media query hook matchMedia prefers color scheme listener' },
  { path: 'src/hooks/useDebounce.ts', start: 1, end: 16, symbol: 'useDebounce', text: 'debounce hook timeout value' },
  { path: 'src/styles/tokens.css', start: 1, end: 23, symbol: ':root', text: 'design tokens colors bg fg border accent radius dark palette theme' },
  { path: 'src/server/db.ts', start: 1, end: 12, symbol: 'db', text: 'prisma client singleton database' },
  { path: 'src/server/auth.ts', start: 1, end: 44, symbol: 'requireUser', text: 'auth session require user cookies redirect sign in' },
  { path: 'src/server/settings/route.ts', start: 1, end: 27, symbol: 'PATCH', text: 'settings api route get patch user settings upsert locale email digest json' },
  { path: 'src/server/settings/schema.ts', start: 1, end: 12, symbol: 'settingsPatch', text: 'zod schema settings patch locale email digest strict partial' },
  { path: 'src/server/settings/route.test.ts', start: 1, end: 16, symbol: null, text: 'settings api tests vitest defaults rejects unknown fields' },
  { path: 'src/server/checkout/route.ts', start: 1, end: 63, symbol: 'createCheckoutSession', text: 'stripe checkout session price line items webhook' },
  { path: 'src/server/billing/route.ts', start: 1, end: 51, symbol: 'GET', text: 'billing invoices subscription plan portal' },
  { path: 'prisma/schema.prisma', start: 18, end: 25, symbol: 'UserSettings', text: 'prisma model user settings locale email digest updated at' },
  { path: 'prisma/schema.prisma', start: 10, end: 16, symbol: 'User', text: 'prisma model user email name settings created at' },
  { path: 'tests/e2e/checkout.spec.ts', start: 1, end: 58, symbol: null, text: 'playwright checkout e2e stripe test card flaky' },
  { path: 'vitest.config.ts', start: 1, end: 18, symbol: null, text: 'vitest config jsdom setup files alias' },
  { path: 'playwright.config.ts', start: 1, end: 31, symbol: null, text: 'playwright config base url web server projects chromium' },
  { path: 'eslint.config.js', start: 1, end: 24, symbol: null, text: 'eslint flat config next typescript rules' },
  { path: '.github/workflows/ci.yml', start: 1, end: 46, symbol: null, text: 'ci workflow install build test lint playwright' },
];

export const INDEX_AFTER: Record<string, IndexEntry[]> = {
  t1: [
    { path: 'src/theme/ThemeProvider.tsx', start: 24, end: 44, symbol: 'ThemeProvider', text: 'theme provider context system light dark resolved data-theme localStorage media query' },
    { path: 'src/theme/useTheme.ts', start: 1, end: 9, symbol: 'useTheme', text: 'use theme hook context setTheme resolved' },
  ],
  t2: [{ path: 'prisma/migrations/20261025093000_add_theme/migration.sql', start: 1, end: 5, symbol: null, text: 'migration theme enum user settings column default system' }],
  t3: [
    { path: 'src/app/settings/ThemeToggle.tsx', start: 14, end: 50, symbol: 'ThemeToggle', text: 'theme toggle segmented radio system light dark patch settings await' },
    { path: 'tests/e2e/settings-theme.spec.ts', start: 1, end: 24, symbol: null, text: 'playwright settings theme e2e persists reload dark' },
  ],
};

export const REPO_LS = [
  '.github', 'README.md', 'eslint.config.js', 'next.config.ts', 'package.json', 'playwright.config.ts',
  'prisma', 'public', 'src', 'tests', 'tsconfig.json', 'vitest.config.ts',
];
