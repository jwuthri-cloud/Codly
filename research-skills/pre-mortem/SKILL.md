---
name: pre-mortem
description: Imagine a research project or experiment already failed and work out why, before starting it. Use when the user says "pre-mortem", "what could go wrong", "risks of this plan", or is about to commit weeks or a large compute budget to a plan.
---

# Pre-mortem

Assume it is three months from now and the project failed or produced nothing usable. Explain why.

1. **The plan** in three sentences: goal, method, the result that would count as success.
2. **Failure stories** - five to eight concrete ways it failed, each one sentence, across:
   - the hypothesis was wrong or the effect too small to see;
   - a confound or leakage explained the result;
   - the metric or judge did not measure what mattered;
   - infrastructure, data or compute problems ate the time;
   - someone published it first, or it does not matter to the field;
   - scope creep or no clear stopping point.
3. **Rank** them by likelihood x damage. For the top three: an early warning sign, and a cheap
   mitigation or check to do in week one.
4. **Kill criteria** - written now: the observations that should make the user stop or pivot.
5. **Changes to the plan** - at most three, concrete.

Keep it to one page. Save the kill criteria with `codly_memory_write`.
