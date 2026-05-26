"""
tau_bench_harness.py
====================
Runs two agents side-by-side on τ-bench (airline + retail + telecom):

  1. baseline       – the stock LLMAgent (gpt-4o-mini, single-pass tool calling)
  2. axion_style    – same model, but with explicit upfront planning and
                      a policy-compliance validation pass before finalising.

Goal: understand whether Axion's planning + validation methodology
adds value on real multi-step service tasks, or whether it adds noise.

Run:
    cd /Users/albi/Projects/axion-lab/tau2-bench
    uv run python ../tau_bench_harness.py [--domains airline retail] [--max-tasks N]
"""

import argparse
import json
import os
import sys
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import List, Optional

# ── Ensure tau2-bench src is on the path ──────────────────────────────────────
BENCH_SRC = Path(__file__).parent / "tau2-bench" / "src"
sys.path.insert(0, str(BENCH_SRC))

# ── Load API key from .env ────────────────────────────────────────────────────
ENV_FILE = Path(__file__).parent / ".env"
if ENV_FILE.exists():
    for line in ENV_FILE.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and "=" in line:
            k, v = line.split("=", 1)
            k, v = k.strip(), v.strip()
            # Force-set: .env always wins so keys added here aren't blocked
            # by empty inherited shell variables (e.g. ANTHROPIC_API_KEY="")
            if v:
                os.environ[k] = v

MODEL_AGENT = "claude-sonnet-4-5"   # agent under test — Claude
MODEL_USER  = "gpt-4o-mini"         # user simulator — OpenAI (halves Claude RPM usage)

# ── Imports (after path setup) ────────────────────────────────────────────────
from pydantic import BaseModel

from tau2.agent.base.llm_config import LLMConfigMixin
from tau2.agent.base_agent import (
    HalfDuplexAgent,
    ValidAgentInputMessage,
    is_valid_agent_history_message,
)
from tau2.data_model.message import (
    APICompatibleMessage,
    AssistantMessage,
    Message,
    MultiToolMessage,
    SystemMessage,
    ToolMessage,
    UserMessage,
)
from tau2.data_model.simulation import TextRunConfig
from tau2.environment.tool import Tool
from tau2.registry import registry
from tau2.run import run_domain
from tau2.utils.llm_utils import generate as _generate_raw


# =============================================================================
# Rate limiter — Tier 1 Anthropic limit: 30K input tokens/min
# Assuming ~5K tokens/call on average → max 6 calls/min.
# We throttle to 5 calls/min (12 s min gap) to stay safely under the limit.
# Only applied to calls that use MODEL_AGENT (Claude); user-sim uses OpenAI.
# =============================================================================

_rate_lock = threading.Lock()
_last_claude_call: float = 0.0
_CLAUDE_MIN_GAP = 25.0  # seconds between Claude API calls (30K TPM / ~12K tokens/call = 2.5/min → 24s gap)


def generate(*args, call_name: str = "", **kwargs):
    """Thin wrapper around tau2's generate() that rate-limits Claude calls."""
    global _last_claude_call
    model = kwargs.get("model") or (args[0] if args else "")
    is_claude = isinstance(model, str) and "claude" in model.lower()
    if is_claude:
        with _rate_lock:
            elapsed = time.time() - _last_claude_call
            if elapsed < _CLAUDE_MIN_GAP:
                time.sleep(_CLAUDE_MIN_GAP - elapsed)
            _last_claude_call = time.time()
    return _generate_raw(*args, call_name=call_name, **kwargs)


# =============================================================================
# AGENT 1: Baseline (mirrors built-in LLMAgent exactly)
# =============================================================================

BASELINE_SYSTEM = """\
You are a customer service agent that helps the user according to the <policy> provided below.
In each turn you can either:
- Send a message to the user.
- Make a tool call.
You cannot do both at the same time.

Try to be helpful and always follow the policy.
<policy>
{domain_policy}
</policy>""".strip()


class BaselineState(BaseModel):
    system_messages: list[SystemMessage]
    messages: list[APICompatibleMessage]


class BaselineAgent(LLMConfigMixin, HalfDuplexAgent[BaselineState]):
    """Bare single-pass tool-calling agent — the control condition."""

    def __init__(self, tools: List[Tool], domain_policy: str, llm: str,
                 llm_args: Optional[dict] = None):
        super().__init__(tools=tools, domain_policy=domain_policy,
                         llm=llm, llm_args=llm_args)

    def get_init_state(self, message_history: Optional[list[Message]] = None) -> BaselineState:
        if message_history is None:
            message_history = []
        assert all(is_valid_agent_history_message(m) for m in message_history)
        system = BASELINE_SYSTEM.format(domain_policy=self.domain_policy)
        return BaselineState(
            system_messages=[SystemMessage(role="system", content=system)],
            messages=list(message_history),
        )

    def generate_next_message(self, message: ValidAgentInputMessage,
                              state: BaselineState) -> tuple[AssistantMessage, BaselineState]:
        if isinstance(message, MultiToolMessage):
            state.messages.extend(message.tool_messages)
        else:
            state.messages.append(message)
        reply = generate(
            model=self.llm,
            tools=self.tools,
            messages=state.system_messages + state.messages,
            call_name="baseline_response",
            **(self.llm_args or {}),
        )
        state.messages.append(reply)
        return reply, state


def create_baseline_agent(tools, domain_policy, **kwargs):
    return BaselineAgent(tools=tools, domain_policy=domain_policy,
                         llm=kwargs.get("llm", MODEL_AGENT),
                         llm_args=kwargs.get("llm_args"))


# =============================================================================
# AGENT 2: Axion-style (plan → execute → validate)
# =============================================================================

AXION_SYSTEM = """\
You are a customer service agent that helps the user according to the <policy> provided below.

You follow a structured three-phase approach:

PHASE 1 — PLAN (internal only, never shown to user):
When you receive a request, think through: what information do I need? what tools should I call and in what order? what constraints does the policy impose?

PHASE 2 — EXECUTE:
Call tools in the planned order. Gather all information needed before responding.

PHASE 3 — VALIDATE before responding:
Check: Does my planned response comply with the policy? Have I missed any required tool calls? If not, make additional tool calls first.

In each turn you can either:
- Send a message to the user.
- Make a tool call.
You cannot do both at the same time.

Always follow the policy. Prioritise correctness over speed.

<policy>
{domain_policy}
</policy>""".strip()

AXION_PLANNER_PROMPT = """\
Before responding to the user's request, create an explicit plan.

User request: {user_message}

Available tools (with full signatures):
{tool_schemas}

Respond with a JSON object:
{{
  "understanding": "what the user needs",
  "policy_constraints": ["constraint 1", "constraint 2"],
  "tool_calls_needed": ["tool_a with args {{arg1: value1}}", "tool_b with args {{arg2: value2}}"],
  "write_preconditions": ["check X before calling write tool Y"],
  "validation_check": "what to verify before finalising"
}}""".strip()

AXION_VALIDATOR_PROMPT = """\
You have taken the following actions to resolve the customer's request.
Policy: {domain_policy}

Actions taken so far (tool calls + results):
{actions_summary}

Planned next response to user: {planned_response}

Check: Does this response fully comply with the policy? Are there any additional tool calls required before responding?
Reply with JSON: {{"compliant": true/false, "missing_actions": [], "corrected_response": "..."}}""".strip()


class AxionState(BaseModel):
    system_messages: list[SystemMessage]
    messages: list[APICompatibleMessage]
    plan: Optional[dict] = None          # populated on first user turn
    turn_count: int = 0
    recent_tool_calls: list[str] = []   # for loop detection (last N call signatures)


class AxionStyleAgent(LLMConfigMixin, HalfDuplexAgent[AxionState]):
    """
    Axion-inspired agent: explicit planning + policy validation gate.
    Same model as baseline — tests whether the methodology helps.
    """

    def __init__(self, tools: List[Tool], domain_policy: str, llm: str,
                 llm_args: Optional[dict] = None):
        super().__init__(tools=tools, domain_policy=domain_policy,
                         llm=llm, llm_args=llm_args)

    def get_init_state(self, message_history: Optional[list[Message]] = None) -> AxionState:
        if message_history is None:
            message_history = []
        assert all(is_valid_agent_history_message(m) for m in message_history)
        system = AXION_SYSTEM.format(domain_policy=self.domain_policy)
        return AxionState(
            system_messages=[SystemMessage(role="system", content=system)],
            messages=list(message_history),
        )

    def _make_plan(self, user_text: str, state: AxionState) -> dict:
        """Call the LLM to produce a structured plan for this request."""
        # Build compact tool schema: name + description + parameter names
        tool_schema_lines = []
        for t in self.tools:
            params = ""
            try:
                props = t.parameters.get("properties", {}) if hasattr(t, "parameters") else {}
                required = t.parameters.get("required", []) if hasattr(t, "parameters") else []
                param_parts = [
                    f"{k}{'*' if k in required else ''}: {v.get('type', '?')}"
                    for k, v in props.items()
                ]
                params = ", ".join(param_parts)
            except Exception:
                pass
            desc = getattr(t, "description", "") or ""
            tool_schema_lines.append(f"  {t.name}({params}) — {desc[:120]}")
        tool_schemas = "\n".join(tool_schema_lines)

        planner_msg = AXION_PLANNER_PROMPT.format(
            user_message=user_text,
            tool_schemas=tool_schemas,
        )
        plan_messages = state.system_messages + [
            UserMessage.text(content=planner_msg)
        ]
        try:
            plan_resp = generate(
                model=self.llm,
                tools=[],               # no tools during planning phase
                messages=plan_messages,
                call_name="axion_plan",
                **(self.llm_args or {}),
            )
            text = plan_resp.content or ""
            # Extract JSON from the response
            start = text.find("{")
            end = text.rfind("}") + 1
            if start >= 0 and end > start:
                return json.loads(text[start:end])
        except Exception:
            pass
        return {}

    # ── Context window guard ──────────────────────────────────────────────────
    _CONTEXT_TOKEN_LIMIT = 100_000   # conservative limit (128K is the hard cap)
    _CHARS_PER_TOKEN = 4             # rough estimate

    def _trim_history_if_needed(self, messages: list) -> list:
        """Keep message history under the context limit by dropping old tool messages."""
        total_chars = sum(len(str(m)) for m in messages)
        if total_chars / self._CHARS_PER_TOKEN <= self._CONTEXT_TOKEN_LIMIT:
            return messages
        # Drop oldest ToolMessages first (they're most verbose), then old turns
        trimmed = []
        tool_budget = int(total_chars * 0.4)   # drop ~40% from tool results
        dropped = 0
        for m in messages:
            if isinstance(m, ToolMessage) and dropped < tool_budget:
                dropped += len(str(m))
            else:
                trimmed.append(m)
        return trimmed

    def _extract_actions_summary(self, state: AxionState) -> str:
        """Summarise tool calls + results from message history."""
        lines = []
        msgs = state.messages
        for i, msg in enumerate(msgs):
            if isinstance(msg, AssistantMessage) and msg.is_tool_call():
                for tc in (msg.tool_calls or []):
                    lines.append(f"  Called {tc.name}({json.dumps(tc.arguments)})")
            elif isinstance(msg, ToolMessage):
                lines.append(f"  → Result: {str(msg.content)[:200]}")
        return "\n".join(lines) if lines else "(no tool calls yet)"

    def _is_looping(self, reply: AssistantMessage, state: AxionState) -> bool:
        """Return True if the agent is repeating the same tool call 3 times."""
        if not reply.is_tool_call():
            return False
        sig = "|".join(
            f"{tc.name}:{json.dumps(tc.arguments, sort_keys=True)}"
            for tc in (reply.tool_calls or [])
        )
        state.recent_tool_calls.append(sig)
        state.recent_tool_calls = state.recent_tool_calls[-9:]  # keep last 9
        # loop = same sig appears 3 times in the last 9 calls
        return state.recent_tool_calls.count(sig) >= 3

    def generate_next_message(self, message: ValidAgentInputMessage,
                              state: AxionState) -> tuple[AssistantMessage, AxionState]:
        state.turn_count += 1

        # Append incoming message(s) to history
        if isinstance(message, MultiToolMessage):
            state.messages.extend(message.tool_messages)
        else:
            state.messages.append(message)

        # ── Phase 1: Plan on first substantive user turn ──────────────────────
        if (state.turn_count == 1 and isinstance(message, UserMessage)
                and message.content):
            state.plan = self._make_plan(message.content, state)

        # ── Phase 2: Execute — call LLM with tools ────────────────────────────
        trimmed_messages = self._trim_history_if_needed(state.messages)
        reply = generate(
            model=self.llm,
            tools=self.tools,
            messages=state.system_messages + trimmed_messages,
            call_name="axion_execute",
            **(self.llm_args or {}),
        )

        # ── Loop guard ────────────────────────────────────────────────────────
        if self._is_looping(reply, state):
            reply = AssistantMessage.text(
                content="I seem to be stuck in a loop. Let me summarise what I've done so far and ask you how to proceed."
            )

        # ── Phase 3: Validate before text reply (skip for tool calls) ─────────
        # We only validate when the agent is about to give a text response
        # to the user (not when it's making a tool call), and only after
        # at least one tool has been called (otherwise there's nothing to validate).
        has_prior_tool_calls = any(
            isinstance(m, ToolMessage) for m in state.messages
        )
        if (not reply.is_tool_call() and has_prior_tool_calls
                and reply.content):
            actions_summary = self._extract_actions_summary(state)
            validator_prompt = AXION_VALIDATOR_PROMPT.format(
                domain_policy=self.domain_policy,       # full policy, no truncation
                actions_summary=actions_summary,
                planned_response=reply.content[:500],
            )
            try:
                val_messages = state.system_messages + [
                    UserMessage.text(content=validator_prompt)
                ]
                val_resp = generate(
                    model=self.llm,
                    tools=[],
                    messages=val_messages,
                    call_name="axion_validate",
                    **(self.llm_args or {}),
                )
                val_text = val_resp.content or ""
                start = val_text.find("{")
                end = val_text.rfind("}") + 1
                if start >= 0 and end > start:
                    val = json.loads(val_text[start:end])
                    if not val.get("compliant", True):
                        corrected = val.get("corrected_response", "")
                        if corrected:
                            reply = AssistantMessage.text(content=corrected)
            except Exception:
                pass  # validation failure → keep original reply

        state.messages.append(reply)
        return reply, state


def create_axion_agent(tools, domain_policy, **kwargs):
    return AxionStyleAgent(tools=tools, domain_policy=domain_policy,
                           llm=kwargs.get("llm", MODEL_AGENT),
                           llm_args=kwargs.get("llm_args"))


# =============================================================================
# Register agents
# =============================================================================

registry.register_agent_factory(create_baseline_agent, "axion_baseline")
registry.register_agent_factory(create_axion_agent, "axion_style")


# =============================================================================
# Harness runner
# =============================================================================

def run_agent_on_domain(agent_name: str, domain: str, max_tasks: Optional[int],
                        output_dir: Path) -> dict:
    """Run one agent on one domain, save results, return summary."""
    print(f"\n{'='*60}")
    print(f"  Agent: {agent_name}  |  Domain: {domain}")
    print(f"{'='*60}")

    config_kwargs = dict(
        domain=domain,
        agent=agent_name,
        llm_agent=MODEL_AGENT,
        llm_user=MODEL_USER,
        user="user_simulator",
        num_trials=1,
        output_dir=str(output_dir / agent_name / domain),
        save_results=True,
    )
    if max_tasks is not None:
        config_kwargs["num_tasks"] = max_tasks
    config_kwargs["max_concurrency"] = 1   # serialise to respect Tier-1 TPM limits

    config = TextRunConfig(**config_kwargs)

    results = run_domain(config)

    # Compute pass rates and extract per-task detail
    rewards, task_details = [], []
    for sim in results.simulations:
        if sim.reward_info is None:
            continue
        r = sim.reward_info.reward
        rewards.append(r)

        # Extract conversation transcript
        transcript = []
        for msg in (sim.messages or []):
            role = getattr(msg, "role", "unknown")
            content = getattr(msg, "content", "")
            if content and isinstance(content, str):
                transcript.append({"role": role, "content": content[:800]})
            # tool calls
            tool_calls = getattr(msg, "tool_calls", None)
            if tool_calls:
                for tc in tool_calls:
                    transcript.append({
                        "role": "tool_call",
                        "name": getattr(tc, "name", "?"),
                        "args": getattr(tc, "arguments", {}),
                    })

        task_details.append({
            "task_id": getattr(sim, "task_id", None),
            "reward": r,
            "passed": r >= 1.0,
            "reward_info": str(sim.reward_info),
            "termination": str(getattr(sim, "termination_reason", "")),
            "cost_agent": getattr(sim, "agent_cost", None),
            "transcript": transcript,
        })

    total = len(rewards)
    passed = sum(1 for r in rewards if r >= 1.0)
    avg_reward = sum(rewards) / total if total else 0.0

    # Save per-task detail for failure analysis
    detail_path = output_dir / agent_name / domain / "task_details.json"
    detail_path.parent.mkdir(parents=True, exist_ok=True)
    with open(detail_path, "w") as f:
        json.dump(task_details, f, indent=2, default=str)

    summary = {
        "agent": agent_name,
        "domain": domain,
        "model": MODEL_AGENT,
        "total_tasks": total,
        "passed": passed,
        "pass_rate": passed / total if total else 0.0,
        "avg_reward": avg_reward,
        "rewards": rewards,
    }

    print(f"\n  Results: {passed}/{total} passed  ({100*summary['pass_rate']:.1f}%)  "
          f"avg_reward={avg_reward:.3f}")
    return summary


def print_comparison_table(summaries: list[dict]):
    """Print a markdown comparison table."""
    print("\n\n" + "="*70)
    print("COMPARISON TABLE")
    print("="*70)
    print(f"{'Agent':<20} {'Domain':<12} {'Pass^1':>8} {'Avg Reward':>12}")
    print("-"*56)
    for s in sorted(summaries, key=lambda x: (x["domain"], x["agent"])):
        pct = f"{100*s['pass_rate']:.1f}%"
        print(f"{s['agent']:<20} {s['domain']:<12} {pct:>8} {s['avg_reward']:>12.3f}")
    print("="*70)

    # Delta: axion_style vs axion_baseline per domain
    baseline = {s["domain"]: s for s in summaries if s["agent"] == "axion_baseline"}
    axion = {s["domain"]: s for s in summaries if s["agent"] == "axion_style"}
    print("\nDELTA (axion_style vs baseline):")
    for domain in sorted(set(baseline) | set(axion)):
        if domain in baseline and domain in axion:
            delta_pass = axion[domain]["pass_rate"] - baseline[domain]["pass_rate"]
            delta_rew = axion[domain]["avg_reward"] - baseline[domain]["avg_reward"]
            sign = "+" if delta_pass >= 0 else ""
            print(f"  {domain:<12}  pass_rate {sign}{100*delta_pass:.1f}pp  "
                  f"avg_reward {'+' if delta_rew >= 0 else ''}{delta_rew:.3f}")


def main():
    parser = argparse.ArgumentParser(description="τ-bench: Axion vs Baseline")
    parser.add_argument("--domains", nargs="+",
                        default=["airline", "retail"],
                        choices=["airline", "retail", "telecom", "mock"],
                        help="Domains to evaluate")
    parser.add_argument("--max-tasks", type=int, default=None, dest="max_tasks",
                        help="Max tasks per domain per agent (None = all)")
    parser.add_argument("--agents", nargs="+",
                        default=["axion_baseline", "axion_style"],
                        choices=["axion_baseline", "axion_style", "llm_agent"],
                        help="Agents to run")
    args = parser.parse_args()

    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    output_dir = Path(__file__).parent / "tau_results" / f"run_{timestamp}"
    output_dir.mkdir(parents=True, exist_ok=True)

    print(f"\nτ-bench harness  |  agent={MODEL_AGENT}  user={MODEL_USER}  |  run={timestamp}")
    print(f"Domains: {args.domains}")
    print(f"Agents:  {args.agents}")
    print(f"Max tasks per combo: {args.max_tasks or 'all'}")
    print(f"Output: {output_dir}")

    summaries = []
    for domain in args.domains:
        for agent_name in args.agents:
            try:
                summary = run_agent_on_domain(agent_name, domain,
                                              args.max_tasks, output_dir)
                summaries.append(summary)
            except Exception as e:
                print(f"\n  ERROR: {agent_name} on {domain}: {e}")
                import traceback; traceback.print_exc()

    # Save summary
    summary_path = output_dir / "summary.json"
    with open(summary_path, "w") as f:
        json.dump(summaries, f, indent=2)

    print_comparison_table(summaries)
    print(f"\nFull results saved to: {output_dir}")
    print(f"Summary: {summary_path}")


if __name__ == "__main__":
    main()
