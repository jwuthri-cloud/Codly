-- "Always allow in this repo": commands you told Codly not to ask about again
-- (orchestrator/approval_rules.rs). They live here, not in the repo and not in an agent's
-- worktree: a rule is yours, every agent of the repo follows it, and it outlives every worktree.
CREATE TABLE approval_rule (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspace(id),
  kind         TEXT NOT NULL,             -- exact|prefix
  command      TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  UNIQUE (workspace_id, kind, command)
);

-- The rules a request can be saved as (ApprovalRuleOffer JSON), written when Codly asks you.
-- NULL: never asked, or nothing can be saved from it.
ALTER TABLE approval ADD COLUMN rule_offer TEXT;
