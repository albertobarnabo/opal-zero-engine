"use client";

import { useState } from "react";

interface MissionResponse {
  status: string;
  context: {
    data: Record<string, string>;
  };
  error?: string;
}

function cardMeta(key: string): { label: string; icon: string; accent: string } {
  const k = key.toLowerCase();
  if (k.includes("flight"))
    return { label: "Flight Research", icon: "✈️", accent: "border-sky-700" };
  if (k.includes("hotel"))
    return { label: "Hotel Research", icon: "🏨", accent: "border-emerald-700" };
  if (k.includes("calculator") || k.includes("total") || k.includes("add_300"))
    return { label: "Cost Calculation", icon: "🧮", accent: "border-violet-700" };
  if (k.includes("save") || k.includes("write") || k.includes("report"))
    return { label: "Final Report", icon: "💾", accent: "border-amber-700" };
  return { label: key.replace(/_/g, " ").slice(0, 48), icon: "📋", accent: "border-gray-700" };
}

export default function Home() {
  const [intent, setIntent] = useState("Plan a trip to Rome");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<MissionResponse | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);

  async function runMission() {
    if (!intent.trim() || loading) return;
    setLoading(true);
    setResult(null);
    setFetchError(null);

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

  return (
    <main className="min-h-screen bg-gray-950 text-gray-100 flex flex-col items-center py-16 px-4">
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
            <p className="text-xs font-semibold uppercase tracking-widest text-gray-500">
              Mission complete — {entries.length} task{entries.length !== 1 ? "s" : ""}
            </p>

            {entries.map(([key, value]) => {
              const { label, icon, accent } = cardMeta(key);
              return (
                <article
                  key={key}
                  className={`bg-gray-800/60 border ${accent} rounded-xl p-5 space-y-2`}
                >
                  <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-200">
                    <span>{icon}</span>
                    <span>{label}</span>
                  </h2>
                  <p className="text-gray-300 text-sm leading-relaxed whitespace-pre-wrap">
                    {value}
                  </p>
                </article>
              );
            })}
          </section>
        )}

      </div>
    </main>
  );
}
