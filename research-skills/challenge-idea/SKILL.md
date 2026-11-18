---
name: challenge-idea
description: Stress-test a research idea or claim before time goes into it. Use when the user says "challenge this", "poke holes", "is this a good idea?", "steelman / red-team my idea", or proposes a new research direction, hypothesis or result interpretation.
---

# Challenge an idea

Goal: find the strongest reasons the idea is wrong, already done, or not worth it, and the cheapest
test that would tell. Be direct; the user wants friction, not encouragement.

1. **Restate** the idea in one sentence, as a falsifiable claim. If it cannot be made falsifiable, say so
   and stop there.
2. **Steelman** it in two or three sentences: the best case for why it matters.
3. **Attack** it. For each angle, one or two sentences, most damaging first:
   - Prior work: has it been done? Search (WebSearch, arXiv, OpenAlex, Semantic Scholar) and cite only
     what you opened. "I found nothing" is a result; say how you searched.
   - Confounds and alternative explanations for the expected result.
   - Measurement: does the metric measure the thing? Goodhart risks.
   - Generalization: toy setting vs. frontier models, one model family, one dataset.
   - Base rates: how often does this kind of idea work?
   - Cost vs. value: what changes in the world (or in a decision) if it works?
4. **Cheapest decisive test**: the smallest experiment (hours, not weeks) whose result would change the
   plan. Give the expected result under the idea and under the main alternative.
5. **Verdict**: go / revise / drop, with confidence (low / medium / high) and the one thing that would
   change your mind.

Keep it under a page. Save the verdict and the test with `codly_memory_write` if the user keeps the idea.
