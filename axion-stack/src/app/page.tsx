"use client";

import { useState, useEffect, useCallback } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Renderer, UIBlueprint } from "@/components/Renderer";

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
  const [uiBlueprint, setUiBlueprint] = useState<UIBlueprint | null>(null);
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

  // Load a past mission from the snapshot store.
  async function loadMission(id: string) {
    if (missionStatus === "streaming") return;
    setActiveMissionId(id);
    setFetchError(null);
    setGovernorBanner(null);
    setUiBlueprint(null);
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
      if (data.ui_blueprint?.components?.length) {
        setUiBlueprint(data.ui_blueprint as UIBlueprint);
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
    setUiBlueprint(null);
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
            if (p.ui_blueprint?.components?.length) {
              setUiBlueprint(p.ui_blueprint as UIBlueprint);
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
                            {m.layout_hint === "Designed"
                              ? "📊"
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
            <div className="flex items-center gap-3 rounded-xl bg-gray-800/50 border border-gray-700/60 px-4 py-3">
              <div className="w-4 h-4 border-2 border-indigo-400 border-t-transparent rounded-full animate-spin shrink-0" />
              <div className="min-w-0">
                <p className="text-xs font-semibold text-indigo-300">{activeAgent.role}</p>
                <p className="text-xs text-gray-400 truncate">{activeAgent.intent}</p>
              </div>
            </div>
          )}

          {/* Initial spinner — before the first task_started event */}
          {isStreaming && !hasCards && !activeAgent && (
            <div className="flex flex-col items-center gap-4 py-10">
              <div className="w-9 h-9 border-[3px] border-indigo-500 border-t-transparent rounded-full animate-spin" />
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

          {/* Result section — visible while streaming AND after completion */}
          {hasCards && (
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
                    {missionMeta.layout_hint === "Designed"
                      ? "📊 Designed"
                      : missionMeta.layout_hint === "Analytical"
                      ? "🐍 Analytical"
                      : "🗺️ Itinerary"}
                  </span>
                )}
              </div>

              {activeMissionId && missionMeta?.intent && (
                <p className="text-xs text-gray-500 italic -mt-2">{missionMeta.intent}</p>
              )}

              {/* ── Dashboard (Renderer) — shown when a UIBlueprint is available ── */}
              {uiBlueprint && (
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <p className="text-[11px] font-semibold uppercase tracking-widest text-pink-400/80">
                      📊 Dashboard
                    </p>
                    <button
                      onClick={() => setShowDetails((v) => !v)}
                      className="text-[11px] text-gray-500 hover:text-gray-300 transition-colors"
                    >
                      {showDetails ? "Hide agent reasoning" : "Show agent reasoning"}
                    </button>
                  </div>
                  <Renderer blueprint={uiBlueprint} />
                </div>
              )}

              {/* ── Raw agent cards ─────────────────────────────────────────────
                   Always visible when no blueprint exists.
                   Collapsible under "Agent Reasoning" when a blueprint is shown. ── */}
              {(!uiBlueprint || showDetails) && (() => {
                // Filter out the card whose result IS the UIBlueprint JSON so raw JSON
                // is never shown as a markdown block.
                const visibleSlugs = cardOrder.filter((slug) => {
                  const result = streamCards[slug]?.result;
                  if (!result) return true;
                  try {
                    const parsed = JSON.parse(result);
                    return !(
                      parsed &&
                      Array.isArray(parsed.components) &&
                      parsed.components.length > 0
                    );
                  } catch {
                    return true;
                  }
                });

                if (visibleSlugs.length === 0) return null;

                return (
                  <div className="space-y-4">
                    {uiBlueprint && (
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
                          className={`bg-gray-800/60 border ${accent} rounded-xl p-5 transition-opacity ${
                            isRunning ? "opacity-80" : "opacity-100"
                          }`}
                        >
                          <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-200 mb-3">
                            <span>{icon}</span>
                            <span>{label}</span>
                            {isRunning && (
                              <span className="ml-auto flex items-center gap-1.5 text-xs text-gray-400 font-normal">
                                <span className="w-3 h-3 border-2 border-indigo-400 border-t-transparent rounded-full animate-spin inline-block" />
                                Working…
                              </span>
                            )}
                            {card.status === "failed" && (
                              <span className="ml-auto text-xs text-red-400 font-normal">
                                ✗ Failed
                              </span>
                            )}
                          </h2>

                          {card.result ? (
                            <ReactMarkdown
                              remarkPlugins={[remarkGfm]}
                              components={mdComponents}
                            >
                              {card.result}
                            </ReactMarkdown>
                          ) : isRunning ? (
                            <p className="text-xs text-gray-500 italic animate-pulse">
                              Agent is working…
                            </p>
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
