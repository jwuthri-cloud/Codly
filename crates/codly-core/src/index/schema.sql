-- Private schema of the per-repo code index (one SQLite file per workspace, WAL).
-- The index is a cache: it can be deleted and rebuilt, so a version or tokenizer change
-- simply drops and recreates everything (see store::init).
--
-- `{TOKENIZE}` is `unicode61 remove_diacritics 2`, optionally wrapped in `porter`.
-- Identifier splitting (camelCase) is done in Rust before insert; see terms.rs.

CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS files (
  id       INTEGER PRIMARY KEY,      -- also the rowid in fts_files
  path     TEXT NOT NULL UNIQUE,     -- repo-relative, '/' separated
  lang     TEXT,
  bytes    INTEGER NOT NULL,
  lines    INTEGER NOT NULL,
  mtime_ns INTEGER NOT NULL,
  hash     INTEGER NOT NULL          -- FNV-1a of the content: skips rewrites when only mtime moved
);

-- Files we looked at and rejected (binary, minified, generated…); remembered by mtime+size
-- so a refresh does not read them again.
CREATE TABLE IF NOT EXISTS skipped (
  path     TEXT PRIMARY KEY,
  mtime_ns INTEGER NOT NULL,
  bytes    INTEGER NOT NULL,
  reason   TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS symbols (
  id        INTEGER PRIMARY KEY,
  file_id   INTEGER NOT NULL,
  name      TEXT NOT NULL,
  kind      TEXT NOT NULL,
  container TEXT,
  line      INTEGER NOT NULL,
  end_line  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS symbols_file ON symbols(file_id);

CREATE TABLE IF NOT EXISTS chunks (
  id         INTEGER PRIMARY KEY,    -- also the rowid in fts_chunks
  file_id    INTEGER NOT NULL,
  start_line INTEGER NOT NULL,
  end_line   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS chunks_file ON chunks(file_id);

-- One row per file: path terms, symbol-name terms, leading-comment terms (weighted bm25).
CREATE VIRTUAL TABLE IF NOT EXISTS fts_files USING fts5(
  path_terms, symbol_terms, doc_terms,
  content='', contentless_delete=1, tokenize='{TOKENIZE}'
);

-- One row per ~60-line chunk. Contentless: the text stays on disk, citations only need
-- the line range.
CREATE VIRTUAL TABLE IF NOT EXISTS fts_chunks USING fts5(
  body_terms,
  content='', contentless_delete=1, tokenize='{TOKENIZE}'
);
