"use client";

import { useState, useEffect, useCallback } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

interface MissionResponse {
  status: string;
  intent?: string;
  task_count?: number;
  expanded_task_count?: number;
  context: {
    data: Record<string, string>;
  };
  error?: string;
}

interface MissionSummary {
  id: string;
  timestamp: number;
  intent: string;
  task_count: number;
  status: string;
}

function cardMeta(key: string): { label: string; icon: string; accent: string } {
  const k = key.toLowerCase();
  if (k.includes("flight"))
    return { label: "Flight Research", icon: "✈️", accent: "border-sky-700" };
  if (k.includes("hotel"))
    return { label: "Hotel Research", icon: "🏨", accent: "border-emerald-700" };
  if (k.includes("python") || k.includes("code") || k.includes("script") || k.includes("coder"))
    return { label: "Python Execution", icon: "🐍", accent: "border-yellow-700" };
  if (k.includes("calculator") || k.includes("total") || k.includes("add"))
    return { label: "Cost Calculation", icon: "🧮", accent: "border-violet-700" };
  if (k.includes("save") || k.includes("write") || k.includes("report"))
    return { label: "Final Report", icon: "💾", accent: "border-amber-700" };
  return { label: key.replace(/_/g, " ").slice(0, 48), icon: "📋", accent: "border-gray-700" };
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
      {/* header bar */}
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
      {/* code body */}
      <pre className="overflow-x-auto p-4 text-xs leading-relaxed text-gray-300 font-mono whitespace-pre">
        <code>{children.trimEnd()}</code>
      </pre>
    </div>
  );
}

function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

// Tailwind-styled Markdown components — works without the typography plugin.
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
  // pre wraps fenced code blocks — strip it so CodeBlock controls the container.
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

export default function Home() {
  const [intent, setIntent] = useState("Plan a trip to Rome");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<MissionResponse | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [history, setHistory] = useState<MissionSummary[]>([]);
  const [activeMissionId, setActiveMissionId] = useState<string | null>(null);

  const fetchHistory = useCallback(async () => {
    try {
      const res = await fetch("http://localhost:8080/missions");
      if (res.ok) setHistory(await res.json());
    } catch {
      // Server may not be running yet — silently ignore.
    }
  }, []);

  useEffect(() => {
    fetchHistory();
  }, [fetchHistory]);

  async function loadMission(id: string) {
    if (loading) return;
    setLoading(true);
    setFetchError(null);
    setActiveMissionId(id);
    try {
      const res = await fetch(`http://localhost:8080/missions/${id}`);
      const data: MissionResponse = await res.json();
      if (res.ok) {
        setResult(data);
      } else {
        setFetchError(data.error ?? "Failed to load mission.");
      }
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Could not load mission.");
    } finally {
      setLoading(false);
    }
  }

  async function runMission() {
    if (!intent.trim() || loading) return;
    setLoading(true);
    setResult(null);
    setFetchError(null);
    setActiveMissionId(null);

    try {
      const res = await fetch("http://localhost:8080/execute", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ intent }),
      });
      const data: MissionResponse = await res.json();
      if (!res.ok) {
        setFetchError(data.error ?? `Server error ${res.status}`);
      } else {
        setResult(data);
        fetchHistory();
      }
    } catch (e) {
      setFetchError(
        e instanceof Error ? e.message : "Could not reach the Axion server."
      );
    } finally {
      setLoading(false);
    }
  }

  const entries = result?.context?.data ? Object.entries(result.context.data) : [];
  const displayIntent = result?.intent ?? intent;

  return (
    <div className="min-h-screen bg-gray-950 text-gray-100 flex">

      {/* ── History Sidebar ─────────────────────────────────────────────── */}
      <aside className="w-60 shrink-0 bg-gray-900 border-r border-gray-800 flex flex-col">
        <div className="px-4 py-4 border-b border-gray-800">
          <h2 className="text-[11px] font-semibold uppercase tracking-widest text-gray-500">
            History
          </h2>
        </div>

        <div className="flex-1 overflow-y-auto">
          {history.length === 0 ? (
            <p className="px-4 py-8 text-xs text-gray-600 text-center leading-relaxed">
              No past missions yet.<br />Run your first mission to see it here.
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
                        isActive
                          ? "bg-gray-800 border-l-2 border-indigo-500 pl-[14px]"
                          : ""
                      }`}
                    >
                      <p className="text-xs font-medium text-gray-200 leading-snug line-clamp-2">
                        {m.intent}
                      </p>
                      <p className="text-[10px] text-gray-500 mt-1">
                        {formatDate(m.timestamp)}
                      </p>
                      <span
                        className={`inline-block mt-1.5 text-[10px] font-semibold rounded-full px-2 py-0.5 ${
                          m.status === "completed"
                            ? "bg-emerald-900/50 text-emerald-400"
                            : "bg-red-900/50 text-red-400"
                        }`}
                      >
                        {m.status === "completed" ? "✓" : "✗"} {m.task_count} task{m.task_count !== 1 ? "s" : ""}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </aside>

      {/* ── Main Content ────────────────────────────────────────────────── */}
      <main className="flex-1 flex flex-col items-center py-16 px-6 overflow-y-auto">
        <div className="w-full max-w-2xl space-y-10">

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
              className="flex-1 bg-gray-800 border border-gray-700 rounded-lg px-4 py-3 text-sm
                         placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-indigo-500"
            />
            <button
              onClick={runMission}
              disabled={loading || !intent.trim()}
              className="bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed
                         text-white font-semibold px-6 py-3 rounded-lg text-sm transition-colors"
            >
              {loading ? "Running…" : "Execute"}
            </button>
          </div>

          {/* Loading */}
          {loading && (
            <div className="flex flex-col items-center gap-4 py-10">
              <div className="w-9 h-9 border-[3px] border-indigo-500 border-t-transparent rounded-full animate-spin" />
              <p className="text-gray-400 text-sm">🤖 Axion swarm is thinking…</p>
            </div>
          )}

          {/* Error */}
          {fetchError && !loading && (
            <div className="bg-red-950 border border-red-700 rounded-xl p-4 text-red-300 text-sm">
              ❌ {fetchError}
            </div>
          )}

          {/* Result cards */}
          {!loading && entries.length > 0 && (
            <section className="space-y-4">
              <div className="flex items-center gap-3 flex-wrap">
                <p className="text-xs font-semibold uppercase tracking-widest text-gray-500">
                  {activeMissionId ? "Loaded from history" : "Mission complete"} —{" "}
                  {result?.task_count ?? entries.length} task
                  {(result?.task_count ?? entries.length) !== 1 ? "s" : ""}
                </p>
                {result?.expanded_task_count != null && result.expanded_task_count > 0 && (
                  <span className="inline-flex items-center gap-1 text-xs font-semibold bg-indigo-900/60 border border-indigo-600 text-indigo-300 rounded-full px-2.5 py-0.5">
                    🔭 +{result.expanded_task_count} expanded
                  </span>
                )}
              </div>

              {activeMissionId && (
                <p className="text-xs text-gray-500 italic -mt-2">
                  {displayIntent}
                </p>
              )}

              {entries.map(([key, value]) => {
                const { label, icon, accent } = cardMeta(key);
                return (
                  <article
                    key={key}
                    className={`bg-gray-800/60 border ${accent} rounded-xl p-5`}
                  >
                    <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-200 mb-3">
                      <span>{icon}</span>
                      <span>{label}</span>
                    </h2>
                    <ReactMarkdown
                      remarkPlugins={[remarkGfm]}
                      components={mdComponents}
                    >
                      {value}
                    </ReactMarkdown>
                  </article>
                );
              })}
            </section>
          )}

        </div>
      </main>
    </div>
  );
}
