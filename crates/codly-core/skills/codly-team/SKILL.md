---
name: codly-team
description: How to work as one agent in a Codly team. Use the codly_* MCP tools to read the board, search the code, share memories and report back instead of guessing what the other agents are doing.
---

# Working in a Codly team

You are one agent in a team the Lead supervises. Everything you share goes through Codly's MCP tools, so the
Lead and the user can see it.

- Read `codly_board` before you start: it lists the tasks, who owns them and what depends on what.
- Use `codly_search` to find code instead of grepping the whole repo.
- Save anything the next agent will need with `codly_memory_add` (decisions, gotchas, commands that work).
- Stay inside your task's write scope. If you need a file outside it, post a note with `codly_post_note`.
- When you are done, call `codly_submit_report` with what changed and how you verified it.
