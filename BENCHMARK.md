# Axion Benchmark Report

> Version 3.0 · May 2026 · Feature Capability Analysis

---

## Abstract

This report evaluates the Axion multi-agent kernel against eight competing frameworks across ten engineering dimensions relevant to production orchestration.

**Methodology.** Scores are assigned by the Axion engineering team based on documented feature analysis of each framework — documentation, source code, and release notes as of May 2026. These are *not* empirical task scores. They measure what each framework's architecture makes possible, not what it achieves on any specific benchmark. Self-evaluation bias is a real risk; scores are intended to be conservative and reasoning is documented.

**What this covers.**
- Feature capability scores across ten dimensions (schema enforcement, DAG, quality gates, provider flexibility, self-hosting, tools, streaming, performance, DX, production readiness)
- Capability ceiling analysis for five representative production task types
- Framework profiles with documented strengths and known limitations
- Axion architecture reference

**What this does not cover.** Empirical run results on any third-party benchmark. We are actively investigating appropriate benchmarks for multi-agent structured-output pipelines and will publish empirical results when a suitable, reproducible evaluation exists. We will not publish benchmark numbers that don't match what Axion is actually built to do.

---

## Part 1: Feature Capability Analysis

> **Label:** All scores in this section are analytical — assigned from documentation and source code review, not from task runs.

### 1.1 Evaluation Dimensions

Ten dimensions scored 1–5 for each framework:

| Dimension | Score 1 | Score 5 |
|-----------|---------|---------|
| Output reliability / schema enforcement | No schema enforcement | Kernel-level enforcement with automatic retry |
| DAG orchestration / parallel execution | Linear chains only | Native DAG with dependency resolution and cascade-fail |
| Quality gates / semantic review | No quality concept | Multi-criterion semantic review with fix-task generation |
| Provider flexibility | Single provider | OpenAI, Anthropic, Ollama local, any OpenAI-compatible endpoint |
| Self-hosting / air-gap | Mandatory cloud egress | Single binary, no cloud egress, zero telemetry |
| Tool ecosystem breadth | No built-in tools | 300+ ready-made connectors |
| Real-time streaming | No streaming | Per-task SSE streaming with client SDK |
| Runtime performance | Python process | Compiled Rust binary, async runtime |
| Developer experience | Steep learning curve | Minimal boilerplate, visual tooling, one-command setup |
| Production readiness | Experimental | Stable API, enterprise auth, RBAC, SLA-backed support |

### 1.2 Dimension Scores

| Dimension | Axion | LangGraph | Haystack | PydanticAI | LlamaIndex | LangChain | CrewAI | AutoGen | smolagents |
|-----------|:-----:|:---------:|:--------:|:----------:|:----------:|:---------:|:------:|:-------:|:----------:|
| Output reliability / schema enforcement | **5** | 3 | 3 | **5** | 3 | 3 | 2 | 2 | 1 |
| DAG orchestration / parallel execution | **5** | **5** | 3 | 2 | 4 | 2 | 3 | 3 | 2 |
| Quality gates / semantic review | **5** | 2 | 2 | 2 | 1 | 1 | 1 | 1 | 1 |
| Provider flexibility | **5** | 4 | 4 | 4 | 4 | 4 | 4 | 4 | **5** |
| Self-hosting / air-gap | **5** | 3 | 4 | 4 | 3 | 3 | 3 | 3 | **5** |
| Tool ecosystem breadth | 2 | **5** | **5** | 3 | **5** | **5** | 4 | 4 | 3 |
| Real-time streaming | **5** | 4 | 3 | 3 | 3 | 3 | 2 | 2 | 1 |
| Runtime performance | **5** | 2 | 2 | 2 | 2 | 2 | 2 | 2 | 2 |
| Developer experience | 3 | 3 | 3 | 4 | 3 | 4 | **5** | 3 | **5** |
| Production readiness | 4 | 4 | 4 | 3 | 3 | 3 | 3 | 3 | 2 |
| **Total (50 max)** | **44** | **35** | **33** | **32** | **31** | **30** | **29** | **27** | **27** |

**Key findings:**

- Axion is the only framework evaluated that scores 5/5 on schema enforcement, DAG orchestration, quality governance, runtime performance, and streaming simultaneously.
- PydanticAI is the only other framework matching Axion on schema enforcement.
- LangGraph is the only other framework matching Axion on DAG orchestration.
- Tool ecosystem breadth (18 vs 300+) is Axion's largest documented gap.

### 1.3 Capability Ceiling Analysis (Analytical)

The following table shows the maximum achievable score for each framework on five representative production tasks, based on whether the framework's architecture makes success structurally possible.

**These are capability ceiling scores, not empirical pass rates.** Actual pass rates from live runs will be lower for all frameworks on all tasks.

| Framework | T1 Financial Research | T2 Pricing Analysis | T3 Document Intel | T4 API Integration | T5 Data Pipeline | Avg |
|-----------|:---:|:---:|:---:|:---:|:---:|:---:|
| **Axion** | 5 | 5 | 5 | 5 | 5 | **5.0** |
| LangGraph | 4 | 4 | 4 | 4 | 3 | **3.8** |
| LangChain | 4 | 4 | 4 | 4 | 3 | **3.8** |
| LlamaIndex | 4 | 3 | 5 | 3 | 4 | **3.8** |
| Haystack | 3 | 3 | 4 | 4 | 4 | **3.6** |
| PydanticAI | 4 | 4 | 3 | 4 | 3 | **3.6** |
| CrewAI | 3 | 3 | 3 | 4 | 3 | **3.2** |
| AutoGen | 3 | 3 | 3 | 4 | 3 | **3.2** |
| smolagents | 2 | 2 | 2 | 3 | 3 | **2.4** |

---

### 1.4 Task Definitions

**T1 — Structured Financial Research**

*Intent:* "Provide a current investment brief for Apple Inc. including stock price, market cap, P/E ratio, top 3 risks, and analyst consensus."

*Required schema:*
```json
{
  "current_price_usd": "number",
  "market_cap_usd": "number",
  "pe_ratio": "number",
  "top_risks": ["string"],
  "analyst_consensus": "string"
}
```

*Why Axion scores 5:* Schema is enforced at kernel level. The Governor rejects outputs where fields are null, vague, or missing citations. The WebSearcher fetches live data; the Analyst synthesises across sources with schema contract binding.

---

**T2 — Competitive Pricing Analysis**

*Intent:* "Compare the pricing plans of Linear, Notion, and Asana. Return a structured comparison."

*Required schema:* nested products → plans → price + features array.

*Why Axion scores 5:* Multi-product research is parallelised across WebSearcher tasks (one per product), merged by the Analyst, and validated by the Governor against the schema. Any missing product triggers a targeted fix task.

---

**T3 — Document Intelligence**

*Intent:* "Extract all clauses related to termination, liability, and governing law from the uploaded contract PDF."

*Why Axion scores 5:* `extract_pdf_text` tool ships built-in. Schema enforcement ensures all three clause categories are non-empty. Governor checks claim specificity — paraphrase without source reference is rejected.

*Why LlamaIndex also scores 5:* LlamaParse v2 provides superior PDF OCR for complex layouts.

---

**T4 — API Integration**

*Intent:* "Fetch the latest 5 Hacker News posts and summarise trending topics."

*Why Axion scores 5:* `http_request` tool fetches the HN API directly. Schema enforces exactly 5 posts with required fields. Governor validates that `trending_topics` is derived from actual post content, not hallucinated.

---

**T5 — Multi-Step Data Pipeline**

*Intent:* "Given the uploaded sales CSV, calculate total revenue by region and identify the top-performing product."

*Why Axion scores 5:* `read_csv` + `sqlite_query` + `python_interpreter` tools compose natively in a DAG. Each computation step is a separate task with declared dependencies. Governor validates arithmetic correctness by re-checking totals.

---

## Part 2: Framework Profiles

### LangChain
**Version:** 1.x · **GitHub:** ~126K stars · **Language:** Python

The most widely deployed LLM framework. 300+ integrations, stable 1.0 API after years of breaking changes. Schema enforcement via `with_structured_output` + Pydantic is application-layer, not kernel-level. Linear chains by default — DAG requires migrating to LangGraph. No semantic quality governor. Silent 60-second timeout is a known production hazard.

**Best fit:** Broadest integration ecosystem; teams with existing LangChain investment.

---

### LangGraph
**Version:** 1.0 (Oct 2025) · **GitHub:** ~24K stars · **Language:** Python

Strongest Python option for DAG orchestration. Durable execution, checkpoints, LangGraph Studio visual debugger. Requires LangSmith cloud egress for observability — disqualifying for air-gap deployments. No semantic quality governor. p95 latency ~4.2s for a 3-step gpt-4o-mini chain (madappgang.com, 2026).

**Best fit:** Teams already on LangChain needing DAG; LangSmith egress acceptable.

---

### CrewAI
**Version:** Studio v2 (May 2025) · **GitHub:** ~44–47K stars · **Language:** Python

Role-based multi-agent metaphor. 60%+ Fortune 500 adoption (self-reported). **Default configuration sends agent prompts and task descriptions to CrewAI cloud telemetry.** Opt-out exists but is not prominently documented. Memory footprint exceeds 2 GB for crews with 10+ agents (SJSU CS, 2025). Token overhead ~3× for simple tasks.

**Best fit:** Rapid prototyping; non-sensitive data; Fortune 500 pilots with enterprise tier.

---

### AutoGen / AG2
**Version:** v0.4+ async; MAF v1.0 GA (Apr 2026) · **GitHub:** ~54K stars · **Language:** Python

GroupChat conversation model produces emergent behaviour at the cost of determinism. Original repo moved to maintenance mode in early 2026; active development split between AG2 fork and Microsoft Agent Framework. Deep Azure integration.

**Best fit:** Azure infrastructure; open-ended research tasks; non-determinism acceptable.

---

### LlamaIndex
**Version:** 0.12+ (Q1 2026) · **GitHub:** ~47K stars · **Language:** Python

Dominates RAG and document retrieval. LlamaParse v2 is the best PDF extraction tool evaluated. Workflows 1.0 adds event-driven parallel agents and MCP connectivity. General-purpose orchestration remains secondary.

**Best fit:** RAG pipelines; document intelligence; knowledge-base Q&A.

---

### Haystack (deepset)
**Version:** 2.28+ · **GitHub:** ~23K stars · **Language:** Python

Rigorous pipeline architecture (directed multigraph of components). Human-in-the-loop tool approval built in — notable differentiator. First-class RAG and hybrid retrieval. Smaller community than LangChain/CrewAI.

**Best fit:** Enterprise search; hybrid retrieval; structured pipeline model.

---

### PydanticAI
**Version:** v1 (Sep 2025; latest May 2026) · **GitHub:** ~15–17K stars · **Language:** Python

Best-in-class type safety. `Agent[ResultType]` enforces Pydantic validation with mypy/pyright type-checking at dev time. Structured output streaming. Explicitly single-agent — no DAG planner, no quality governor.

**Best fit:** Type-conscious Python teams; single-agent apps where output correctness is critical.

---

### smolagents (HuggingFace)
**Version:** 1.24.0 · **GitHub:** ~26K stars · **Language:** Python

Code-action model — agents write Python rather than JSON tool calls. Most expressive action model evaluated. Best sandboxing (E2B, Modal, Docker, Pyodide). **Documented bug:** `final_answer` called mid-script silently discards downstream code. No DAG, no schema enforcement, no streaming.

**Best fit:** Research prototyping; sandboxed execution; expressiveness over reliability.

---

## Part 3: Axion Architecture

### Core model

```
User intent
    │
    ▼
 Planner ──── builds a dependency-ordered task graph (DAG)
    │
    ├──► WebSearcher   ──┐
    ├──► WebSearcher   ──┼──► ContextBus (shared memory)
    └──► Analyst       ──┘
                          │
                          ▼
                      Governor (5-criterion semantic review)
                          │
              ┌───────────┴────────────┐
              ▼                        ▼
       MissionComplete         REVISE → targeted fix task → loop
```

### Governor criteria

| Criterion | What it checks |
|-----------|----------------|
| Intent coverage | Does the output address what was actually asked? |
| Data density | Is it substantively populated, or padded with filler? |
| Structural validity | Does it conform to the declared schema? |
| Claim specificity | Are claims concrete and verifiable? |
| Synthesis completeness | Has the agent integrated all relevant inputs? |

### Built-in tools (18)

| Category | Tools |
|----------|-------|
| Web | `web_search` · `fetch_page` · `http_request` · `rss_reader` |
| Data | `read_csv` · `sqlite_query` · `diff` · `calculator` |
| Document | `extract_pdf_text` |
| Compute | `python_interpreter` |
| Communication | `send_email` (SMTP, any provider) |
| Vision | `vision` |
| Finance | Alpha Vantage integration |
| Control flow | `finalize_mission_state` · `feedback` (HITL gate) |
| Persistence | `memory_persist` · `write_file` · `read_file` |

### Where Axion leads

- **Output reliability in production.** Kernel-level schema enforcement + Governor semantic review + automatic fix-task generation. For workflows where a wrong answer is costly (financial data, legal documents, customer-facing content), this is the dominant selection criterion.
- **Air-gap deployment.** Single binary + Ollama = zero data egress. No equivalent in the evaluated Python frameworks without significant application-layer engineering.
- **DAG orchestration with quality gates.** LangGraph matches Axion on DAG capability. No other framework combines both.
- **Runtime performance.** Rust async runtime: ~36% higher throughput, ~44% lower p95 latency vs Python-based frameworks at equivalent concurrency (dev.to/saivishwak, 2026).

### Where Axion lags

- **Tool ecosystem.** 18 built-in tools vs 300+ for LangChain, LangGraph, LlamaIndex.
- **Developer experience.** Rust learning curve. No visual mission debugger (LangGraph Studio equivalent not yet built).
- **Community.** Early-stage. Stack Overflow answers, tutorials, and third-party integrations are sparse.

---

## Part 4: Performance Benchmarks

Published data from third-party research (cited):

| Metric | Axion vs Python frameworks |
|--------|---------------------------|
| Throughput under concurrent load | ~36% higher (Rust async vs CPython) |
| p95 latency under concurrent load | ~44% lower |
| Memory at scale | ~5× more efficient |
| LangGraph p95 latency (3-step, gpt-4o-mini) | 4.2s baseline |
| CrewAI memory (10+ agents, 50+ tasks) | >2 GB (SJSU CS, 2025) |

Sources: Saiva Vishwak (dev.to, 2026), madappgang.com (2026), SJSU CS Department (2025), CrewAI Engineering Blog (2025).

---

## References

1. Saiva Vishwak, "Rust Agent Frameworks vs Python," dev.to, 2026.
2. madappgang.com, "AI Framework Comparison Guide 2026."
3. SJSU Computer Science Department, "Memory and Token Overhead in Multi-Agent Frameworks," 2025.
4. CrewAI Engineering Blog, "CrewAI vs LangGraph Performance Benchmark," 2025.
5. LangGraph 1.0 Release Notes, October 2025.
6. PydanticAI v1 Release Announcement, September 2025.
7. HuggingFace, "smolagents Bug Tracker: final_answer mid-script issue," 2025.
8. Microsoft Agent Framework v1.0 GA, April 2026.
9. LlamaIndex, "Workflows 1.0 and LlamaParse v2 Release," 2025–2026.
