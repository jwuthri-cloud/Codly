-- Indexes for reads that walked a whole table, so they got slower as the history grew.
-- Only indexes: no row changes, safe on a database of any size.

-- Reads of one kind of event (open questions, artifacts, memories), in id order.
CREATE INDEX event_type ON event(type, id);

-- One artifact by id. The expression and the WHERE are those of `Store::artifact`: SQLite uses
-- the index only when the query spells them the same way.
CREATE INDEX event_artifact ON event(json_extract(payload, '$.artifact.id')) WHERE type = 'artifact_added';

-- The newest run of a task (asked at every gate).
CREATE INDEX run_task ON run(task_id, started_at);
