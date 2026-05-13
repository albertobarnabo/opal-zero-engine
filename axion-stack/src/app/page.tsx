"use client";

import { useState, useEffect, useCallback } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Renderer, UIBlueprint, UIComponent } from "@/components/Renderer";

// ── Types ─────────────────────────────────────────────────────────────────────

interface StreamCard {
  slug: string;
  role: string;
  intent: string;
  status: "running" | "completed" | "failed";
  result?: string;
}

interface MissionMeta {
  task_count: number;
  expanded_task_count: number;
  mission_id: string;
  intent: string;
  layout_hint?: string;
}

interface MissionSummary {
  id: string;
  timestamp: number;
  intent: string;
  task_count: number;
  status: string;
  layout_hint?: string;
}

type MissionStatus = "idle" | "streaming" | "complete" | "failed";

// ── MissionState & DesignTokens (the backend's canonical output) ─────────────

interface DesignTokens {
  primary_accent: string;
  glass_intensity: number;
  theme_preset: string;
  layout_density: "spacious" | "compact";
}

interface MissionState {
  intent_resolved: boolean;
  data_payload: Record<string, unknown>;
  verification_logs: string[];
  design_tokens: DesignTokens;
}

// ── Theme engine ──────────────────────────────────────────────────────────────
// Converts DesignTokens into CSS custom properties on <html> so every
// component picks them up without prop-drilling.

function hexToRgb(hex: string): [number, number, number] {
  const h = hex.replace("#", "").padEnd(6, "0");
  return [
    parseInt(h.slice(0, 2), 16) || 99,
    parseInt(h.slice(2, 4), 16) || 102,
    parseInt(h.slice(4, 6), 16) || 241,
  ];
}

function applyDesignTokens(tokens: DesignTokens) {
  const root = document.documentElement;
  const [r, g, b] = hexToRgb(tokens.primary_accent);
  const gi = Math.max(0, Math.min(1, tokens.glass_intensity));
  const blur = Math.round(4 + gi * 28);          // 4–32 px
  const bgAlpha = (0.04 + gi * 0.18).toFixed(3); // 0.04–0.22
  const borderAlpha = (0.2 + gi * 0.3).toFixed(3); // 0.2–0.5
  const compact = tokens.layout_density === "compact";

  root.style.setProperty("--axion-accent",       tokens.primary_accent);
  root.style.setProperty("--axion-blur",         `${blur}px`);
  root.style.setProperty("--axion-glass-bg",     `rgba(${r},${g},${b},${bgAlpha})`);
  root.style.setProperty("--axion-glass-border", `rgba(${r},${g},${b},${borderAlpha})`);
  root.style.setProperty("--axion-glow",         `rgba(${r},${g},${b},0.22)`);
  root.style.setProperty("--axion-pad",          compact ? "12px" : "20px");
  root.style.setProperty("--axion-gap",          compact ? "8px"  : "12px");
}

function resetDesignTokens() {
  const props = [
    "--axion-accent", "--axion-blur", "--axion-glass-bg",
    "--axion-glass-border", "--axion-glow", "--axion-pad", "--axion-gap",
  ];
  props.forEach((p) => document.documentElement.style.removeProperty(p));
}

// ── ApplicationMapper ─────────────────────────────────────────────────────────
// Converts a raw `data_payload` JSON object into a `UIBlueprint` that the
// existing `Renderer` can display.  The Brain provides facts; we decide the UI.

function formatKey(key: string): string {
  return key
    .replace(/_/g, " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

function applicationMapper(payload: Record<string, unknown>): UIBlueprint {
  const components: UIComponent[] = [];

  for (const [key, value] of Object.entries(payload)) {
    if (value === null || value === undefined) continue;
    const label = formatKey(key);
    const lk = key.toLowerCase();

    // ── Arrays ──────────────────────────────────────────────────────────────
    if (Array.isArray(value)) {
      if (value.length === 0) continue;
      const first = value[0];

      // Array of step-like objects (have a "label" key) → Timeline
      if (
        typeof first === "object" &&
        first !== null &&
        !Array.isArray(first) &&
        "label" in (first as object)
      ) {
        components.push({ component_type: "Timeline", props: { title: label, steps: value } });
        continue;
      }

      // Array of plain objects → ComparisonTable
      if (typeof first === "object" && first !== null && !Array.isArray(first)) {
        const headers = Object.keys(first as object);
        const rows = value.map((item) =>
          headers.map((h) => {
            const v = (item as Record<string, unknown>)[h];
            return v != null ? String(v) : "";
          })
        );
        components.push({
          component_type: "ComparisonTable",
          props: { title: label, headers: headers.map(formatKey), rows },
        });
        continue;
      }

      // Array of primitives → single-column ComparisonTable
      components.push({
        component_type: "ComparisonTable",
        props: { title: label, headers: [label], rows: value.map((v) => [String(v)]) },
      });
      continue;
    }

    // ── Objects ───────────────────────────────────────────────────────────────
    if (typeof value === "object" && value !== null) {
      const obj = value as Record<string, unknown>;

      if ("title" in obj && "value" in obj) {
        components.push({ component_type: "MetricCard", props: obj });
        continue;
      }
      if ("label" in obj && "status" in obj) {
        components.push({ component_type: "StatusBadge", props: obj });
        continue;
      }

      const rows = Object.entries(obj).map(([k, v]) => [formatKey(k), String(v ?? "")]);
      if (rows.length > 0) {
        components.push({
          component_type: "ComparisonTable",
          props: { title: label, headers: ["Field", "Value"], rows },
        });
      }
      continue;
    }

    // ── Scalars ───────────────────────────────────────────────────────────────
    const isStatus =
      lk.includes("status") || lk.includes("state") || typeof value === "boolean";

    if (isStatus) {
      const strVal = String(value).toLowerCase();
      const status =
        value === true ||
        strVal === "ok" ||
        strVal === "success" ||
        strVal === "done" ||
        strVal === "completed"
          ? "success"
          : value === false || strVal === "error" || strVal === "failed"
          ? "error"
          : strVal.includes("warn") || strVal.includes("partial")
          ? "warning"
          : "info";
      components.push({
        component_type: "StatusBadge",
        props: { label, status, description: String(value) },
      });
    } else {
      components.push({ component_type: "MetricCard", props: { title: label, value: String(value) } });
    }
  }

  return { components };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function cardMeta(
  key: string,
  role?: string
): { label: string; icon: string; accent: string } {
  // Role-based overrides take priority during live streaming so the card
  // shows the correct icon before the result text arrives.
  if (role === "Coder")
    return { label: "Python Execution", icon: "🐍", accent: "border-yellow-700" };

  const k = key.toLowerCase();
  if (k.includes("flight"))
    return { label: "Flight Research", icon: "✈️", accent: "border-sky-700" };
  if (k.includes("hotel"))
    return { label: "Hotel Research", icon: "🏨", accent: "border-emerald-700" };
  if (k.includes("python") || k.includes("code") || k.includes("script"))
    return { label: "Python Execution", icon: "🐍", accent: "border-yellow-700" };
  if (k.includes("calculator") || k.includes("total") || k.includes("add"))
    return { label: "Cost Calculation", icon: "🧮", accent: "border-violet-700" };
  if (k.includes("save") || k.includes("write") || k.includes("report"))
    return { label: "Final Report", icon: "💾", accent: "border-amber-700" };

  // Generic role fallbacks for tasks whose keys aren't otherwise matched.
  if (role === "WebSearcher")
    return { label: "Web Research", icon: "🔍", accent: "border-sky-700" };
  if (role === "Analyst")
    return { label: "Analysis", icon: "🧮", accent: "border-violet-700" };
  if (role === "Planner")
    return { label: "Planning", icon: "📋", accent: "border-gray-700" };
  if (role === "Designer")
    return { label: "UI Builder", icon: "🎨", accent: "border-pink-700" };

  return {
    label: key.replace(/_/g, " ").slice(0, 48),
    icon: "📋",
    accent: "border-gray-700",
  };
}

function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// ── CodeBlock ─────────────────────────────────────────────────────────────────

const LANG_LABELS: Record<string, string> = {
  python: "Python",
  py: "Python",
  js: "JavaScript",
  ts: "TypeScript",
  bash: "Shell",
  sh: "Shell",
  json: "JSON",
  rust: "Rust",
};

function CodeBlock({ lang, children }: { lang: string; children: string }) {
  const [copied, setCopied] = useState(false);
  const label = LANG_LABELS[lang.toLowerCase()] ?? lang.toUpperCase();

  function handleCopy() {
    navigator.clipboard.writeText(children.trimEnd()).catch(() => {});
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  return (
    <div className="my-3 rounded-lg overflow-hidden border border-gray-700 bg-gray-950">
      <div className="flex items-center justify-between px-4 py-1.5 bg-gray-900 border-b border-gray-700">
        <span className="text-[11px] font-semibold tracking-wide text-indigo-400">
          {label}
        </span>
        <button
          onClick={handleCopy}
          className="text-[11px] text-gray-400 hover:text-gray-100 transition-colors select-none"
        >
          {copied ? "✓ Copied" : "Copy"}
        </button>
      </div>
      <pre className="overflow-x-auto p-4 text-xs leading-relaxed text-gray-300 font-mono whitespace-pre">
        <code>{children.trimEnd()}</code>
      </pre>
    </div>
  );
}

// ── Markdown components ───────────────────────────────────────────────────────

const mdComponents: React.ComponentProps<typeof ReactMarkdown>["components"] = {
  h1: ({ children }) => (
    <h1 className="text-base font-bold text-white mt-3 mb-1">{children}</h1>
  ),
  h2: ({ children }) => (
    <h2 className="text-sm font-bold text-gray-100 mt-3 mb-1">{children}</h2>
  ),
  h3: ({ children }) => (
    <h3 className="text-sm font-semibold text-gray-200 mt-2 mb-1">{children}</h3>
  ),
  strong: ({ children }) => (
    <strong className="font-semibold text-white">{children}</strong>
  ),
  p: ({ children }) => (
    <p className="text-gray-300 text-sm leading-relaxed mb-2">{children}</p>
  ),
  ul: ({ children }) => (
    <ul className="list-disc pl-5 space-y-1 mb-2">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="list-decimal pl-5 space-y-1 mb-2">{children}</ol>
  ),
  li: ({ children }) => (
    <li className="text-gray-300 text-sm leading-relaxed">{children}</li>
  ),
  table: ({ children }) => (
    <div className="overflow-x-auto my-3">
      <table className="w-full text-sm border-collapse">{children}</table>
    </div>
  ),
  thead: ({ children }) => <thead className="bg-gray-700">{children}</thead>,
  th: ({ children }) => (
    <th className="text-left text-gray-100 font-semibold px-3 py-2 border border-gray-600">
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td className="text-gray-300 px-3 py-2 border border-gray-600">{children}</td>
  ),
  // Pass-through pre so CodeBlock controls the wrapper element.
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => {
    const lang = className?.replace("language-", "") ?? "";
    if (lang) {
      return <CodeBlock lang={lang}>{String(children)}</CodeBlock>;
    }
    return (
      <code className="bg-gray-900 text-emerald-400 text-xs px-1.5 py-0.5 rounded">
        {children}
      </code>
    );
  },
  blockquote: ({ children }) => (
    <blockquote className="border-l-2 border-gray-500 pl-3 italic text-gray-400 my-2">
      {children}
    </blockquote>
  ),
  hr: () => <hr className="border-gray-700 my-3" />,
};

// ── Main component ────────────────────────────────────────────────────────────

export default function Home() {
  const [intent, setIntent] = useState("Plan a trip to Rome");
  const [missionStatus, setMissionStatus] = useState<MissionStatus>("idle");
  const [streamCards, setStreamCards] = useState<Record<string, StreamCard>>({});
  const [cardOrder, setCardOrder] = useState<string[]>([]);
  const [missionMeta, setMissionMeta] = useState<MissionMeta | null>(null);
  const [missionState, setMissionState] = useState<MissionState | null>(null);
  const [showDetails, setShowDetails] = useState(false);
  const [activeAgent, setActiveAgent] = useState<{ role: string; intent: string } | null>(null);
  const [governorBanner, setGovernorBanner] = useState<string | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [history, setHistory] = useState<MissionSummary[]>([]);
  const [activeMissionId, setActiveMissionId] = useState<string | null>(null);

  const fetchHistory = useCallback(async () => {
    try {
      const res = await fetch("http://localhost:8080/missions");
      if (res.ok) setHistory(await res.json());
    } catch {
      // Server may not be running yet — ignore.
    }
  }, []);

  useEffect(() => {
    fetchHistory();
  }, [fetchHistory]);

  // Apply design tokens whenever a new mission state arrives; reset on new run.
  useEffect(() => {
    if (missionState?.design_tokens) {
      applyDesignTokens(missionState.design_tokens);
    }
  }, [missionState]);

  useEffect(() => {
    if (missionStatus === "streaming") resetDesignTokens();
  }, [missionStatus]);

  // Load a past mission from the snapshot store.
  async function loadMission(id: string) {
    if (missionStatus === "streaming") return;
    setActiveMissionId(id);
    setFetchError(null);
    setGovernorBanner(null);
    setMissionState(null);
    setShowDetails(false);

    try {
      const res = await fetch(`http://localhost:8080/missions/${id}`);
      const data = await res.json();
      if (!res.ok) {
        setFetchError(data.error ?? "Failed to load mission.");
        return;
      }

      // Convert context.data (slug → result) into StreamCards for uniform display.
      const cards: Record<string, StreamCard> = {};
      const order: string[] = [];
      for (const [slug, result] of Object.entries(
        (data.context?.data ?? {}) as Record<string, string>
      )) {
        cards[slug] = { slug, role: "", intent: "", status: "completed", result };
        order.push(slug);
      }

      setStreamCards(cards);
      setCardOrder(order);
      setMissionMeta({
        task_count: data.task_count ?? order.length,
        expanded_task_count: data.expanded_task_count ?? 0,
        mission_id: data.id ?? id,
        intent: data.intent ?? "",
        layout_hint: data.layout_hint,
      });
      if (data.mission_state?.data_payload) {
        setMissionState(data.mission_state as MissionState);
      }
      setMissionStatus("complete");
      setActiveAgent(null);
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Could not load mission.");
    }
  }

  // Execute a new mission via the streaming SSE endpoint.
  async function runMission() {
    if (!intent.trim() || missionStatus === "streaming") return;

    setMissionStatus("streaming");
    setStreamCards({});
    setCardOrder([]);
    setMissionMeta(null);
    setMissionState(null);
    setShowDetails(false);
    setActiveAgent(null);
    setGovernorBanner(null);
    setFetchError(null);
    setActiveMissionId(null);

    // SSE event handler — defined inside runMission so it closes over the
    // state setters without needing useCallback.
    function handleSSEEvent(eventType: string, raw: string) {
      try {
        const p = JSON.parse(raw);
        switch (eventType) {
          case "task_started":
            setStreamCards((prev) => ({
              ...prev,
              [p.slug]: { slug: p.slug, role: p.role, intent: p.intent, status: "running" },
            }));
            setCardOrder((prev) => (prev.includes(p.slug) ? prev : [...prev, p.slug]));
            setActiveAgent({ role: p.role, intent: p.intent });
            break;

          case "task_completed":
            setStreamCards((prev) => ({
              ...prev,
              [p.slug]: { ...prev[p.slug], slug: p.slug, role: p.role, status: "completed", result: p.result },
            }));
            setActiveAgent(null);
            break;

          case "task_failed":
            setStreamCards((prev) => ({
              ...prev,
              [p.slug]: { ...prev[p.slug], status: "failed" },
            }));
            setActiveAgent(null);
            break;

          case "governor_expand":
            setGovernorBanner(
              `🔭 Governor expanding mission with ${p.new_task_count} new task(s)`
            );
            setTimeout(() => setGovernorBanner(null), 6000);
            break;

          case "mission_complete":
            setMissionMeta({
              task_count: p.task_count,
              expanded_task_count: p.expanded_task_count,
              mission_id: p.mission_id,
              intent: p.intent,
              layout_hint: p.layout_hint,
            });
            if (p.mission_state?.data_payload) {
              setMissionState(p.mission_state as MissionState);
              // State payload is the primary view — keep agent reasoning collapsed.
              setShowDetails(false);
            } else {
              // No state payload produced — auto-reveal agent cards so the user
              // always sees a result rather than a blank screen.
              setShowDetails(true);
            }
            setMissionStatus("complete");
            setActiveAgent(null);
            fetchHistory();
            break;

          case "mission_failed":
            setFetchError(p.error);
            setMissionStatus("failed");
            setActiveAgent(null);
            break;
        }
      } catch {
        // Ignore malformed events.
      }
    }

    try {
      const res = await fetch("http://localhost:8080/execute", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ intent }),
      });

      if (!res.ok) {
        const err = await res.json().catch(() => ({}));
        setFetchError((err as { error?: string }).error ?? `Server error ${res.status}`);
        setMissionStatus("failed");
        return;
      }

      if (!res.body) {
        setFetchError("No response body from server.");
        setMissionStatus("failed");
        return;
      }

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        // Accumulate decoded bytes; SSE events are delimited by \n\n.
        buffer += decoder.decode(value, { stream: true });
        const chunks = buffer.split("\n\n");
        buffer = chunks.pop() ?? "";

        for (const chunk of chunks) {
          if (!chunk.trim()) continue;
          let eventType = "";
          let data = "";
          for (const line of chunk.split("\n")) {
            if (line.startsWith("event: ")) eventType = line.slice(7).trim();
            else if (line.startsWith("data: ")) data = line.slice(6).trim();
          }
          if (eventType && data) handleSSEEvent(eventType, data);
        }
      }

      // If the stream ends without a mission_complete event, mark as failed.
      setMissionStatus((prev) => (prev === "streaming" ? "failed" : prev));
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Connection error.");
      setMissionStatus("failed");
    }
  }

  const isStreaming = missionStatus === "streaming";
  const hasCards = cardOrder.length > 0;

  return (
    <div className="min-h-screen bg-gray-950 text-gray-100 flex">

      {/* ── History Sidebar ───────────────────────────────────────────── */}
      <aside className="w-60 shrink-0 bg-gray-900 border-r border-gray-800 flex flex-col">
        <div className="px-4 py-4 border-b border-gray-800">
          <h2 className="text-[11px] font-semibold uppercase tracking-widest text-gray-500">
            History
          </h2>
        </div>

        <div className="flex-1 overflow-y-auto">
          {history.length === 0 ? (
            <p className="px-4 py-8 text-xs text-gray-600 text-center leading-relaxed">
              No past missions yet.
              <br />
              Run your first mission to see it here.
            </p>
          ) : (
            <ul>
              {history.map((m) => {
                const isActive = activeMissionId === m.id;
                return (
                  <li key={m.id} className="border-b border-gray-800/60 last:border-0">
                    <button
                      onClick={() => loadMission(m.id)}
                      className={`w-full text-left px-4 py-3 transition-colors hover:bg-gray-800/70 ${
                        isActive ? "bg-gray-800 border-l-2 border-indigo-500 pl-[14px]" : ""
                      }`}
                    >
                      <p className="text-xs font-medium text-gray-200 leading-snug line-clamp-2">
                        {m.intent}
                      </p>
                      <p className="text-[10px] text-gray-500 mt-1">
                        {formatDate(m.timestamp)}
                      </p>
                      <div className="flex items-center gap-1.5 mt-1.5 flex-wrap">
                        <span
                          className={`text-[10px] font-semibold rounded-full px-2 py-0.5 ${
                            m.status === "completed"
                              ? "bg-emerald-900/50 text-emerald-400"
                              : "bg-red-900/50 text-red-400"
                          }`}
                        >
                          {m.status === "completed" ? "✓" : "✗"}{" "}
                          {m.task_count} task{m.task_count !== 1 ? "s" : ""}
                        </span>
                        {m.layout_hint && (
                          <span className="text-[10px] text-gray-500">
                            {m.layout_hint === "Synthesized"
                              ? "🧠"
                              : m.layout_hint === "Analytical"
                              ? "🐍"
                              : "🗺️"}
                          </span>
                        )}
                      </div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </aside>

      {/* ── Main Content ──────────────────────────────────────────────── */}
      <main className="flex-1 flex flex-col items-center py-16 px-6 overflow-y-auto">
        <div className="w-full max-w-2xl space-y-8">

          {/* Header */}
          <header className="text-center space-y-1">
            <h1 className="text-4xl font-bold tracking-tight">⚡ Axion Stack</h1>
            <p className="text-gray-400 text-sm">AI Agent Mission Control</p>
          </header>

          {/* Input row */}
          <div className="flex gap-3">
            <input
              type="text"
              value={intent}
              onChange={(e) => setIntent(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && runMission()}
              placeholder="Describe your mission intent…"
              disabled={isStreaming}
              className="flex-1 bg-gray-800 border border-gray-700 rounded-lg px-4 py-3 text-sm
                         placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-indigo-500
                         disabled:opacity-50"
            />
            <button
              onClick={runMission}
              disabled={isStreaming || !intent.trim()}
              className="bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed
                         text-white font-semibold px-6 py-3 rounded-lg text-sm transition-colors"
            >
              {isStreaming ? "Running…" : "Execute"}
            </button>
          </div>

          {/* Active agent indicator — shows which specialist is working */}
          {isStreaming && activeAgent && (
            <div
              className="flex items-center gap-3 rounded-xl px-4 py-3"
              style={{
                background: "var(--axion-glass-bg, rgb(31 41 55 / 0.5))",
                border: "1px solid",
                borderColor: "var(--axion-glass-border, rgb(55 65 81 / 0.6))",
                backdropFilter: "blur(var(--axion-blur, 0px))",
                WebkitBackdropFilter: "blur(var(--axion-blur, 0px))",
                boxShadow: "0 0 18px var(--axion-glow, transparent)",
              }}
            >
              <div
                className="w-4 h-4 rounded-full animate-spin shrink-0"
                style={{
                  border: "2px solid var(--axion-accent, #6366f1)",
                  borderTopColor: "transparent",
                }}
              />
              <div className="min-w-0">
                <p
                  className="text-xs font-semibold"
                  style={{ color: "var(--axion-accent, #a5b4fc)" }}
                >
                  {activeAgent.role}
                </p>
                <p className="text-xs text-gray-400 truncate">{activeAgent.intent}</p>
              </div>
            </div>
          )}

          {/* Initial spinner — before the first task_started event */}
          {isStreaming && !hasCards && !activeAgent && (
            <div className="flex flex-col items-center gap-4 py-10">
              <div
                className="w-9 h-9 rounded-full animate-spin"
                style={{
                  border: "3px solid var(--axion-accent, #6366f1)",
                  borderTopColor: "transparent",
                  boxShadow: "0 0 16px var(--axion-glow, rgba(99,102,241,0.3))",
                }}
              />
              <p className="text-gray-400 text-sm">🤖 Axion swarm initializing…</p>
            </div>
          )}

          {/* Governor expand banner */}
          {governorBanner && (
            <div className="bg-indigo-950/70 border border-indigo-700 rounded-xl px-4 py-2.5 text-indigo-300 text-xs">
              {governorBanner}
            </div>
          )}

          {/* Error */}
          {fetchError && missionStatus !== "streaming" && (
            <div className="bg-red-950 border border-red-700 rounded-xl p-4 text-red-300 text-sm">
              ❌ {fetchError}
            </div>
          )}

          {/* Result section — hidden during streaming; revealed only after completion */}
          {hasCards && missionStatus !== "streaming" && (
            <section className="space-y-4">

              {/* ── Section header ── */}
              <div className="flex items-center gap-3 flex-wrap">
                <p className="text-xs font-semibold uppercase tracking-widest text-gray-500">
                  {missionStatus === "complete"
                    ? activeMissionId
                      ? "Loaded from history"
                      : "Mission complete"
                    : "Mission in progress"}{" "}
                  {missionMeta &&
                    `— ${missionMeta.task_count} task${missionMeta.task_count !== 1 ? "s" : ""}`}
                </p>
                {missionMeta?.expanded_task_count != null &&
                  missionMeta.expanded_task_count > 0 && (
                    <span className="inline-flex items-center gap-1 text-xs font-semibold bg-indigo-900/60 border border-indigo-600 text-indigo-300 rounded-full px-2.5 py-0.5">
                      🔭 +{missionMeta.expanded_task_count} expanded
                    </span>
                  )}
                {missionMeta?.layout_hint && (
                  <span className="text-xs text-gray-500">
                    {missionMeta.layout_hint === "Synthesized"
                      ? "🧠 Synthesized"
                      : missionMeta.layout_hint === "Analytical"
                      ? "🐍 Analytical"
                      : "🗺️ Itinerary"}
                  </span>
                )}
              </div>

              {activeMissionId && missionMeta?.intent && (
                <p className="text-xs text-gray-500 italic -mt-2">{missionMeta.intent}</p>
              )}

              {/* ── Dashboard — shown when a MissionState payload is available ── */}
              {missionState && (() => {
                const blueprint = applicationMapper(missionState.data_payload);
                return blueprint.components.length > 0 ? (
                  <div className="space-y-3">
                    <div className="flex items-center justify-between">
                      <p className="text-[11px] font-semibold uppercase tracking-widest text-pink-400/80">
                        🧠 Synthesized State
                      </p>
                      <button
                        onClick={() => setShowDetails((v) => !v)}
                        className="text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
                      >
                        {showDetails ? "Hide agent reasoning" : "Show agent reasoning"}
                      </button>
                    </div>
                    <Renderer blueprint={blueprint} />
                  </div>
                ) : null;
              })()}

              {/* ── Raw agent cards ─────────────────────────────────────────────
                   Always visible when no state payload exists.
                   Collapsible under "Agent Reasoning" when a state is shown. ── */}
              {(!missionState || showDetails) && (() => {
                // Filter out the card whose result IS the MissionState JSON so raw
                // JSON is never shown as a markdown block.
                const visibleSlugs = cardOrder.filter((slug) => {
                  const result = streamCards[slug]?.result;
                  if (!result) return true;
                  try {
                    const parsed = JSON.parse(result);
                    return !(
                      parsed &&
                      typeof parsed.data_payload === "object" &&
                      parsed.data_payload !== null
                    );
                  } catch {
                    return true;
                  }
                });

                if (visibleSlugs.length === 0) return null;

                return (
                  <div className="space-y-4">
                    {missionState && (
                      <p className="text-[11px] font-semibold uppercase tracking-widest text-gray-600">
                        Agent Reasoning
                      </p>
                    )}
                    {visibleSlugs.map((slug) => {
                      const card = streamCards[slug];
                      if (!card) return null;
                      const { label, icon, accent } = cardMeta(slug, card.role);
                      const isRunning = card.status === "running";

                      return (
                        <article
                          key={slug}
                          className={`border rounded-xl transition-opacity ${
                            isRunning ? "opacity-80" : "opacity-100"
                          }`}
                          style={{
                            background: "var(--axion-glass-bg, rgb(31 41 55 / 0.6))",
                            borderColor: isRunning
                              ? "var(--axion-accent, #6366f1)"
                              : `var(--axion-glass-border, rgb(55 65 81 / 0.7))`,
                            backdropFilter: "blur(var(--axion-blur, 0px))",
                            WebkitBackdropFilter: "blur(var(--axion-blur, 0px))",
                            padding: "var(--axion-pad, 20px)",
                            boxShadow: isRunning
                              ? "0 0 16px var(--axion-glow, transparent)"
                              : "none",
                          }}
                        >
                          <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-200 mb-3">
                            <span>{icon}</span>
                            <span>{label}</span>
                            {isRunning && (
                              <span className="ml-auto flex items-center gap-1.5 text-xs text-gray-400 font-normal">
                                <span
                                  className="w-3 h-3 rounded-full animate-spin inline-block"
                                  style={{
                                    border: "2px solid var(--axion-accent, #6366f1)",
                                    borderTopColor: "transparent",
                                  }}
                                />
                                Working…
                              </span>
                            )}
                            {card.status === "failed" && (
                              <span className="ml-auto text-xs text-red-400 font-normal">
                                ✗ Failed
                              </span>
                            )}
                          </h2>

                          {isRunning ? (
                            <p className="text-xs text-gray-500 italic animate-pulse">
                              Agent is working…
                            </p>
                          ) : showDetails && card.result ? (
                            <ReactMarkdown
                              remarkPlugins={[remarkGfm]}
                              components={mdComponents}
                            >
                              {card.result}
                            </ReactMarkdown>
                          ) : null}
                        </article>
                      );
                    })}
                  </div>
                );
              })()}

            </section>
          )}

        </div>
      </main>
    </div>
  );
}
