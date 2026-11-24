//! "Said in conversations": the messages of the open repo whose text holds a query, for the
//! palette. The query is in `store` (`Store::search_chats`); this only picks the workspace.

use super::*;

/// How many messages one search returns at most.
const CHAT_HITS: usize = 40;

impl Core {
    /// The messages of the open repo whose text holds `query` (ASCII case aside), newest first:
    /// open requests, 1:1 chats and closed requests. Fewer than 2 characters find nothing.
    pub async fn search_chats(&self, query: &str) -> anyhow::Result<Vec<ChatMessage>> {
        let ws = self.inner.require_ws()?;
        self.inner.store.search_chats(&ws.id, query, CHAT_HITS)
    }
}
