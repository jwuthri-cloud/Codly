-- The sign-in (account id, e.g. "claude:/Users/me/.claude-work") a run used. A session
-- lives in that account's config home, so it can only be resumed there. NULL = the CLI's
-- primary sign-in (runs from before accounts).
ALTER TABLE run ADD COLUMN account TEXT;
