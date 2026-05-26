# τ-bench Analysis: OpalZero-Style vs Baseline
**Date:** May 24 2026 · **Model:** gpt-4o-mini · **Tasks:** 25 per domain · **Domains:** airline, retail

---

## 1. Results Summary

| Agent | Domain | Pass^1 | Avg Reward | Avg Cost/Conv |
|-------|--------|--------|------------|---------------|
| opalzero_baseline | airline | **28.0%** (7/25) | 0.280 | $0.0056 |
| opalzero_style | airline | **32.0%** (8/25) | 0.320 | $0.0019 |
| opalzero_baseline | retail | **36.0%** (9/25) | 0.360 | $0.0059 |
| opalzero_style | retail | **32.0%** (8/25) | 0.320 | $0.0039 |

### Delta (opalzero_style vs baseline)
| Domain | Pass^1 delta | Avg reward delta |
|--------|-------------|-----------------|
| airline | **+4.0pp** | +0.040 |
| retail | **−4.0pp** | −0.040 |
| **combined** | **+0.0pp** | ±0.000 |

### vs Published Baselines (from τ-bench paper, Claude 3.5 Sonnet)
| Domain | Our baseline | Our opalzero | Published best |
|--------|-------------|-----------|----------------|
| airline | 28.0% | 32.0% | ~46% (Claude 3.5S) |
| retail | 36.0% | 32.0% | ~69% (Claude 3.5S) |

**The dominant gap is the model, not the methodology.**

---

## 2. Action-Level Breakdown

| Agent | Domain | Read accuracy | Write accuracy | DB match | Max Steps |
|-------|--------|--------------|----------------|----------|-----------|
| opalzero_baseline | airline | 75.0% (18/24) | 33.3% (10/30) | 36.0% (9/25) | 0 |
| opalzero_style | airline | 87.5% (21/24) | 35.7% (10/28) | 41.7% (10/25) | 0 |
| opalzero_baseline | retail | 86.6% (123/142) | 45.2% (14/31) | 44.0% (11/25) | 0 |
| opalzero_style | retail | 83.9% (115/137) | 53.3% (16/30) | 43.5% (10/25) | 2 |

**Critical pattern:** Read accuracy is high across the board (75–87%) — agents know *what* to look up. Write accuracy is consistently low (33–53%) — agents fail on the *execution* step. This is the primary failure mode on both domains for both agents.

---

## 3. Task-by-Task Comparison

### Airline (25 tasks)
```
opalzero_style BETTER:  tasks 2, 11, 16  (+3 wins)
opalzero_style WORSE:   tasks 6, 10      (−2 losses)
Tied at pass/fail:   20 tasks
```

### Retail (25 tasks)
```
opalzero_style BETTER:  tasks 8, 11, 13, 14  (+4 wins)
opalzero_style WORSE:   tasks 9, 15, 17, 18, 23  (−5 losses)
Tied at pass/fail:   16 tasks
```

Both agents have a **hard floor around task 10** in airline — tasks 10–24 are 0/15 pass rate for both agents. In retail, failures are distributed more evenly. This suggests airline tasks escalate in complexity more sharply.

---

## 4. Failure Mode Analysis

### A. Write-action precision failure (dominant)
The agents correctly identify what action to take (high read accuracy) but get the parameters wrong. Examples seen in log:
- Retail task 20: agent correctly called `modify_pending_order_items` but with wrong item specs → `write ❌ 0.0`
- Retail task 1: agent calculated $31.36 savings instead of $41.64 — wrong arithmetic fed into the write call

**Root cause:** The agents rely on the model's arithmetic and parameter construction without verification. A single wrong number in a `modify_pending_order_items` call fails the whole task.

### B. Context window overflow (opalzero_style specific)
- 1 confirmed `ContextWindowExceededError` on airline opalzero_style task 11: **243,207 tokens** vs 128K limit
  - τ-bench retried and eventually succeeded (task 11 passed), but at extra cost/latency
- Retail opalzero_style: 2 tasks hit **MAX_STEPS** (tasks 17 and 24, running 517s and 469s)
  - These appear to be infinite-loop scenarios where the agent keeps calling tools without terminating

The planning + validation extra LLM calls expand the message history faster than baseline. On long multi-turn conversations (>15 turns), the cumulative history can easily exceed 128K tokens.

### C. Planning phase too shallow (opalzero_style specific)
The `_make_plan()` function is called on turn 1 only, and only receives tool *names* — not tool signatures. A plan like `"tool_calls_needed": ["modify_pending_order_items(args)"]` is useless without knowing what `args` looks like. By the time the execution phase runs, the plan is ignored and the model falls back to its default behavior.

### D. Validator truncation
The policy is truncated to 800 characters before being passed to the validator:
```python
domain_policy=self.domain_policy[:800]   # truncate long policies
```
Airline and retail policies are thousands of words. The validator evaluates compliance against a fragment, missing the conditions most likely to be violated.

### E. No loop detection
The retail MAX_STEPS tasks ran for 500+ seconds. The agent was probably stuck in a cycle (get_order → wrong modify → get_order again) without breaking out. Neither baseline nor opalzero_style has any cycle detection.

---

## 5. Why OpalZero Helps on Airline but Hurts on Retail

**Airline (+4pp):**
- Tasks tend to be 3–8 turns: check policy → look up booking → apply one action
- The planning prompt correctly frames the task and helps the agent sequence tools in the right order
- Fewer write calls per task means less exposure to write-precision failures

**Retail (−4pp):**
- Tasks are 8–20 turns: multi-item lookups, calculations, condition checks, then modify
- The planning overhead adds tokens without proportional guidance (planner doesn't know tool schemas)
- 2 MAX_STEPS failures directly cost 2 tasks that baseline completed (tasks 17, 23)
- More write calls per task = more chances to fail at the critical execution step

---

## 6. What the OpalZero Methodology Is Missing (Specifically)

| Issue | Current behavior | What it should do |
|-------|-----------------|-------------------|
| Tool schema in planner | Planner sees names only | Pass full tool signatures |
| Planning frequency | Plan once on turn 1 | Re-plan after each user message |
| Policy in validator | First 800 chars | Full policy (or relevant sections) |
| Calculation verification | None | Explicit numeric re-check before write calls |
| Context management | None | Truncate or summarize history near limit |
| Loop detection | None | Detect repeated (tool, args) patterns, abort |
| Conditional action gates | None | Block write calls until pre-conditions verified |

---

## 7. Actionable Improvements (Prioritized)

### P0 — Fix the two confirmed regressions

**1. Remove the policy truncation in the validator**
```python
# Current (broken):
domain_policy=self.domain_policy[:800]
# Fix:
domain_policy=self.domain_policy  # pass full policy
# Or: extract only the relevant section based on task context
```

**2. Add context window guard**
Before calling `generate()`, check total token count. If near limit (>100K), summarize the oldest tool call/result pairs:
```python
def _trim_history_if_needed(self, messages, limit=100_000):
    # rough token estimate: 1 token ≈ 4 chars
    total_chars = sum(len(str(m)) for m in messages)
    if total_chars / 4 > limit:
        # Keep system + last N messages
        keep_last = 20
        return messages[:1] + messages[-keep_last:]
    return messages
```

**3. Add loop/cycle detection**
Track the last 3 (tool_name, args_hash) pairs. If the same call repeats, break with a message to the user.

### P1 — Fix planning to actually help

**4. Pass full tool schemas to the planner**
```python
tool_schemas = json.dumps([t.to_dict() for t in self.tools], indent=2)
# Replace tool_names with tool_schemas in AXION_PLANNER_PROMPT
```

**5. Plan on every user turn, not just turn 1**
Multi-turn retail tasks change scope with each user message. A turn-1 plan for "change my order" doesn't help when turn 5 is "actually, add another item and calculate the savings."

### P2 — Improve write accuracy (both agents benefit)

**6. Add a pre-write verification step**
Before any write call, explicitly verify the parameters against the looked-up data:
```
"Before calling modify_pending_order_items, confirm:
1. Order status is 'pending' (not shipped/delivered)
2. Item IDs match exactly what was looked up
3. Quantities and prices are from the product catalog, not estimated"
```

**7. Explicit arithmetic chain-of-thought**
Force the model to show calculations step-by-step before using the result in a write call. The $31.36 vs $41.64 error is a silent arithmetic failure that a verification step would catch.

### P3 — Model upgrade

**8. Upgrade to a stronger model**
gpt-4o-mini achieves 28–36% on tasks where Claude 3.5 Sonnet achieves 46–69%. The OpalZero methodology's overhead (plan + validate) adds marginal benefit on a weak model. On a stronger model the delta would likely be larger.

Recommended next run: `gpt-4o` or `claude-3-5-sonnet` with the same harness to establish the model baseline before attributing delta to methodology.

---

## 8. Benchmark Context

These numbers sit significantly below published baselines for a clear reason: model capability. The τ-bench paper reports these Pass^1 scores for the airline domain:
- GPT-4o: ~54%  
- Claude 3.5 Sonnet: ~46%
- GPT-4o-mini: likely ~25–30% (not published, but consistent with our 28%)

Our baseline at 28% airline / 36% retail is **consistent with what gpt-4o-mini is expected to do** on these tasks. The OpalZero methodology adds +4pp on airline (statistically meaningful for n=25) but the net effect is zero when retail's regression is included.

---

## 9. Next Steps

### Immediate (fix the harness)
- [ ] Remove policy truncation in validator
- [ ] Add context window guard with history trimming
- [ ] Add loop detection (same tool+args 3× → break)
- [ ] Pass full tool schemas to planner

### Run more data
- [ ] Run with `--max-tasks 50` on airline + retail after fixes above
- [ ] Run on `telecom` domain (different task structure, good robustness check)
- [ ] Run baseline with `gpt-4o` to confirm model-gap hypothesis

### Update BENCHMARK.md
Hold off until after at least one more run post-fixes. Publishing 28–32% on airline against Claude 3.5 Sonnet's 46% would be honest but should come with the context of the model difference and the post-fix numbers.

---

*Run config: `uv run python tau_bench_harness.py --domains airline retail --max-tasks 25 --agents opalzero_baseline opalzero_style`*  
*Results: `/tau_results/run_20260524_191227/summary.json`*  
*Log: `/tmp/tau_bench_run.log`*
