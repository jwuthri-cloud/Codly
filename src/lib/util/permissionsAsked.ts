// A Codex agent asks for more than its sandbox gives it (`item/permissions/requestApproval`):
// what it asks, in words. The shape is `PermissionsRequestApprovalParams` of the Codex app-server
// protocol: `cwd`, `reason` and `permissions: RequestPermissionProfile` = `{ network: { enabled }
// | null, fileSystem: { read: path[] | null, write: path[] | null, entries?: [{ path, access }] }
// | null }`, where an entry's path is `{ type: "path", path }`, `{ type: "glob_pattern", pattern }`
// or `{ type: "special", value: { kind } }`. The request does not say for how long: the answer
// does (Codly answers "Allow for this turn" with scope "turn", "Allow for this session" with
// scope "session").
import { oneLine, tildify } from './sanitize';

export type PermissionLine = {
  /** "Write in", "Read", "Use the network", "Keep out of". */
  verb: string;
  /** The folder, file or pattern, as it reads best (relative to where the agent works when near it); null for the network. */
  place: string | null;
  /** The place is not inside the folder the agent works in. */
  outside: boolean;
};

export type PermissionsAsk = {
  /** After "<agent> wants to": "write in ../acme-docs", "read 2 folders and use the network". */
  wants: string;
  /** One line per thing it asks for, in the request's order; empty when Codly cannot read the shape. */
  lines: PermissionLine[];
  /** The folder the agent works in, as the request says it. */
  cwd: string | null;
  /** Codly read the request: false for a shape it does not know (the card then shows the raw detail). */
  known: boolean;
};

const UNKNOWN = 'get more permissions';

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);
const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && x !== '') : []);
const trimEnd = (p: string) => (p.length > 1 ? p.replace(/\/+$/, '') : p);

/** Whether `path` is `dir` or under it. */
export function within(path: string, dir: string): boolean {
  const p = trimEnd(path);
  const d = trimEnd(dir);
  return p === d || p.startsWith(d === '/' ? '/' : `${d}/`);
}

/**
 * A path as it reads best: relative to `cwd` when it is inside it ("docs/x") or at most two
 * folders up ("../acme-docs"), else whole with "~" for the home folder. A relative path stays as it is.
 */
export function placeOf(path: string, cwd: string | null): string {
  if (!path.startsWith('/') || !cwd || !cwd.startsWith('/')) return tildify(path);
  const p = trimEnd(path).split('/').filter(Boolean);
  const c = trimEnd(cwd).split('/').filter(Boolean);
  let same = 0;
  while (same < p.length && same < c.length && p[same] === c[same]) same++;
  const up = c.length - same;
  if (same === 0 || up > 2) return tildify(path);
  const rest = p.slice(same);
  if (up === 0) return rest.length ? rest.join('/') : '.';
  return [...Array(up).fill('..'), ...rest].join('/');
}

const SPECIAL: Record<string, string> = {
  root: 'the whole disk',
  minimal: 'the system folders a command needs',
  tmpdir: 'the temporary folder',
  slash_tmp: '/tmp',
};

/** One sandbox entry (`FileSystemSandboxEntry`) as a line; null for a shape Codly does not know. */
function entryLine(e: unknown, cwd: string | null): PermissionLine | null {
  if (!isObj(e) || !isObj(e.path)) return null;
  const verb = e.access === 'write' ? 'Write in' : e.access === 'read' ? 'Read' : e.access === 'deny' ? 'Keep out of' : null;
  if (!verb) return null;
  const p = e.path;
  if (p.type === 'path' && typeof p.path === 'string' && p.path) {
    return { verb, place: placeOf(p.path, cwd), outside: !!cwd && p.path.startsWith('/') && !within(p.path, cwd) };
  }
  if (p.type === 'glob_pattern' && typeof p.pattern === 'string' && p.pattern) {
    return { verb: `${verb} the files that match`, place: p.pattern, outside: false };
  }
  if (p.type === 'special' && isObj(p.value) && typeof p.value.kind === 'string') {
    const kind = p.value.kind;
    const sub = typeof p.value.subpath === 'string' && p.value.subpath ? p.value.subpath : null;
    if (kind === 'project_roots') return { verb, place: sub ? `${sub} in the project folders` : 'the project folders', outside: false };
    if (kind === 'unknown' && typeof p.value.path === 'string') return { verb, place: tildify(p.value.path), outside: true };
    const named = SPECIAL[kind];
    return named ? { verb, place: named, outside: kind !== 'minimal' } : null;
  }
  return null;
}

/** What a permissions request asks for, in words. */
export function permissionsAsked(detail: unknown): PermissionsAsk {
  const d = isObj(detail) ? detail : {};
  const cwd = typeof d.cwd === 'string' && d.cwd ? d.cwd : null;
  const perms = d.permissions;
  if (!isObj(perms) || !('network' in perms || 'fileSystem' in perms)) return { wants: UNKNOWN, lines: [], cwd, known: false };

  const lines: PermissionLine[] = [];
  let unread = false;
  const add = (l: PermissionLine | null) => {
    if (!l) unread = true;
    else if (!lines.some((x) => x.verb === l.verb && x.place === l.place)) lines.push(l);
  };
  const fs = perms.fileSystem;
  if (isObj(fs)) {
    // `read` and `write` are the older form of `entries`: a request may carry both, for the same paths.
    for (const p of strings(fs.write)) add({ verb: 'Write in', place: placeOf(p, cwd), outside: !!cwd && p.startsWith('/') && !within(p, cwd) });
    for (const p of strings(fs.read)) add({ verb: 'Read', place: placeOf(p, cwd), outside: !!cwd && p.startsWith('/') && !within(p, cwd) });
    if (Array.isArray(fs.entries)) for (const e of fs.entries) add(entryLine(e, cwd));
  } else if (fs != null) unread = true;
  const net = perms.network;
  if (isObj(net) && net.enabled === true) lines.push({ verb: 'Use the network', place: null, outside: false });
  else if (net != null && !isObj(net)) unread = true;

  if (!lines.length) return { wants: UNKNOWN, lines, cwd, known: !unread };

  // A "Keep out of" line narrows what it asks: the headline names only what it gets.
  const parts: string[] = [];
  for (const kind of ['Write', 'Read'] as const) {
    const these = lines.filter((l) => l.place !== null && l.verb.startsWith(kind));
    if (these.length === 1) parts.push(`${these[0].verb.toLowerCase()} ${oneLine(these[0].place ?? '', 60)}`);
    else if (these.length) parts.push(`${kind === 'Write' ? 'write in' : 'read'} ${these.length} places`);
  }
  if (lines.some((l) => l.place === null)) parts.push('use the network');
  if (!parts.length) return { wants: UNKNOWN, lines, cwd, known: !unread };
  const wants = parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
  return { wants, lines, cwd, known: !unread };
}

/** One line of the card: "Write in ../acme-docs". */
export const lineText = (l: PermissionLine): string => (l.place === null ? l.verb : `${l.verb} ${l.place}`);

/** How long an allow of a permissions request lasts, by the choice: the label of its button. */
export const PERMISSION_SCOPE = { allow_once: 'Allow for this turn', allow_session: 'Allow for this session' } as const;
