-- Codly main database (codly.sqlite, WAL). Migration 1.
-- The `event` table is the source of truth; every other table is a projection the
-- orchestrator maintains in the same transaction as the event that changed it.
-- JSON columns hold serde_json of the matching model.rs type or list.

CREATE TABLE workspace (
  id             TEXT PRIMARY KEY,
  repo_path      TEXT NOT NULL UNIQUE,
  name           TEXT NOT NULL,
  default_branch TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  opened_at      INTEGER NOT NULL
);

-- key/value settings; key 'settings' holds the Settings JSON (global, v1).
CREATE TABLE kv (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE agent (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT REFERENCES workspace(id),
  name         TEXT NOT NULL,
  provider     TEXT NOT NULL,
  role         TEXT NOT NULL,
  json         TEXT NOT NULL,             -- full Agent (status, usage, activity…)
  archived     INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX agent_ws ON agent(workspace_id, archived);

CREATE TABLE plan (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspace(id),
  status       TEXT NOT NULL,
  json         TEXT NOT NULL,             -- full Plan
  plan_json    TEXT NOT NULL DEFAULT '{}',-- validated planner output (integration_checks…)
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX plan_ws ON plan(workspace_id, created_at);

CREATE TABLE task (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspace(id),
  plan_id      TEXT REFERENCES plan(id),
  status       TEXT NOT NULL,
  json         TEXT NOT NULL,             -- full Task
  base_commit  TEXT,
  head_commit  TEXT,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX task_ws ON task(workspace_id, status);
CREATE INDEX task_plan ON task(plan_id);

CREATE TABLE run (
  id          TEXT PRIMARY KEY,
  agent_id    TEXT NOT NULL REFERENCES agent(id),
  task_id     TEXT REFERENCES task(id),
  role        TEXT NOT NULL,
  provider    TEXT NOT NULL,
  model       TEXT,
  cwd         TEXT NOT NULL,
  session_id  TEXT,
  status      TEXT NOT NULL,              -- starting|running|completed|failed|cancelled|lost
  raw_log     TEXT NOT NULL,
  report_json TEXT,                       -- codly_submit_report / codly_submit_review payload
  started_at  INTEGER NOT NULL,
  ended_at    INTEGER
);
CREATE INDEX run_agent ON run(agent_id, started_at);

CREATE TABLE event (
  id           INTEGER PRIMARY KEY,       -- the global UI cursor (Snapshot.as_of)
  ts           INTEGER NOT NULL,
  workspace_id TEXT,
  agent_id     TEXT,
  task_id      TEXT,
  run_id       TEXT,
  type         TEXT NOT NULL,
  payload      TEXT NOT NULL              -- EventKind JSON
);
CREATE INDEX event_agent ON event(agent_id, id);
CREATE INDEX event_task ON event(task_id, id);
CREATE INDEX event_ws ON event(workspace_id, id);

CREATE TABLE chat (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT,
  thread       TEXT NOT NULL,
  json         TEXT NOT NULL,             -- full ChatMessage
  ts           INTEGER NOT NULL
);
CREATE INDEX chat_thread ON chat(workspace_id, thread, ts);

CREATE TABLE approval (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT,
  run_id       TEXT NOT NULL,
  status       TEXT NOT NULL,
  json         TEXT NOT NULL,             -- full Approval
  created_at   INTEGER NOT NULL,
  decided_at   INTEGER
);
CREATE INDEX approval_status ON approval(status);

-- Explainability + calibration log: every routing/scheduling/gate/merge/policy decision.
CREATE TABLE decision (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT,
  plan_id      TEXT,
  task_id      TEXT,
  type         TEXT NOT NULL,             -- route|plan_validate|schedule|gate|review|merge|retry|approval_policy|context
  method       TEXT NOT NULL,             -- rule|model|human
  summary      TEXT NOT NULL,
  detail       TEXT NOT NULL,             -- JSON: inputs, alternatives, scores
  outcome      TEXT,
  created_at   INTEGER NOT NULL
);

-- Long-term memory: shared (global/project), private (agent) and task-scoped.
CREATE TABLE memory (
  rowid_       INTEGER PRIMARY KEY,       -- FTS rowid
  id           TEXT NOT NULL UNIQUE,
  workspace_id TEXT,
  scope        TEXT NOT NULL,             -- global|project|agent|task
  scope_id     TEXT,
  kind         TEXT NOT NULL,             -- fact|decision|lesson|preference|note|summary
  text         TEXT NOT NULL,
  author       TEXT NOT NULL,
  source_event INTEGER,
  importance   REAL NOT NULL DEFAULT 0.5,
  pinned       INTEGER NOT NULL DEFAULT 0,
  superseded_by TEXT,
  created_at   INTEGER NOT NULL,
  accessed_at  INTEGER NOT NULL
);
CREATE INDEX memory_scope ON memory(workspace_id, scope, scope_id);
CREATE VIRTUAL TABLE memory_fts USING fts5(
  text, content='memory', content_rowid='rowid_', tokenize='porter unicode61 remove_diacritics 2'
);
CREATE TRIGGER memory_ai AFTER INSERT ON memory BEGIN
  INSERT INTO memory_fts(rowid, text) VALUES (new.rowid_, new.text);
END;
CREATE TRIGGER memory_ad AFTER DELETE ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, text) VALUES ('delete', old.rowid_, old.text);
END;
CREATE TRIGGER memory_au AFTER UPDATE OF text ON memory BEGIN
  INSERT INTO memory_fts(memory_fts, rowid, text) VALUES ('delete', old.rowid_, old.text);
  INSERT INTO memory_fts(rowid, text) VALUES (new.rowid_, new.text);
END;

-- Knowledge graph: temporal (subject, predicate, object) facts.
CREATE TABLE triple (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT,
  subject      TEXT NOT NULL,
  predicate    TEXT NOT NULL,
  object       TEXT NOT NULL,
  source       TEXT NOT NULL,             -- observed|agent|user
  source_event INTEGER,
  confidence   REAL NOT NULL DEFAULT 1.0,
  valid_from   INTEGER NOT NULL,
  valid_to     INTEGER
);
CREATE INDEX triple_s ON triple(workspace_id, subject, predicate);
CREATE INDEX triple_o ON triple(workspace_id, object, predicate);
CREATE UNIQUE INDEX triple_live ON triple(workspace_id, subject, predicate, object) WHERE valid_to IS NULL;
