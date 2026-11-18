---
name: design-experiment
description: Turn a research question into a concrete, reproducible experiment plan with hypothesis, baseline, controls, metrics, sample size and a decision rule. Use when the user says "design an experiment", "how would we test this?", "plan the eval", or before any run that costs real compute.
---

# Design an experiment

Write the plan as a Markdown file in your worktree (`experiments/<short-name>.md`) and give its path.
Answer first in two sentences: what you will test and how you will decide.

## The plan

1. **Question** - one sentence.
2. **Hypothesis** - what you expect and why; the **null** and the main **alternative explanation**.
3. **Setup** - models (exact names and versions), data (source, split, size), code entry point.
4. **Baseline** - the strongest simple thing it must beat, and a sanity baseline (random / majority /
   no-intervention).
5. **Controls and ablations** - what is held fixed; which single factor each ablation removes.
6. **Metrics** - primary (one) and secondary; how each is computed; known failure modes of the metric.
7. **Sample size and uncertainty** - n per condition, seeds, how you will report spread (CI or SE),
   and a rough power argument: the smallest effect worth detecting and whether n can see it.
8. **Decision rule** - written before the run: "if primary metric improves by at least X over the
   baseline with the CI excluding 0, we do A; otherwise B".
9. **Threats to validity** - leakage, contamination, prompt sensitivity, judge bias (for LLM judges:
   calibration against a human-labelled sample).
10. **Cost** - GPU-hours or API calls, wall-clock time; the cheapest pilot to run first (1-2 jobs).
11. **Reproducibility** - seed, pinned versions, config, exact command, commit.

Do not launch anything. Propose the exact pilot command and let the user run it.
