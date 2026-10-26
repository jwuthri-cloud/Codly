// Artifact bytes → Blob URLs. Bytes come from `api.readArtifact(id)` (MIME on the artifact).
// Blobs are cached (bounded); object URLs are reference-counted and revoked as soon as the last
// view that uses them unmounts.
import { api } from '../api';
import type { Artifact } from '../gen/Artifact';

const MAX_CACHE = 48 * 1024 * 1024;
const blobs = new Map<string, Blob>();
const pending = new Map<string, Promise<Blob>>();
const urls = new Map<string, { url: string; refs: number }>();

async function blobOf(a: Pick<Artifact, 'id' | 'mime'>): Promise<Blob> {
  const hit = blobs.get(a.id);
  if (hit) {
    blobs.delete(a.id);
    blobs.set(a.id, hit); // LRU touch
    return hit;
  }
  let p = pending.get(a.id);
  if (!p) {
    p = api.readArtifact(a.id).then((buf) => {
      const b = new Blob([buf], { type: a.mime || 'application/octet-stream' });
      blobs.set(a.id, b);
      let total = 0;
      for (const x of blobs.values()) total += x.size;
      for (const [id, x] of blobs) {
        if (total <= MAX_CACHE || id === a.id) break;
        if (urls.has(id)) continue;
        blobs.delete(id);
        total -= x.size;
      }
      return b;
    });
    pending.set(a.id, p);
    p.finally(() => pending.delete(a.id)).catch(() => {});
  }
  return p;
}

/** An object URL for the artifact; pair every call with `releaseArtifactUrl(id)`. */
export async function acquireArtifactUrl(a: Pick<Artifact, 'id' | 'mime'>): Promise<string> {
  const have = urls.get(a.id);
  if (have) {
    have.refs++;
    return have.url;
  }
  const b = await blobOf(a);
  const again = urls.get(a.id);
  if (again) {
    again.refs++;
    return again.url;
  }
  const url = URL.createObjectURL(b);
  urls.set(a.id, { url, refs: 1 });
  return url;
}

export function releaseArtifactUrl(id: string) {
  const u = urls.get(id);
  if (!u) return;
  u.refs--;
  if (u.refs <= 0) {
    URL.revokeObjectURL(u.url);
    urls.delete(id);
  }
}

/** Text content (Markdown, CSV, HTML source, SVG source). */
export async function artifactText(a: Pick<Artifact, 'id' | 'mime'>): Promise<string> {
  return (await blobOf(a)).text();
}

/** Minimal RFC-4180 CSV parser (quotes, escaped quotes, CRLF). */
export function parseCsv(text: string, maxRows = 500): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = '';
  let q = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (q) {
      if (c === '"' && text[i + 1] === '"') {
        cell += '"';
        i++;
      } else if (c === '"') q = false;
      else cell += c;
    } else if (c === '"') q = true;
    else if (c === ',') {
      row.push(cell);
      cell = '';
    } else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(cell);
      rows.push(row);
      row = [];
      cell = '';
      if (rows.length >= maxRows) return rows;
    } else cell += c;
  }
  if (cell || row.length) {
    row.push(cell);
    rows.push(row);
  }
  return rows;
}

export const KIND_LABEL: Record<Artifact['kind'], string> = {
  image: 'Image',
  svg: 'SVG',
  html: 'Web page',
  markdown: 'Document',
  pdf: 'PDF',
  csv: 'Table',
};

export const KIND_ICON: Record<Artifact['kind'], string> = {
  image: 'image',
  svg: 'shapes',
  html: 'globe',
  markdown: 'book',
  pdf: 'file',
  csv: 'table',
};

export function prettyBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
