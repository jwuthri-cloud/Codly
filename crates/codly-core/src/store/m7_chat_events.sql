-- The event of each message by the message's id ("Load earlier" in a request's thread). The
-- expression and the WHERE are those of `THREAD_MESSAGES_SQL`: SQLite uses the index only when
-- the query spells them the same way. Only an index: no row changes, safe on a database of any size.
CREATE INDEX event_chat ON event(json_extract(payload, '$.message.id')) WHERE type = 'chat';
