---
name: lit-review
description: Map the literature on a research question - key papers, what each shows, where they agree and disagree, and the open gap. Use when the user asks "what's known about X", "related work", "lit review", "who has worked on this", or "what are the latest papers on X".
---

# Literature review

Answer first in two or three sentences: the state of the field and the main open gap.

1. **Search** broadly, then narrow: arXiv API (`export.arxiv.org/api/query`, sort by date for recent
   work), OpenAlex (`api.openalex.org/works?search=`), Semantic Scholar (citations and references of
   the key papers), WebSearch for lab blogs and the Alignment Forum. Note the queries you used.
2. **Select** 5-12 papers: the founding ones, the most cited, and the newest. Prefer primary sources.
3. **For each paper** (open it; cite only what you opened): title, authors, year, venue or arXiv id,
   link; the claim; the method; the key number(s); the main limitation.
4. **Synthesize**: themes, where results agree, where they conflict (and why: setup, scale, metric),
   and what nobody has tested yet.
5. **Gap and next step**: the most promising open question for the user, and one cheap first
   experiment (hand it to design-experiment if the user wants a plan).

Write the review to `notes/lit-<topic>.md` in your worktree, give the path, and save the gap with
`codly_memory_write`. Say how recent the newest source is.
