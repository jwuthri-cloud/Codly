-- Structured part of a memory: JSON {"title": str, "tags": [str], "refs": [entity id]}.
-- `text` stays the body that is searched (FTS) and packed into agent briefs; refs use the
-- knowledge graph's entity ids ("task:<id>", "file:<path>", "concept:<name>", "agent:<id>").
ALTER TABLE memory ADD COLUMN data TEXT;
