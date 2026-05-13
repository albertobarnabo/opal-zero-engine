"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Renderer, UIBlueprint, UIComponent } from "@/components/Renderer";
import { ActionBar } from "@/components/ActionBar";

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
  border_radius: number;
  surface_opacity: number;
  layout_strategy?: string;
}

// Derive a stable accent color from layout_hint for history items
function accentForMission(m: MissionSummary): string {
  if (m.status === "failed") return "#f87171";
  switch (m.layout_hint) {
    case "Synthesized": return "#8b9cf4";
    case "Analytical":  return "#6ee7b7";
    case "Itinerary":   return "#fbbf24";
    default:            return "#6b7280";
  }
}

// Deterministic sparkline heights from mission id
function sparklineHeights(id: string, count: number): number[] {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return Array.from({ length: Math.min(count, 8) }, () => {
    h = (h * 1664525 + 1013904223) >>> 0;
    return 4 + (h % 13);
  });
}

interface MissionState {
  intent_resolved: boolean;
  data_payload: Record<string, unknown>;
  verification_logs: string[];
  design_tokens: DesignTokens;
  suggested_widgets?: string[];
}

// ── Theme engine ──────────────────────────────────────────────────────────────

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
  const blur = Math.round(56 + gi * 24);            // 56–80 px (Pearl Glass)
  const so = Math.max(0, Math.min(1, tokens.surface_opacity ?? 0.04));
  // Pearl Glass: white-tinted bg — subtle but luminous on Deep Obsidian
  const bgAlpha = Math.min(so + gi * 0.025, 0.08).toFixed(3);
  const borderAlpha = (0.10 + gi * 0.08).toFixed(3);
  const insetAlpha = (0.12 + gi * 0.06).toFixed(3); // stronger specular
  const compact = tokens.layout_density === "compact";
  const radius = Math.min(tokens.border_radius ?? 24, 48);

  root.style.setProperty("--axion-accent",       tokens.primary_accent);
  root.style.setProperty("--axion-accent-rgb",   `${r},${g},${b}`);
  root.style.setProperty("--axion-blur",         `${blur}px`);
  // Pearl Glass: pure white micro-tint (the accent lives in glow, not bg)
  root.style.setProperty("--axion-glass-bg",     `rgba(255,255,255,${bgAlpha})`);
  root.style.setProperty("--axion-glass-border", `rgba(255,255,255,${borderAlpha})`);
  // Specular top highlight (simulates light hitting the top glass edge) + subtle overall edge
  root.style.setProperty("--axion-glass-inset",  `inset 0 1px 0 rgba(255,255,255,0.18), inset 0 0 0 0.5px rgba(255,255,255,${insetAlpha})`);
  root.style.setProperty("--axion-glow",         `rgba(${r},${g},${b},0.35)`);
  root.style.setProperty("--axion-pad",          compact ? "16px" : "24px");
  root.style.setProperty("--axion-gap",          compact ? "20px" : "36px");
  root.style.setProperty("--axion-radius",       `${radius}px`);
}

function resetDesignTokens() {
  const props = [
    "--axion-accent", "--axion-accent-rgb", "--axion-blur", "--axion-glass-bg",
    "--axion-glass-border", "--axion-glass-inset", "--axion-glow", "--axion-pad",
    "--axion-gap", "--axion-radius",
  ];
  props.forEach((p) => document.documentElement.style.removeProperty(p));
}

// ── ApplicationMapper ─────────────────────────────────────────────────────────

function formatKey(key: string): string {
  return key
    .replace(/_/g, " ")
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .replace(/\b\w/g, (c) => c.toUpperCase());
}

const TEMPORAL_KEYS = ["date", "time", "month", "year", "week", "day", "quarter", "period", "timestamp"];
const IMAGE_KEYS    = ["visual", "image", "photo", "scene", "render", "illustration", "picture"];

function isChartArray(arr: unknown[]): boolean {
  if (arr.length < 2) return false; // allow 2-item comparisons
  const first = arr[0];
  if (typeof first !== "object" || first === null || Array.isArray(first)) return false;
  const keys = Object.keys(first as object);
  const hasTemporalKey = keys.some((k) => TEMPORAL_KEYS.some((t) => k.toLowerCase().includes(t)));
  // coerce string numbers — LLMs frequently serialize numbers as strings
  const numericKeys = keys.filter((k) => {
    const v = (first as Record<string, unknown>)[k];
    if (typeof v === "number") return true;
    if (typeof v === "string" && v.trim() !== "" && !isNaN(Number(v))) return true;
    return false;
  });
  return hasTemporalKey || numericKeys.length >= 2;
}

function applicationMapper(
  payload: Record<string, unknown>,
  hints: string[] = [],
  layoutStrategy?: string,
): UIBlueprint {
  const hintMap = new Map<string, string>();
  for (const h of hints) {
    const sep = h.indexOf(":");
    if (sep > 0) hintMap.set(h.slice(sep + 1), h.slice(0, sep));
  }

  const components: UIComponent[] = [];

  for (const [key, value] of Object.entries(payload)) {
    if (value === null || value === undefined) continue;
    const label = formatKey(key);
    const lk = key.toLowerCase();

    const hintType = hintMap.get(key);
    if (hintType === "ChartCard" && Array.isArray(value)) {
      components.push({ component_type: "ChartCard", props: { title: label, data: value } });
      continue;
    }
    if (hintType === "ImageCard") {
      components.push({ component_type: "ImageCard", props: { title: label, description: String(value) } });
      continue;
    }

    if (typeof value === "string" && IMAGE_KEYS.some((k) => lk.includes(k))) {
      components.push({ component_type: "ImageCard", props: { title: label, description: value } });
      continue;
    }

    if (Array.isArray(value)) {
      if (value.length === 0) continue;
      const first = value[0];

      // ChartCard FIRST — before Timeline, so numeric arrays aren't stolen by label check
      if (isChartArray(value)) {
        components.push({ component_type: "ChartCard", props: { title: label, data: value } });
        continue;
      }

      // Timeline: objects with a "label" key and no strong numeric signature
      if (
        typeof first === "object" &&
        first !== null &&
        !Array.isArray(first) &&
        "label" in (first as object)
      ) {
        components.push({ component_type: "Timeline", props: { title: label, steps: value } });
        continue;
      }

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

      components.push({
        component_type: "ComparisonTable",
        props: { title: label, headers: [label], rows: value.map((v) => [String(v)]) },
      });
      continue;
    }

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

      // Chart coercion: >2 numeric entries → synthetic bar chart (more visual than a 2-column table)
      const numericEntries = Object.entries(obj).filter(([, v]) => {
        if (typeof v === "number") return true;
        if (typeof v === "string" && v.trim() !== "" && !isNaN(Number(v))) return true;
        return false;
      });
      if (numericEntries.length > 2) {
        const chartData = numericEntries.map(([k, v]) => ({
          metric: formatKey(k),
          value: typeof v === "number" ? v : Number(v),
        }));
        components.push({
          component_type: "ChartCard",
          props: { title: label, data: chartData, chartType: "bar", xKey: "metric", dataKeys: ["value"] },
        });
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

    const isStatus =
      lk.includes("status") || lk.includes("state") || typeof value === "boolean";

    if (isStatus) {
      const strVal = String(value).toLowerCase();
      const status =
        value === true || strVal === "ok" || strVal === "success" || strVal === "done" || strVal === "completed"
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

  // Do NOT pre-assign spans here — let Renderer.resolveSpan handle it with strategy awareness.
  // Only set span for MetricCards with long values that always need more space.
  const withSpans = components.map((c) => ({
    ...c,
    span:
      c.component_type === "MetricCard" &&
      (String(c.props.value ?? "").length > 15 || c.props.subtitle)
        ? { col: 2, row: 1 }
        : undefined,
  }));

  return { components: withSpans, layout_strategy: layoutStrategy };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function cardMeta(key: string, role?: string): { label: string; icon: string; accent: string } {
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

  if (role === "WebSearcher")
    return { label: "Web Research", icon: "🔍", accent: "border-sky-700" };
  if (role === "Analyst")
    return { label: "Analysis", icon: "🧮", accent: "border-violet-700" };
  if (role === "Planner")
    return { label: "Planning", icon: "📋", accent: "border-gray-700" };
  if (role === "Designer")
    return { label: "UI Builder", icon: "🎨", accent: "border-pink-700" };

  return { label: key.replace(/_/g, " ").slice(0, 48), icon: "📋", accent: "border-gray-700" };
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
  python: "Python", py: "Python", js: "JavaScript", ts: "TypeScript",
  bash: "Shell", sh: "Shell", json: "JSON", rust: "Rust",
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
        <span className="text-[11px] font-semibold tracking-wide text-indigo-400">{label}</span>
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
  h1: ({ children }) => <h1 className="text-base font-bold text-white mt-3 mb-1">{children}</h1>,
  h2: ({ children }) => <h2 className="text-sm font-bold text-gray-100 mt-3 mb-1">{children}</h2>,
  h3: ({ children }) => <h3 className="text-sm font-semibold text-gray-200 mt-2 mb-1">{children}</h3>,
  strong: ({ children }) => <strong className="font-semibold text-white">{children}</strong>,
  p: ({ children }) => <p className="text-gray-300 text-sm leading-relaxed mb-2">{children}</p>,
  ul: ({ children }) => <ul className="list-disc pl-5 space-y-1 mb-2">{children}</ul>,
  ol: ({ children }) => <ol className="list-decimal pl-5 space-y-1 mb-2">{children}</ol>,
  li: ({ children }) => <li className="text-gray-300 text-sm leading-relaxed">{children}</li>,
  table: ({ children }) => (
    <div className="overflow-x-auto my-3">
      <table className="w-full text-sm border-collapse">{children}</table>
    </div>
  ),
  thead: ({ children }) => <thead className="bg-gray-700">{children}</thead>,
  th: ({ children }) => (
    <th className="text-left text-gray-100 font-semibold px-3 py-2 border border-gray-600">{children}</th>
  ),
  td: ({ children }) => (
    <td className="text-gray-300 px-3 py-2 border border-gray-600">{children}</td>
  ),
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => {
    const lang = className?.replace("language-", "") ?? "";
    if (lang) return <CodeBlock lang={lang}>{String(children)}</CodeBlock>;
    return (
      <code className="bg-gray-900 text-emerald-400 text-xs px-1.5 py-0.5 rounded">{children}</code>
    );
  },
  blockquote: ({ children }) => (
    <blockquote className="border-l-2 border-gray-500 pl-3 italic text-gray-400 my-2">{children}</blockquote>
  ),
  hr: () => <hr className="border-gray-700 my-3" />,
};

// ── Main component ────────────────────────────────────────────────────────────

export default function Home() {
  const [intent, setIntent] = useState("Plan a trip to Rome");
  const [missionStatus, setMissionStatus] = useState<MissionStatus>("idle");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const userScrolledRef = useRef(false);
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
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  async function deleteMission(id: string) {
    try {
      await fetch(`http://localhost:8080/missions/${id}`, { method: "DELETE" });
    } catch { /* ignore if server is down */ }
    setHistory((prev) => prev.filter((m) => m.id !== id));
    if (activeMissionId === id) newMission();
    setConfirmDeleteId(null);
  }

  function newMission() {
    setMissionStatus("idle");
    setStreamCards({});
    setCardOrder([]);
    setMissionMeta(null);
    setMissionState(null);
    setActiveAgent(null);
    setGovernorBanner(null);
    setFetchError(null);
    setActiveMissionId(null);
    setIntent("");
    setSidebarOpen(false);
    resetDesignTokens();
    setTimeout(() => textareaRef.current?.focus(), 100);
  }

  const fetchHistory = useCallback(async () => {
    try {
      const res = await fetch("http://localhost:8080/missions");
      if (res.ok) setHistory(await res.json());
    } catch {
      // Server may not be running yet — ignore.
    }
  }, []);

  useEffect(() => { fetchHistory(); }, [fetchHistory]);

  useEffect(() => {
    if (missionState?.design_tokens) applyDesignTokens(missionState.design_tokens);
  }, [missionState]);

  useEffect(() => {
    if (missionStatus === "streaming") resetDesignTokens();
  }, [missionStatus]);

  // Scroll lock: track if the user has manually scrolled up
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const onScroll = () => {
      const atBottom = stage.scrollTop + stage.clientHeight >= stage.scrollHeight - 60;
      userScrolledRef.current = !atBottom;
    };
    stage.addEventListener("scroll", onScroll, { passive: true });
    return () => stage.removeEventListener("scroll", onScroll);
  }, []);

  // Auto-scroll to newest card when streaming, unless user scrolled up
  useEffect(() => {
    if (missionStatus !== "streaming" || userScrolledRef.current) return;
    const stage = stageRef.current;
    if (stage) stage.scrollTo({ top: stage.scrollHeight, behavior: "smooth" });
  }, [cardOrder, missionStatus]);

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
    userScrolledRef.current = false;

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
            setStreamCards((prev) => ({ ...prev, [p.slug]: { ...prev[p.slug], status: "failed" } }));
            setActiveAgent(null);
            break;
          case "governor_expand":
            setGovernorBanner(`🔭 Governor expanding mission with ${p.new_task_count} new task(s)`);
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
              setShowDetails(false);
            } else {
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

      setMissionStatus((prev) => (prev === "streaming" ? "failed" : prev));
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Connection error.");
      setMissionStatus("failed");
    }
  }

  const isIdle = missionStatus === "idle";
  const isStreaming = missionStatus === "streaming";
  const hasCards = cardOrder.length > 0;

  // ── Shared command bar ───────────────────────────────────────────────────────
  // Always lives in the Control Zone at the bottom — never floats/overlaps.
  const commandBarContent = (
    <div
      className={`relative p-[1.5px] rounded-2xl ${
        isStreaming
          ? "axion-light-trace-active axion-breathe-glow"
          : "axion-light-trace"
      }`}
      style={{
        boxShadow: "0 8px 40px rgba(0,0,0,0.75)",
        border: "0.5px solid rgba(255,255,255,0.16)",
      }}
    >
      {/* Glass inner — visibly lifted off the page bg */}
      <div
        className="relative rounded-2xl flex items-end gap-3 px-5 py-4"
        style={{
          background: "rgba(22,28,46,0.97)",
          backdropFilter: "blur(80px)",
          WebkitBackdropFilter: "blur(80px)",
          border: "0.5px solid rgba(255,255,255,0.10)",
          boxShadow: "inset 0 1px 0 rgba(255,255,255,0.10)",
        }}
      >
        <textarea
          ref={textareaRef}
          rows={1}
          value={intent}
          onChange={(e) => {
            setIntent(e.target.value);
            const el = e.target;
            el.style.height = "auto";
            el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              runMission();
            }
          }}
          placeholder={
            intent === "" && history.length > 0
              ? `Continue: "${history[0].intent.slice(0, 55)}${history[0].intent.length > 55 ? "…" : ""}"`
              : "Describe your mission intent…"
          }
          disabled={isStreaming}
          className="flex-1 bg-transparent text-lg text-gray-100 placeholder-gray-500
                     focus:outline-none resize-none leading-relaxed overflow-hidden
                     disabled:opacity-50"
          style={{ minHeight: "28px", maxHeight: "200px" }}
        />
        <button
          onClick={runMission}
          disabled={isStreaming || !intent.trim()}
          className="shrink-0 text-sm font-semibold px-5 py-2 rounded-xl transition-all
                     disabled:opacity-40 disabled:cursor-not-allowed"
          style={{
            background: isStreaming
              ? "rgba(255,255,255,0.06)"
              : "var(--axion-accent, #8b9cf4)",
            color: isStreaming ? "rgba(255,255,255,0.4)" : "#fff",
            boxShadow: isStreaming
              ? "none"
              : "0 0 20px rgba(var(--axion-accent-rgb, 139,156,244), 0.30)",
          }}
        >
          {isStreaming ? "Running…" : "Execute"}
        </button>
      </div>
    </div>
  );

  return (
    <div
      className="text-gray-100"
      style={{ display: "flex", flexDirection: "column", height: "100vh", overflow: "hidden", background: "transparent" }}
    >

      {/* ── Ambient mesh ──────────────────────────────────────────────────── */}
      <div className="axion-mesh" aria-hidden="true">
        <div className="axion-mesh-blob-1" />
        <div className="axion-mesh-blob-2" />
      </div>

      {/* ── Sidebar backdrop ──────────────────────────────────────────────── */}
      <AnimatePresence>
        {sidebarOpen && (
          <motion.div
            key="sidebar-backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={() => setSidebarOpen(false)}
            style={{
              position: "fixed", inset: 0,
              background: "rgba(0,0,0,0.45)",
              backdropFilter: "blur(4px)",
              WebkitBackdropFilter: "blur(4px)",
              zIndex: 30,
            }}
          />
        )}
      </AnimatePresence>

      {/* ── Sidebar drawer ────────────────────────────────────────────────── */}
      <AnimatePresence>
        {sidebarOpen && (
          <motion.aside
            key="sidebar-drawer"
            initial={{ x: -296 }}
            animate={{ x: 0 }}
            exit={{ x: -296 }}
            transition={{ type: "spring", stiffness: 300, damping: 30 }}
            style={{
              position: "fixed",
              top: 0, left: 0, bottom: 0,
              width: 288,
              zIndex: 40,
              display: "flex",
              flexDirection: "column",
              background: "rgba(4,8,20,0.85)",
              backdropFilter: "blur(64px)",
              WebkitBackdropFilter: "blur(64px)",
              borderRight: "0.5px solid rgba(255,255,255,0.10)",
              boxShadow: "4px 0 48px rgba(0,0,0,0.50), inset -1px 0 0 rgba(255,255,255,0.04)",
            }}
          >
            {/* Drawer header */}
            <div
              style={{
                padding: "20px 16px 14px",
                borderBottom: "0.5px solid rgba(255,255,255,0.07)",
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
              }}
            >
              <span
                style={{
                  fontSize: 10,
                  fontWeight: 600,
                  letterSpacing: "0.12em",
                  textTransform: "uppercase",
                  color: "rgba(255,255,255,0.30)",
                }}
              >
                Mission Gallery
              </span>
              <button
                onClick={() => setSidebarOpen(false)}
                style={{
                  color: "rgba(255,255,255,0.35)",
                  fontSize: 20,
                  lineHeight: 1,
                  width: 28,
                  height: 28,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  borderRadius: 8,
                  transition: "color 0.2s, background 0.2s",
                }}
                onMouseEnter={(e) => {
                  (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.75)";
                  (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.06)";
                }}
                onMouseLeave={(e) => {
                  (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.35)";
                  (e.currentTarget as HTMLButtonElement).style.background = "transparent";
                }}
              >
                ×
              </button>
            </div>

            {/* New Mission button — slate glass, pinned above scroll */}
            <div style={{ padding: "12px 12px 6px", flexShrink: 0 }}>
              <button
                onClick={newMission}
                style={{
                  width: "100%",
                  padding: "12px 16px",
                  borderRadius: 14,
                  background: "rgba(51,65,85,0.55)",
                  color: "rgba(255,255,255,0.92)",
                  fontWeight: 700,
                  fontSize: 14,
                  textAlign: "left",
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  border: "1px solid rgba(255,255,255,0.18)",
                  backdropFilter: "blur(20px)",
                  WebkitBackdropFilter: "blur(20px)",
                  boxShadow: "inset 0 1px 0 rgba(255,255,255,0.14)",
                  transition: "background 0.2s, border-color 0.2s",
                }}
                onMouseEnter={(e) => {
                  const el = e.currentTarget as HTMLButtonElement;
                  el.style.background = "rgba(71,85,105,0.65)";
                  el.style.borderColor = "rgba(255,255,255,0.28)";
                }}
                onMouseLeave={(e) => {
                  const el = e.currentTarget as HTMLButtonElement;
                  el.style.background = "rgba(51,65,85,0.55)";
                  el.style.borderColor = "rgba(255,255,255,0.18)";
                }}
              >
                <span style={{ fontSize: 18, lineHeight: 1, opacity: 0.80 }}>＋</span>
                New Mission
              </button>
            </div>

            {/* History list */}
            <div style={{ flex: 1, overflowY: "auto", padding: "6px 8px 20px" }}>
              {history.length === 0 ? (
                <div style={{ padding: "36px 16px", textAlign: "center" }}>
                  <div style={{ fontSize: 24, opacity: 0.18, marginBottom: 8 }}>◈</div>
                  <p style={{ fontSize: 11, color: "rgba(255,255,255,0.22)", lineHeight: 1.6 }}>
                    No missions yet.<br />Run your first to begin.
                  </p>
                </div>
              ) : (
                <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
                  <AnimatePresence initial={false}>
                  {history.map((m) => {
                    const accent = accentForMission(m);
                    const isActive = activeMissionId === m.id;
                    const heights = sparklineHeights(m.id, m.task_count);
                    const isConfirming = confirmDeleteId === m.id;
                    return (
                      <motion.div
                        key={m.id}
                        layout
                        initial={{ opacity: 0, x: -16 }}
                        animate={{ opacity: 1, x: 0 }}
                        exit={{ opacity: 0, x: -48, height: 0, marginBottom: 0, overflow: "hidden" }}
                        transition={{ duration: 0.22 }}
                        style={{ position: "relative" }}
                        onMouseLeave={() => { if (isConfirming) setConfirmDeleteId(null); }}
                      >
                        {/* Mission card button */}
                        <motion.button
                          onClick={() => { loadMission(m.id); setSidebarOpen(false); }}
                          whileHover={{ boxShadow: `0 0 18px ${accent}2a, 0 2px 8px rgba(0,0,0,0.30)` }}
                          style={{
                            width: "100%",
                            textAlign: "left",
                            borderRadius: 12,
                            padding: "12px 14px",
                            paddingRight: 40,
                            cursor: "pointer",
                            background: isActive
                              ? `linear-gradient(135deg, ${accent}20 0%, rgba(255,255,255,0.04) 100%)`
                              : `linear-gradient(135deg, ${accent}0d 0%, transparent 70%)`,
                            border: `0.5px solid ${isActive ? accent + "44" : "rgba(255,255,255,0.07)"}`,
                          }}
                        >
                          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                            <span
                              style={{
                                width: 16, height: 16, borderRadius: "50%", flexShrink: 0,
                                background: m.status === "completed" ? `${accent}28` : "rgba(248,113,113,0.15)",
                                color: m.status === "completed" ? accent : "#f87171",
                                fontSize: 9, fontWeight: 700,
                                display: "flex", alignItems: "center", justifyContent: "center",
                              }}
                            >
                              {m.status === "completed" ? "✓" : "✕"}
                            </span>
                            <span style={{ fontSize: 10, fontFamily: "monospace", color: "rgba(255,255,255,0.25)", marginLeft: "auto" }}>
                              {formatDate(m.timestamp)}
                            </span>
                          </div>
                          <p
                            style={{
                              fontSize: "1.05rem", fontWeight: isActive ? 600 : 500, lineHeight: 1.4,
                              color: isActive ? "rgba(255,255,255,0.92)" : "rgba(255,255,255,0.65)",
                              display: "-webkit-box", WebkitLineClamp: 2,
                              WebkitBoxOrient: "vertical", overflow: "hidden",
                            }}
                          >
                            {m.intent}
                          </p>
                          <div style={{ marginTop: 8, display: "flex", alignItems: "flex-end", gap: 2, height: 14 }}>
                            {heights.map((h, i) => (
                              <span key={i} style={{ display: "inline-block", width: 3, height: h, borderRadius: 2, background: accent, opacity: 0.38 + i * 0.045 }} />
                            ))}
                            <span style={{ marginLeft: 4, fontSize: 9, fontFamily: "monospace", color: `${accent}90`, lineHeight: "13px" }}>{m.task_count}t</span>
                          </div>
                        </motion.button>

                        {/* Delete button — shown on hover via CSS group */}
                        <motion.button
                          onClick={(e: React.MouseEvent) => {
                            e.stopPropagation();
                            if (isConfirming) {
                              deleteMission(m.id);
                            } else {
                              setConfirmDeleteId(m.id);
                            }
                          }}
                          title={isConfirming ? "Click again to confirm" : "Delete mission"}
                          animate={{ opacity: isConfirming ? 1 : 0 }}
                          whileHover={{ opacity: 1 }}
                          transition={{ duration: 0.15 }}
                          style={{
                            position: "absolute", top: "50%", right: 10,
                            transform: "translateY(-50%)",
                            width: 26, height: 26, borderRadius: 7,
                            display: "flex", alignItems: "center", justifyContent: "center",
                            background: isConfirming ? "rgba(239,68,68,0.18)" : "rgba(255,255,255,0.07)",
                            border: `1px solid ${isConfirming ? "rgba(239,68,68,0.45)" : "rgba(255,255,255,0.10)"}`,
                            color: isConfirming ? "#f87171" : "rgba(255,255,255,0.40)",
                            fontSize: 12, cursor: "pointer",
                            transition: "background 0.15s, border-color 0.15s, color 0.15s",
                          }}
                        >
                          {isConfirming ? "✕" : "🗑"}
                        </motion.button>
                      </motion.div>
                    );
                  })}
                  </AnimatePresence>
                </div>
              )}
            </div>
          </motion.aside>
        )}
      </AnimatePresence>

      {/* ── Sidebar toggle button — vanishes when drawer is open ────────── */}
      <motion.button
        onClick={() => setSidebarOpen((v) => !v)}
        animate={{ opacity: sidebarOpen ? 0 : 1, scale: sidebarOpen ? 0.85 : 1 }}
        whileHover={{ scale: sidebarOpen ? 0.85 : 1.08 }}
        whileTap={{ scale: 0.92 }}
        transition={{ duration: 0.2 }}
        style={{
          position: "fixed",
          top: 20, left: 20,
          zIndex: 50,
          width: 40, height: 40,
          borderRadius: 12,
          background: "rgba(255,255,255,0.055)",
          border: "0.5px solid rgba(255,255,255,0.12)",
          backdropFilter: "blur(20px)",
          WebkitBackdropFilter: "blur(20px)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          color: "rgba(255,255,255,0.55)",
          fontSize: 15,
          cursor: "pointer",
          pointerEvents: sidebarOpen ? "none" : "auto",
        }}
      >
        ☰
      </motion.button>

      {/* ── Stage (scrolling content area) ───────────────────────────────── */}
      <div ref={stageRef} style={{ flex: 1, minHeight: 0, overflowY: "auto", overflowX: "hidden", position: "relative", zIndex: 1 }}>

        {/* Idle: in-flow centered hero header */}
        <AnimatePresence>
          {isIdle && (
            <motion.div
              key="idle-hero"
              initial={{ opacity: 0, y: -10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.97 }}
              transition={{ duration: 0.35 }}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                minHeight: "44vh",
                textAlign: "center",
                pointerEvents: "none",
              }}
            >
              <img
                src="/axion-logo.png"
                alt="Axion"
                style={{
                  width: 160, height: 160,
                  filter: "brightness(0) invert(1)",
                  mixBlendMode: "screen",
                  marginBottom: 16,
                }}
              />
              <h1
                style={{
                  fontSize: "clamp(3.4rem, 7.5vw, 5.2rem)",
                  fontWeight: 900,
                  letterSpacing: "-0.05em",
                  lineHeight: 1,
                  color: "rgba(255,255,255,0.94)",
                  marginBottom: 10,
                }}
              >
                AXION
              </h1>
              <p style={{ fontSize: 13, color: "rgba(255,255,255,0.28)", letterSpacing: "0.12em", textTransform: "uppercase", fontWeight: 500 }}>
                The Intelligence Kernel
              </p>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Active: in-flow header */}
        <AnimatePresence>
          {!isIdle && (
            <motion.header
              key="active-header"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.3 }}
              style={{ paddingTop: 56, paddingBottom: 20, display: "flex", alignItems: "center", justifyContent: "center", gap: 14 }}
            >
              <img
                src="/axion-logo.png"
                alt="Axion"
                style={{ width: 48, height: 48, filter: "invert(1)", mixBlendMode: "screen", flexShrink: 0 }}
              />
              <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: 2 }}>
                <h1
                  style={{
                    fontSize: "clamp(1.6rem, 3.5vw, 2.25rem)",
                    fontWeight: 900,
                    letterSpacing: "-0.045em",
                    lineHeight: 1,
                    background: "linear-gradient(135deg, rgba(255,255,255,0.95) 0%, rgba(200,210,255,0.80) 45%, rgba(255,255,255,0.92) 100%)",
                    WebkitBackgroundClip: "text",
                    WebkitTextFillColor: "transparent",
                    backgroundClip: "text",
                  }}
                >
                  AXION
                </h1>
                <p style={{ fontSize: 11, color: "rgba(255,255,255,0.28)", letterSpacing: "0.025em", fontStyle: "italic" }}>
                  The Intelligence Kernel.
                </p>
              </div>
            </motion.header>
          )}
        </AnimatePresence>

        {/* Idle: discovery widgets */}
        <AnimatePresence>
          {isIdle && (
            <motion.div
              key="idle-widgets"
              initial={{ opacity: 0, y: 24 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.96 }}
              transition={{ delay: 0.1, duration: 0.4, exit: { duration: 0.18 } }}
              style={{
                paddingTop: "3rem",
                paddingBottom: "2.5rem",
                paddingLeft: 24,
                paddingRight: 24,
              }}
            >
              <div style={{ maxWidth: 1040, margin: "0 auto", width: "100%" }}>

                {/* Quick-start cards */}
                <div style={{ display: "grid", gridTemplateColumns: "repeat(3, minmax(0, 1fr))", gap: 16, marginBottom: 16 }}>
                  {[
                    {
                      title: "Market Deep-Dive",
                      desc: "Research a stock, sector, or company in depth — financials, outlook, competitive landscape.",
                      accent: "#5eead4",
                      icon: "📈",
                      intent: "Research Tesla's current market position and financial outlook",
                    },
                    {
                      title: "Code Architect",
                      desc: "Design, review, or analyze any software system with multi-agent precision.",
                      accent: "#a78bfa",
                      icon: "⚙️",
                      intent: "Design a scalable microservice architecture for a fintech app",
                    },
                    {
                      title: "Research Synthesis",
                      desc: "Synthesize knowledge across a complex topic into a structured, cited report.",
                      accent: "#6ee7b7",
                      icon: "🔬",
                      intent: "Synthesize recent advances in quantum computing for 2025",
                    },
                  ].map((tpl) => (
                    <motion.button
                      key={tpl.title}
                      onClick={() => setIntent(tpl.intent)}
                      whileHover={{ y: -4, boxShadow: `inset 0 1px 0 rgba(255,255,255,0.22), 0 24px 64px rgba(0,0,0,0.55), 0 0 0 1px ${tpl.accent}33` }}
                      whileTap={{ scale: 0.98 }}
                      transition={{ type: "spring", stiffness: 320, damping: 28 }}
                      style={{
                        textAlign: "left",
                        borderRadius: 20,
                        padding: "28px 28px 32px",
                        background: "rgba(255,255,255,0.055)",
                        border: "1px solid rgba(255,255,255,0.11)",
                        backdropFilter: "blur(64px)",
                        WebkitBackdropFilter: "blur(64px)",
                        boxShadow: "inset 0 1px 0 rgba(255,255,255,0.14), 0 12px 40px rgba(0,0,0,0.40)",
                        cursor: "pointer",
                      }}
                    >
                      <div
                        style={{
                          display: "inline-flex", alignItems: "center", justifyContent: "center",
                          width: 48, height: 48, borderRadius: 14, marginBottom: 20,
                          background: `${tpl.accent}18`,
                          border: `1px solid ${tpl.accent}30`,
                          fontSize: 22,
                        }}
                      >
                        {tpl.icon}
                      </div>
                      <p style={{ fontSize: 17, fontWeight: 700, marginBottom: 8, color: "rgba(255,255,255,0.92)", letterSpacing: "-0.01em" }}>
                        {tpl.title}
                      </p>
                      <p style={{ fontSize: 13, lineHeight: 1.65, color: "rgba(255,255,255,0.48)" }}>
                        {tpl.desc}
                      </p>
                      <div style={{ marginTop: 20, display: "flex", alignItems: "center", gap: 6 }}>
                        <span style={{ width: 6, height: 6, borderRadius: "50%", background: tpl.accent, opacity: 0.7 }} />
                        <span style={{ fontSize: 11, color: tpl.accent, opacity: 0.7, letterSpacing: "0.04em", fontWeight: 500 }}>Launch mission →</span>
                      </div>
                    </motion.button>
                  ))}
                </div>

                {/* Bottom row: Kernel Status + Recent Themes */}
                <div style={{ display: "grid", gridTemplateColumns: "1fr 2fr", gap: 16 }}>

                  {/* Kernel Status */}
                  <div
                    style={{
                      borderRadius: 20,
                      background: "rgba(255,255,255,0.04)",
                      border: "1px solid rgba(255,255,255,0.09)",
                      backdropFilter: "blur(64px)",
                      WebkitBackdropFilter: "blur(64px)",
                      boxShadow: "inset 0 1px 0 rgba(255,255,255,0.10), 0 8px 24px rgba(0,0,0,0.30)",
                      padding: "24px",
                    }}
                  >
                    <p style={{ fontSize: 10, fontWeight: 600, letterSpacing: "0.12em", textTransform: "uppercase", color: "rgba(255,255,255,0.25)", marginBottom: 16 }}>
                      Kernel Status
                    </p>
                    <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 10 }}>
                      <span className="status-pulse-dot w-2 h-2 rounded-full shrink-0" style={{ background: "#34d399", color: "#34d399" }} />
                      <span style={{ fontSize: 13, color: "rgba(255,255,255,0.70)" }}>axion-core online</span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                      <span style={{ width: 8, height: 8, borderRadius: "50%", background: "rgba(255,255,255,0.18)", flexShrink: 0 }} />
                      <span style={{ fontSize: 13, color: "rgba(255,255,255,0.35)" }}>Awaiting mission</span>
                    </div>
                  </div>

                  {/* Recent Themes */}
                  {history.length > 0 && (() => {
                    const words = history
                      .flatMap((m) => m.intent.toLowerCase().split(/\W+/))
                      .filter((w) => w.length > 4)
                      .reduce((acc, w) => { acc[w] = (acc[w] ?? 0) + 1; return acc; }, {} as Record<string, number>);
                    const tags = Object.entries(words).sort((a, b) => b[1] - a[1]).slice(0, 8).map(([w]) => w);
                    return (
                      <div
                        style={{
                          borderRadius: 20,
                          background: "rgba(255,255,255,0.04)",
                          border: "1px solid rgba(255,255,255,0.09)",
                          backdropFilter: "blur(64px)",
                          WebkitBackdropFilter: "blur(64px)",
                          boxShadow: "inset 0 1px 0 rgba(255,255,255,0.10), 0 8px 24px rgba(0,0,0,0.30)",
                          padding: "24px",
                        }}
                      >
                        <p style={{ fontSize: 10, fontWeight: 600, letterSpacing: "0.12em", textTransform: "uppercase", color: "rgba(255,255,255,0.25)", marginBottom: 16 }}>
                          Recent Themes
                        </p>
                        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
                          {tags.map((tag) => (
                            <button
                              key={tag}
                              onClick={() => setIntent(tag.charAt(0).toUpperCase() + tag.slice(1))}
                              style={{
                                fontSize: 12, padding: "6px 14px", borderRadius: 999, fontWeight: 500,
                                background: "rgba(139,156,244,0.12)",
                                color: "rgba(139,156,244,0.80)",
                                border: "0.5px solid rgba(139,156,244,0.22)",
                                backdropFilter: "blur(20px)",
                                cursor: "pointer",
                              }}
                            >
                              {tag}
                            </button>
                          ))}
                        </div>
                      </div>
                    );
                  })()}
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* Active content */}
        <AnimatePresence>
          {!isIdle && (
            <motion.div
              key="active-content"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.25 }}
              style={{ padding: "0 24px 52px", maxWidth: 960, margin: "0 auto" }}
            >

              {/* Active agent indicator */}
              {isStreaming && activeAgent && (
                <div
                  className="flex items-center gap-3 rounded-xl px-4 py-3 mb-4"
                  style={{
                    background: "var(--axion-glass-bg, rgba(31,41,55,0.5))",
                    border: "1px solid var(--axion-glass-border, rgba(55,65,81,0.6))",
                    backdropFilter: "blur(var(--axion-blur, 24px))",
                    WebkitBackdropFilter: "blur(var(--axion-blur, 24px))",
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
                    <p className="text-xs font-semibold" style={{ color: "var(--axion-accent, #a5b4fc)" }}>
                      {activeAgent.role}
                    </p>
                    <p className="text-xs text-gray-400 truncate">{activeAgent.intent}</p>
                  </div>
                </div>
              )}

              {/* Initial spinner */}
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
                <div className="bg-indigo-950/70 border border-indigo-700 rounded-xl px-4 py-2.5 text-indigo-300 text-xs mb-4">
                  {governorBanner}
                </div>
              )}

              {/* Error */}
              {fetchError && missionStatus !== "streaming" && (
                <div className="bg-red-950 border border-red-700 rounded-xl p-4 text-red-300 text-sm mb-4">
                  ❌ {fetchError}
                </div>
              )}

              {/* Results section — visible during streaming AND after completion */}
              {hasCards && (
                <section className="space-y-5">

                  {/* Section header */}
                  <div className="flex items-center gap-3 flex-wrap">
                    <p className="text-xs font-semibold uppercase tracking-widest text-gray-500">
                      {missionStatus === "complete"
                        ? activeMissionId ? "Loaded from history" : "Mission complete"
                        : "Mission in progress"}{" "}
                      {missionMeta && `— ${missionMeta.task_count} task${missionMeta.task_count !== 1 ? "s" : ""}`}
                    </p>
                    {missionMeta?.expanded_task_count != null && missionMeta.expanded_task_count > 0 && (
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
                    <p className="text-xs text-gray-500 italic -mt-3">{missionMeta.intent}</p>
                  )}

                  {/* Dashboard */}
                  {missionState && (() => {
                    const blueprint = applicationMapper(
                      missionState.data_payload,
                      missionState.suggested_widgets ?? [],
                      missionState.design_tokens?.layout_strategy,
                    );
                    return blueprint.components.length > 0 ? (
                      <div className="space-y-4">
                        <div className="flex items-center justify-between">
                          <p className="text-sm font-semibold uppercase tracking-widest text-pink-400/80">
                            🧠 Synthesized State
                          </p>
                          <button
                            onClick={() => setShowDetails((v) => !v)}
                            className="text-xs text-gray-500 hover:text-gray-300 transition-colors"
                          >
                            {showDetails ? "Hide agent reasoning" : "Show agent reasoning"}
                          </button>
                        </div>
                        <div className={missionStatus === "complete" && missionState ? "axion-sheen-wrapper" : ""}>
                          <Renderer blueprint={blueprint} />
                        </div>

                        {/* Conflict badge — when Analyst flagged conflicting data */}
                        {Array.isArray((missionState.data_payload as Record<string,unknown>).data_conflicts) && (
                          <div style={{
                            marginTop: 16,
                            padding: "12px 16px",
                            borderRadius: 14,
                            background: "rgba(251,191,36,0.08)",
                            border: "1px solid rgba(251,191,36,0.28)",
                            backdropFilter: "blur(20px)",
                            display: "flex", alignItems: "flex-start", gap: 10,
                          }}>
                            <span style={{ fontSize: 15, flexShrink: 0 }}>⚠️</span>
                            <div>
                              <p style={{ fontSize: 12, fontWeight: 700, color: "rgba(251,191,36,0.90)", marginBottom: 4 }}>
                                Conflicting Data Detected
                              </p>
                              {((missionState.data_payload as Record<string,unknown>).data_conflicts as {field:string; values:string[]; sources:string[]}[]).map((c, i) => (
                                <p key={i} style={{ fontSize: 11, color: "rgba(255,255,255,0.55)", marginBottom: 2 }}>
                                  <strong style={{ color: "rgba(255,255,255,0.75)" }}>{c.field}:</strong>{" "}
                                  {c.values.join(" vs ")} — {c.sources.join(", ")}
                                </p>
                              ))}
                            </div>
                          </div>
                        )}

                        {/* Action Bar — completed missions only */}
                        {missionStatus === "complete" && (
                          <ActionBar
                            intent={missionMeta?.intent ?? intent}
                            missionState={missionState}
                            onNewMission={(newIntent) => { setIntent(newIntent); setSidebarOpen(false); setTimeout(() => textareaRef.current?.focus(), 80); }}
                          />
                        )}
                      </div>
                    ) : null;
                  })()}

                  {/* Raw agent cards */}
                  {(!missionState || showDetails) && (() => {
                    const visibleSlugs = cardOrder.filter((slug) => {
                      const result = streamCards[slug]?.result;
                      if (!result) return true;
                      try {
                        const parsed = JSON.parse(result);
                        return !(parsed && typeof parsed.data_payload === "object" && parsed.data_payload !== null);
                      } catch {
                        return true;
                      }
                    });

                    if (visibleSlugs.length === 0) return null;

                    return (
                      <div className="space-y-4">
                        {missionState && (
                          <p className="text-xs font-semibold uppercase tracking-widest text-gray-600">
                            Agent Reasoning
                          </p>
                        )}
                        {visibleSlugs.map((slug) => {
                          const card = streamCards[slug];
                          if (!card) return null;
                          const { label, icon, accent } = cardMeta(slug, card.role);
                          const isRunning = card.status === "running";

                          return (
                            <motion.article
                              key={slug}
                              layout
                              initial={{ opacity: 0, y: 14 }}
                              animate={{ opacity: 1, y: 0 }}
                              transition={{ duration: 0.28 }}
                              className={`rounded-2xl ${isRunning ? "axion-neural-pulse" : ""}`}
                              style={{
                                background: isRunning
                                  ? "rgba(255,255,255,0.04)"
                                  : "var(--axion-glass-bg, rgba(255,255,255,0.04))",
                                border: isRunning
                                  ? `1px solid rgba(var(--axion-accent-rgb, 139,156,244), 0.40)`
                                  : "0.5px solid var(--axion-glass-border, rgba(255,255,255,0.10))",
                                backdropFilter: "blur(var(--axion-blur, 56px))",
                                WebkitBackdropFilter: "blur(var(--axion-blur, 56px))",
                                padding: "var(--axion-pad, 20px)",
                                boxShadow: isRunning
                                  ? "inset 0 1px 0 rgba(255,255,255,0.10)"
                                  : "inset 0 1px 0 rgba(255,255,255,0.08)",
                              }}
                            >
                              <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-200 mb-3">
                                <span>{icon}</span>
                                <span>{label}</span>
                                {isRunning && (
                                  <span className="ml-auto flex items-center gap-1.5 text-xs font-normal"
                                    style={{ color: "var(--axion-accent, #a5b4fc)" }}>
                                    <span className="w-2 h-2 rounded-full inline-block"
                                      style={{ background: "var(--axion-accent, #a5b4fc)", animation: "neural-pulse 1.4s ease-in-out infinite" }} />
                                    Processing…
                                  </span>
                                )}
                                {card.status === "failed" && (
                                  <span className="ml-auto text-xs text-red-400 font-normal">✗ Failed</span>
                                )}
                              </h2>

                              {isRunning ? (
                                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                                  {[1, 0.55, 0.3].map((op, i) => (
                                    <div key={i} className="animate-pulse" style={{
                                      height: 10, borderRadius: 6,
                                      background: `rgba(255,255,255,${op * 0.08})`,
                                      width: i === 2 ? "55%" : "100%",
                                    }} />
                                  ))}
                                </div>
                              ) : showDetails && card.result ? (
                                <ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents}>
                                  {card.result}
                                </ReactMarkdown>
                              ) : null}
                            </motion.article>
                          );
                        })}
                      </div>
                    );
                  })()}

                </section>
              )}
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      {/* ── Control Zone — anchored, never overlaps Stage ─────────────────── */}
      <div
        style={{
          flexShrink: 0,
          position: "relative",
          zIndex: 10,
          background: "rgba(7,9,12,1)",
        }}
      >
        {/* Gradient fade — bleeds upward into the Stage so last card dissolves softly */}
        <div
          aria-hidden="true"
          style={{
            position: "absolute",
            bottom: "100%",
            left: 0, right: 0,
            height: 72,
            background: "linear-gradient(to bottom, transparent, rgba(7,9,12,0.94))",
            pointerEvents: "none",
          }}
        />
        <div
          style={{
            display: "flex",
            justifyContent: "center",
            padding: "6px 24px 22px",
          }}
        >
          <div style={{ width: "100%", maxWidth: 720 }}>
            {commandBarContent}
          </div>
        </div>
      </div>

    </div>
  );
}
