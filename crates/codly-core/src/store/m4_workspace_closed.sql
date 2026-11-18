-- A repo you closed in Codly (Close office): hidden from the map, the recent list and the offices,
-- its plot freed. Nothing else is deleted: opening the repo again clears it and brings it all back.
ALTER TABLE workspace ADD COLUMN closed_at INTEGER;
