// Unified diff parser (git format) for the diff viewer.

export type DiffLine = {
  kind: 'ctx' | 'add' | 'del' | 'meta';
  text: string;
  oldNo: number | null;
  newNo: number | null;
};

export type DiffHunk = { header: string; section: string; oldStart: number; newStart: number; lines: DiffLine[] };

export type DiffFile = {
  oldPath: string;
  newPath: string;
  path: string;
  status: 'added' | 'deleted' | 'modified' | 'renamed' | 'binary';
  hunks: DiffHunk[];
  added: number;
  removed: number;
};

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@ ?(.*)$/;

const strip = (p: string) => p.replace(/^"?[ab]\//, '').replace(/"$/, '');

export function parseDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;
  let oldNo = 0;
  let newNo = 0;
  const lines = text.replace(/\r\n/g, '\n').split('\n');

  const start = (oldPath: string, newPath: string) => {
    file = { oldPath, newPath, path: newPath, status: 'modified', hunks: [], added: 0, removed: 0 };
    files.push(file);
    hunk = null;
  };

  for (const line of lines) {
    if (line.startsWith('diff --git ')) {
      const m = /^diff --git (\S+) (\S+)$/.exec(line);
      start(m ? strip(m[1]) : '', m ? strip(m[2]) : '');
      continue;
    }
    if (!file && (line.startsWith('--- ') || line.startsWith('+++ '))) start('', '');
    const f = file as DiffFile | null;
    if (!f) continue;
    if (!hunk) {
      if (line.startsWith('new file mode')) f.status = 'added';
      else if (line.startsWith('deleted file mode')) f.status = 'deleted';
      else if (line.startsWith('rename from ')) { f.status = 'renamed'; f.oldPath = line.slice(12); }
      else if (line.startsWith('rename to ')) { f.newPath = f.path = line.slice(10); }
      else if (line.startsWith('Binary files')) f.status = 'binary';
      else if (line.startsWith('--- ')) {
        const p = line.slice(4);
        if (p === '/dev/null') f.status = 'added';
        else f.oldPath = strip(p);
      } else if (line.startsWith('+++ ')) {
        const p = line.slice(4);
        if (p === '/dev/null') { f.status = 'deleted'; f.path = f.oldPath; }
        else { f.newPath = f.path = strip(p); }
      }
    }
    const hm = HUNK.exec(line);
    if (hm) {
      oldNo = +hm[1];
      newNo = +hm[3];
      hunk = { header: line, section: hm[5] ?? '', oldStart: oldNo, newStart: newNo, lines: [] };
      f.hunks.push(hunk);
      continue;
    }
    const h = hunk as DiffHunk | null;
    if (!h) continue;
    const c = line[0];
    if (c === '+') { h.lines.push({ kind: 'add', text: line.slice(1), oldNo: null, newNo: newNo++ }); f.added++; }
    else if (c === '-') { h.lines.push({ kind: 'del', text: line.slice(1), oldNo: oldNo++, newNo: null }); f.removed++; }
    else if (c === ' ') h.lines.push({ kind: 'ctx', text: line.slice(1), oldNo: oldNo++, newNo: newNo++ });
    else if (c === '\\') h.lines.push({ kind: 'meta', text: line, oldNo: null, newNo: null });
  }
  return files;
}

export function diffStats(files: DiffFile[]) {
  return files.reduce((a, f) => ({ added: a.added + f.added, removed: a.removed + f.removed }), { added: 0, removed: 0 });
}
