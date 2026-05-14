"use client";

import { useState, useEffect, useCallback, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Renderer, UIBlueprint, UIComponent } from "@/components/Renderer";
import { ActionBar } from "@/components/ActionBar";
import { TemplateGallery } from "@/components/TemplateGallery";
import { AxionClient } from "@axion/sdk";
import type {
  MissionEvent,
  TaskStartedEvent,
  TaskCompletedEvent,
  TaskFailedEvent,
  GovernorExpandEvent,
  MissionCompleteEvent,
  MissionFailedEvent,
  AwaitingFeedbackEvent,
} from "@axion/sdk";

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

// ── Slash command discovery ───────────────────────────────────────────────────

interface SlashCommand {
  command: string;      // the full slash command text, e.g. "/export md"
  description: string;  // shown in the discovery popover
  category: "export" | "mission" | "memo";
}

// ── Execution trace ───────────────────────────────────────────────────────────

interface TraceEvent {
  id: string;
  timestamp: number;
  type: string;
  slug?: string;
  role?: string;
  label: string;
  durationMs?: number;
}

// ── Refinement history ────────────────────────────────────────────────────────

interface RefinementRound {
  intent: string;
  timestamp: number;
  /** data_payload keys that were added or updated by this round. */
  newPayloadKeys: string[];
}

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
    case "Synthesized": return "#a7cadc";
    case "Analytical":  return "#6ee7b7";
    case "Itinerary":   return "#fbbf24";
    default:            return "#6b7280";
  }
}

// Returns a human-readable relative time string for a Unix-ms timestamp
// e.g. "just now", "2 min ago", "1 hr ago"
function timeAgo(timestamp: number): string {
  const secs = Math.floor((Date.now() - timestamp) / 1000);
  if (secs < 60)   return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60)   return `${mins} min ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24)    return `${hrs} hr ago`;
  const days = Math.floor(hrs / 24);
  return `${days} day${days !== 1 ? "s" : ""} ago`;
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
  // Landing-page range: 28–40 px
  const blur = Math.round(28 + gi * 12);
  const so = Math.max(0, Math.min(1, tokens.surface_opacity ?? 0.04));
  const bgAlpha = Math.min(so + gi * 0.02, 0.06).toFixed(3);
  const borderAlpha = (0.07 + gi * 0.04).toFixed(3);
  const compact = tokens.layout_density === "compact";
  const radius = Math.min(tokens.border_radius ?? 14, 22);

  // Determine foreground colour for accent-background buttons (WCAG contrast)
  // Relative luminance of accent: 0.2126R + 0.7152G + 0.0722B (linearised)
  const toLinear = (c: number) => c / 255 <= 0.04045 ? c / 255 / 12.92 : ((c / 255 + 0.055) / 1.055) ** 2.4;
  const lum = 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
  const accentFg = lum > 0.35 ? "#07090c" : "#f0f4f8";

  root.style.setProperty("--axion-accent",       tokens.primary_accent);
  root.style.setProperty("--axion-accent-rgb",   `${r},${g},${b}`);
  root.style.setProperty("--axion-accent-fg",    accentFg);
  root.style.setProperty("--axion-blur",         `${blur}px`);
  root.style.setProperty("--axion-glass-bg",     `rgba(255,255,255,${bgAlpha})`);
  root.style.setProperty("--axion-glass-border", `rgba(255,255,255,${borderAlpha})`);
  root.style.setProperty("--axion-glass-inset",  `0 1px 0 0 rgba(255,255,255,0.06) inset, 0 0 0 1px rgba(255,255,255,0.05), 0 40px 80px -30px rgba(0,0,0,0.70)`);
  root.style.setProperty("--axion-glow",         `rgba(${r},${g},${b},0.35)`);
  root.style.setProperty("--axion-pad",          compact ? "16px" : "24px");
  root.style.setProperty("--axion-gap",          compact ? "20px" : "36px");
  root.style.setProperty("--axion-radius",       `${radius}px`);
}

function resetDesignTokens() {
  const props = [
    "--axion-accent", "--axion-accent-rgb", "--axion-accent-fg", "--axion-blur",
    "--axion-glass-bg", "--axion-glass-border", "--axion-glass-inset", "--axion-glow",
    "--axion-pad", "--axion-gap", "--axion-radius",
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
  /** Tracks the animated (partially-revealed) portion of each completed task result. */
  const [displayedResults, setDisplayedResults] = useState<Record<string, string>>({});
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
  const [slashQuery, setSlashQuery] = useState<string | null>(null); // null = closed
  const [slashIndex, setSlashIndex] = useState(0);
  // Discovery popover state (separate from execution state above)
  const [slashPopoverOpen, setSlashPopoverOpen] = useState(false);
  const [slashPopoverIndex, setSlashPopoverIndex] = useState(0);
  const [slashFilter, setSlashFilter] = useState("");
  const [pinnedCards, setPinnedCards] = useState<Set<number>>(new Set());
  const [dismissedCards, setDismissedCards] = useState<Set<number>>(new Set());
  const [refinementHistory, setRefinementHistory] = useState<RefinementRound[]>([]);
  const [isRefining, setIsRefining] = useState(false);
  const [refineError, setRefineError] = useState<string | null>(null);

  // ── Image upload ───────────────────────────────────────────────────────────
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [uploadedFile, setUploadedFile] = useState<{
    filename: string;
    originalName: string;
    fileType: "image" | "data";
    previewUrl: string | null;
  } | null>(null);
  const [isUploading, setIsUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);

  // ── Settings ────────────────────────────────────────────────────────────────
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [configStatus, setConfigStatus] = useState<{ openai: boolean; tavily: boolean } | null>(null);
  const [draftAxionKey, setDraftAxionKey] = useState("");
  const [draftOpenAI, setDraftOpenAI] = useState("");
  const [draftTavily, setDraftTavily] = useState("");
  const [settingsSaved, setSettingsSaved] = useState(false);
  const [notifPermission, setNotifPermission] = useState<NotificationPermission | "unsupported">(
    typeof window !== "undefined" && "Notification" in window
      ? Notification.permission
      : "unsupported"
  );
  /** Keys in data_payload that came from the most recent refinement pass. */
  const [refinedPayloadKeys, setRefinedPayloadKeys] = useState<Set<string>>(new Set());
  const [showRefinementHistory, setShowRefinementHistory] = useState(false);

  // ── Execution trace ────────────────────────────────────────────────────────
  const [traceEvents, setTraceEvents] = useState<TraceEvent[]>([]);
  const [traceOpen, setTraceOpen] = useState(false);
  const taskStartTimes = useRef<Record<string, number>>({});

  // ── History search & clear ─────────────────────────────────────────────────
  const [historyQuery, setHistoryQuery] = useState("");
  const [historyQueryFocused, setHistoryQueryFocused] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  async function deleteMission(id: string) {
    try {
      await buildClient().missions.delete(id);
    } catch { /* ignore if server is down */ }
    setHistory((prev) => prev.filter((m) => m.id !== id));
    if (activeMissionId === id) newMission();
    setConfirmDeleteId(null);
  }

  async function clearAllHistory() {
    const client = buildClient();
    await Promise.all(
      history.map((m) => client.missions.delete(m.id).catch(() => {}))
    );
    setHistory([]);
    setHistoryQuery("");
    setConfirmClear(false);
    if (activeMissionId) newMission();
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
    setPinnedCards(new Set());
    setDismissedCards(new Set());
    setRefinementHistory([]);
    setIsRefining(false);
    setRefineError(null);
    setRefinedPayloadKeys(new Set());
    setShowRefinementHistory(false);
    setDisplayedResults({});
    setHistoryQuery("");
    setConfirmClear(false);
    setSlashPopoverOpen(false);
    setSlashFilter("");
    setSlashPopoverIndex(0);
    resetDesignTokens();
    setTimeout(() => textareaRef.current?.focus(), 100);
  }

  const fetchHistory = useCallback(async () => {
    try {
      const missions = await buildClient().missions.list();
      setHistory(missions as MissionSummary[]);
    } catch {
      // Server may not be running yet — ignore.
    }
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

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

  // Drive typewriter animation for each newly-completed task result.
  useEffect(() => {
    Object.entries(streamCards).forEach(([slug, card]) => {
      if (card.status !== "completed" || !card.result) return;
      // Already fully revealed — nothing to do
      if (displayedResults[slug] === card.result) return;
      // Already mid-animation or at full length — skip
      const alreadyShown = displayedResults[slug] ?? "";
      if (alreadyShown.length >= card.result.length) return;

      let i = alreadyShown.length;
      const fullText = card.result;
      const CHARS_PER_TICK = 3;
      const INTERVAL_MS   = 16; // ~60 fps

      const timer = setInterval(() => {
        i += CHARS_PER_TICK;
        if (i >= fullText.length) {
          setDisplayedResults(prev => ({ ...prev, [slug]: fullText }));
          clearInterval(timer);
        } else {
          setDisplayedResults(prev => ({ ...prev, [slug]: fullText.slice(0, i) }));
        }
      }, INTERVAL_MS);

      // Cleanup note: return inside forEach is a no-op for useEffect cleanup.
      // The timers are self-terminating via the i >= fullText.length guard.
      return () => clearInterval(timer);
    });
  }, [streamCards]); // eslint-disable-line react-hooks/exhaustive-deps

  // Fetch /config/status on mount, hydrate draft keys from localStorage, and
  // auto-open the settings drawer when no OpenAI key is configured anywhere.
  useEffect(() => {
    function hydrate(openai: boolean) {
      const storedAxionKey = localStorage.getItem("axion_api_key") ?? "";
      const storedOpenAI = localStorage.getItem("axion_openai_key") ?? "";
      const storedTavily = localStorage.getItem("axion_tavily_key") ?? "";
      if (storedAxionKey) setDraftAxionKey(storedAxionKey);
      if (storedOpenAI) setDraftOpenAI(storedOpenAI);
      if (storedTavily) setDraftTavily(storedTavily);
      if (!openai && !storedOpenAI) setSettingsOpen(true);
    }

    buildClient().configStatus()
      .then((status) => {
        setConfigStatus(status);
        hydrate(status.openai);
      })
      .catch(() => {
        // Server offline on first load — rely solely on localStorage.
        hydrate(false);
      });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  async function loadMission(id: string) {
    if (missionStatus === "streaming") return;
    setActiveMissionId(id);
    setFetchError(null);
    setGovernorBanner(null);
    setMissionState(null);
    setShowDetails(false);

    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const data = await buildClient().missions.get(id) as any;

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

      // Reconstruct a minimal trace from the task slugs.
      {
        const syntheticEvents: TraceEvent[] = [];
        order.forEach((slug, i) => {
          // Derive a readable role name from the slug, e.g. "web_search_0" → "Web Search"
          const parts = slug.split("_");
          const roleParts = parts[parts.length - 1].match(/^\d+$/) ? parts.slice(0, -1) : parts;
          const roleLabel = roleParts.map((p) => p.charAt(0).toUpperCase() + p.slice(1)).join(" ");
          syntheticEvents.push({
            id: `reconstructed-${slug}-start`,
            timestamp: i * 2,       // synthetic ordering
            type: "task_started",
            slug,
            role: roleLabel,
            label: `◷ ${roleLabel}: ${slug}`,
          });
          syntheticEvents.push({
            id: `reconstructed-${slug}-done`,
            timestamp: i * 2 + 1,
            type: "task_completed",
            slug,
            role: roleLabel,
            label: `◷ ${roleLabel}: ${slug}`,
          });
        });
        // Add a final mission_complete marker
        syntheticEvents.push({
          id: "reconstructed-mission-complete",
          timestamp: order.length * 2,
          type: "mission_complete",
          label: "◉ Mission complete",
        });
        setTraceEvents(syntheticEvents);
        setTraceOpen(false);
        taskStartTimes.current = {};
      }

      // Rebuild refinement history from the snapshot intent chain (best-effort).
      {
        const raw: string = (data.intent as string) ?? "";
        const rounds: RefinementRound[] = [];
        const refineMatch = raw.match(/^REFINE\[([^\]]+)\]:\s*(.+)$/);
        if (refineMatch) {
          rounds.push({ intent: refineMatch[2], timestamp: (data.timestamp as number) * 1000, newPayloadKeys: [] });
        }
        setRefinementHistory(rounds);
      }
      setRefinedPayloadKeys(new Set());
      setMissionStatus("complete");
      setActiveAgent(null);
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Could not load mission.");
    }
  }

  // ── Browser notifications ──────────────────────────────────────────────────

  function showMissionNotification(status: "complete" | "failed", intentText: string) {
    if (typeof window === "undefined") return;
    if (!("Notification" in window)) return;
    if (Notification.permission !== "granted") return;
    if (document.visibilityState === "visible") return; // tab is focused — skip

    const title = status === "complete" ? "Mission complete" : "Mission failed";
    const body  = intentText.length > 80 ? intentText.slice(0, 77) + "…" : intentText;
    const icon  = "/axion-logo.png";

    try {
      const n = new Notification(title, { body, icon });
      setTimeout(() => n.close(), 6000);
      n.onclick = () => { window.focus(); n.close(); };
    } catch {
      // OS may block even with permission granted — ignore silently
    }
  }

  /** Builds a fully-configured AxionClient from current localStorage keys. */
  function buildClient(): AxionClient {
    return new AxionClient({
      baseUrl:   "http://localhost:8080",
      apiKey:    localStorage.getItem("axion_api_key")    ?? undefined,
      openAiKey: localStorage.getItem("axion_openai_key") ?? undefined,
      tavilyKey: localStorage.getItem("axion_tavily_key") ?? undefined,
    });
  }

  async function runMission() {
    if (!intent.trim() || missionStatus === "streaming") return;

    // Request notification permission on first Execute — fire-and-forget, non-blocking
    if (typeof window !== "undefined" && "Notification" in window
        && Notification.permission === "default") {
      Notification.requestPermission().then((perm) => setNotifPermission(perm));
    }

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
    setTraceEvents([]);
    setTraceOpen(false);
    taskStartTimes.current = {};

    function handleSSEEvent(event: MissionEvent) {
      const now = Date.now();
      switch (event.type) {
        case "task_started": {
          const e = event as TaskStartedEvent;
          setStreamCards((prev) => ({
            ...prev,
            [e.slug]: { slug: e.slug, role: e.role, intent: e.intent, status: "running" },
          }));
          setCardOrder((prev) => (prev.includes(e.slug) ? prev : [...prev, e.slug]));
          setActiveAgent({ role: e.role, intent: e.intent });
          taskStartTimes.current[e.slug] = now;
          setTraceEvents((prev) => [...prev, {
            id: `${now}-${e.slug}`,
            timestamp: now,
            type: "task_started",
            slug: e.slug,
            role: e.role,
            label: `▶ ${e.role} started: ${e.intent.slice(0, 48)}`,
          }]);
          break;
        }
        case "task_completed": {
          const e = event as TaskCompletedEvent;
          const durationMs = now - (taskStartTimes.current[e.slug] ?? now);
          setStreamCards((prev) => ({
            ...prev,
            [e.slug]: { ...prev[e.slug], slug: e.slug, role: e.role, status: "completed", result: e.result },
          }));
          setActiveAgent(null);
          setTraceEvents((prev) => [...prev, {
            id: `${now}-${e.slug}-done`,
            timestamp: now,
            type: "task_completed",
            slug: e.slug,
            role: e.role,
            label: `✓ ${e.role} completed in ${durationMs}ms`,
            durationMs,
          }]);
          break;
        }
        case "task_failed": {
          const e = event as TaskFailedEvent;
          setStreamCards((prev) => ({ ...prev, [e.slug]: { ...prev[e.slug], status: "failed" } }));
          setActiveAgent(null);
          setTraceEvents((prev) => [...prev, {
            id: `${now}-${e.slug}-fail`,
            timestamp: now,
            type: "task_failed",
            slug: e.slug,
            role: e.role,
            label: `✗ ${e.role ?? e.slug} failed`,
          }]);
          break;
        }
        case "governor_expand": {
          const e = event as GovernorExpandEvent;
          setGovernorBanner(`🔭 Governor expanding mission with ${e.new_task_count} new task(s)`);
          setTimeout(() => setGovernorBanner(null), 6000);
          setTraceEvents((prev) => [...prev, {
            id: `${now}-governor`,
            timestamp: now,
            type: "governor_expand",
            label: `· governor_expand (+${e.new_task_count} tasks)`,
          }]);
          break;
        }
        case "mission_complete": {
          const e = event as MissionCompleteEvent;
          setMissionMeta({
            task_count: e.task_count,
            expanded_task_count: e.expanded_task_count,
            mission_id: e.mission_id,
            intent: e.intent,
            layout_hint: e.layout_hint,
          });
          if (e.mission_state?.data_payload) {
            setMissionState(e.mission_state as unknown as MissionState);
            setShowDetails(false);
          } else {
            setShowDetails(true);
          }
          setMissionStatus("complete");
          showMissionNotification("complete", intent);
          setActiveAgent(null);
          fetchHistory();
          setTraceEvents((prev) => [...prev, {
            id: `${now}-mission-complete`,
            timestamp: now,
            type: "mission_complete",
            label: "◉ Mission complete",
          }]);
          break;
        }
        case "mission_failed": {
          const e = event as MissionFailedEvent;
          setFetchError(e.error);
          setMissionStatus("failed");
          showMissionNotification("failed", intent);
          setActiveAgent(null);
          setTraceEvents((prev) => [...prev, {
            id: `${now}-mission-failed`,
            timestamp: now,
            type: "mission_failed",
            label: "◉ Mission failed",
          }]);
          break;
        }
        default:
          setTraceEvents((prev) => [...prev, {
            id: `${now}-${event.type}`,
            timestamp: now,
            type: event.type as string,
            label: `· ${event.type}`,
          }]);
      }
    }

    // Inject vision / data-file tool hints when an attachment is present
    const effectiveIntent = uploadedFile
      ? uploadedFile.fileType === "image"
        ? `${intent}\n\n[Image attached: ${uploadedFile.filename}. Use the vision tool to analyse this image as part of your research.]`
        : `${intent}\n\n[Data file attached: ${uploadedFile.filename} (original: ${uploadedFile.originalName}). Use the read_file tool with filename="${uploadedFile.filename}" to load its contents before analysis.]`
      : intent;

    // Snapshot the attachment so we can release it on the first event (server accepted)
    const pendingAttachment = uploadedFile;
    let attachmentReleased = false;

    try {
      for await (const event of buildClient().execute(effectiveIntent)) {
        // Release the attachment chip as soon as the first event arrives (server accepted)
        if (!attachmentReleased && pendingAttachment) {
          attachmentReleased = true;
          if (pendingAttachment.previewUrl) URL.revokeObjectURL(pendingAttachment.previewUrl);
          setUploadedFile(null);
        }
        handleSSEEvent(event);
      }

      setMissionStatus((prev) => {
        if (prev === "streaming") { showMissionNotification("failed", intent); return "failed"; }
        return prev;
      });
    } catch (e) {
      setFetchError(e instanceof Error ? e.message : "Connection error.");
      setMissionStatus("failed");
      showMissionNotification("failed", intent);
    }
  }

  // ── Image upload ──────────────────────────────────────────────────────────────

  async function handleFileUpload(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    if (!file) return;

    // Client-side type guard — allow images + data files
    const isImage = file.type.startsWith("image/");
    const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
    const isData = ["csv", "json", "txt"].includes(ext) ||
      ["text/csv", "application/json", "text/plain"].includes(file.type);

    if (!isImage && !isData) {
      setUploadError("Accepted: images, CSV, JSON, TXT");
      setTimeout(() => setUploadError(null), 3000);
      if (fileInputRef.current) fileInputRef.current.value = "";
      return;
    }

    setIsUploading(true);
    setUploadError(null);

    try {
      const result = await buildClient().upload(file);
      const previewUrl = result.file_type === "image" ? URL.createObjectURL(file) : null;
      setUploadedFile({
        filename: result.filename,
        originalName: result.original_name ?? file.name,
        fileType: result.file_type,
        previewUrl,
      });
    } catch {
      setUploadError("Upload failed — try again");
    }

    setIsUploading(false);
    // Reset so the same file can be selected again
    if (fileInputRef.current) fileInputRef.current.value = "";
  }

  // ── Refinement ────────────────────────────────────────────────────────────────

  async function runRefinement(refinementIntent: string) {
    const targetId = activeMissionId ?? missionMeta?.mission_id;
    if (!targetId || isRefining || missionStatus === "streaming") return;

    setIsRefining(true);
    setRefineError(null);

    // Capture the payload keys that exist BEFORE the refinement so we can diff.
    const keysBefore = new Set(Object.keys(missionState?.data_payload ?? {}));

    try {
      for await (const event of buildClient().missions.refine(targetId, refinementIntent)) {
        const now = Date.now();
        switch (event.type) {
          case "task_started": {
            const e = event as TaskStartedEvent;
            setStreamCards((prev) => ({
              ...prev,
              [e.slug]: { slug: e.slug, role: e.role, intent: e.intent, status: "running" },
            }));
            setCardOrder((prev) => (prev.includes(e.slug) ? prev : [...prev, e.slug]));
            setActiveAgent({ role: e.role, intent: e.intent });
            taskStartTimes.current[e.slug] = now;
            setTraceEvents((prev) => [...prev, {
              id: `${now}-${e.slug}`,
              timestamp: now,
              type: "task_started",
              slug: e.slug,
              role: e.role,
              label: `▶ ${e.role} started: ${e.intent.slice(0, 48)}`,
            }]);
            break;
          }
          case "task_completed": {
            const e = event as TaskCompletedEvent;
            const durationMs = now - (taskStartTimes.current[e.slug] ?? now);
            setStreamCards((prev) => ({
              ...prev,
              [e.slug]: { ...prev[e.slug], slug: e.slug, role: e.role, status: "completed", result: e.result },
            }));
            setActiveAgent(null);
            setTraceEvents((prev) => [...prev, {
              id: `${now}-${e.slug}-done`,
              timestamp: now,
              type: "task_completed",
              slug: e.slug,
              role: e.role,
              label: `✓ ${e.role} completed in ${durationMs}ms`,
              durationMs,
            }]);
            break;
          }
          case "task_failed": {
            const e = event as TaskFailedEvent;
            setStreamCards((prev) => ({ ...prev, [e.slug]: { ...prev[e.slug], status: "failed" } }));
            setActiveAgent(null);
            setTraceEvents((prev) => [...prev, {
              id: `${now}-${e.slug}-fail`,
              timestamp: now,
              type: "task_failed",
              slug: e.slug,
              role: e.role,
              label: `✗ ${e.role ?? e.slug} failed`,
            }]);
            break;
          }
          case "awaiting_feedback": {
            const _e = event as AwaitingFeedbackEvent;
            setTraceEvents((prev) => [...prev, {
              id: `${now}-feedback`,
              timestamp: now,
              type: "awaiting_feedback",
              label: "⏸ Awaiting human input",
            }]);
            break;
          }
          case "mission_complete": {
            const e = event as MissionCompleteEvent;
            const newMissionState = e.mission_state as (typeof missionState) | undefined;
            if (newMissionState?.data_payload) {
              const keysAfter = new Set(Object.keys(newMissionState.data_payload));
              const newKeys   = new Set([...keysAfter].filter(k => !keysBefore.has(k)));
              setRefinedPayloadKeys(newKeys);
              setMissionState(newMissionState);
              setRefinementHistory((prev) => [
                ...prev,
                { intent: refinementIntent, timestamp: now, newPayloadKeys: [...newKeys] },
              ]);
            }
            setMissionStatus("complete");
            showMissionNotification("complete", refinementIntent);
            setMissionMeta((prev) => prev
              ? { ...prev, task_count: e.task_count ?? prev.task_count }
              : prev
            );
            setActiveAgent(null);
            fetchHistory();
            setTraceEvents((prev) => [...prev, {
              id: `${now}-mission-complete`,
              timestamp: now,
              type: "mission_complete",
              label: "◉ Mission complete",
            }]);
            break;
          }
          case "mission_failed": {
            const e = event as MissionFailedEvent;
            setRefineError(e.error ?? "Refinement failed.");
            showMissionNotification("failed", refinementIntent);
            setActiveAgent(null);
            setTraceEvents((prev) => [...prev, {
              id: `${now}-mission-failed`,
              timestamp: now,
              type: "mission_failed",
              label: "◉ Mission failed",
            }]);
            break;
          }
          default:
            setTraceEvents((prev) => [...prev, {
              id: `${now}-${event.type}`,
              timestamp: now,
              type: event.type as string,
              label: `· ${event.type}`,
            }]);
        }
      }
    } catch (e) {
      setRefineError(e instanceof Error ? e.message : "Connection error.");
    } finally {
      setIsRefining(false);
    }
  }

  const isIdle = missionStatus === "idle";
  const isStreaming = missionStatus === "streaming";
  const hasCards = cardOrder.length > 0;

  // ── Trace helpers ─────────────────────────────────────────────────────────

  function formatTraceTime(ts: number): string {
    const d = new Date(ts);
    const h = String(d.getHours()).padStart(2, "0");
    const m = String(d.getMinutes()).padStart(2, "0");
    const s = String(d.getSeconds()).padStart(2, "0");
    return `${h}:${m}:${s}`;
  }

  function dotColorForType(type: string): string {
    if (type === "task_completed")   return "rgba(74,222,128,0.7)";
    if (type === "task_failed")      return "rgba(248,113,113,0.7)";
    if (type === "mission_complete") return "var(--axion-accent, #a7cadc)";
    if (type === "task_started")     return "rgba(255,255,255,0.25)";
    return "rgba(255,255,255,0.15)";
  }

  // Instant-filter: matches intent substring (case-insensitive)
  const filteredHistory = historyQuery.trim()
    ? history.filter((m) =>
        m.intent.toLowerCase().includes(historyQuery.trim().toLowerCase())
      )
    : history;

  // ── Slash commands ────────────────────────────────────────────────────────────
  const SLASH_COMMANDS = [
    { cmd: "/export md",   icon: "↓", desc: "Download mission results as Markdown",  accent: "#6ee7b7", category: "export"   as const,
      run: () => { const id = activeMissionId ?? missionMeta?.mission_id; if (id) { buildClient().missions.export(id, "md").then(b=>{const u=URL.createObjectURL(b);const a=document.createElement("a");a.href=u;a.download=`axion-${id}.md`;a.click();URL.revokeObjectURL(u);}); } } },
    { cmd: "/export csv",  icon: "↓", desc: "Download mission results as CSV",        accent: "#6ee7b7", category: "export"   as const,
      run: () => { const id = activeMissionId ?? missionMeta?.mission_id; if (id) { buildClient().missions.export(id, "csv").then(b=>{const u=URL.createObjectURL(b);const a=document.createElement("a");a.href=u;a.download=`axion-${id}.csv`;a.click();URL.revokeObjectURL(u);}); } } },
    { cmd: "/export html", icon: "↓", desc: "Download mission results as HTML page",  accent: "#6ee7b7", category: "export"   as const,
      run: () => { const id = activeMissionId ?? missionMeta?.mission_id; if (id) { buildClient().missions.export(id, "html").then(b=>{const u=URL.createObjectURL(b);const a=document.createElement("a");a.href=u;a.download=`axion-${id}.html`;a.click();URL.revokeObjectURL(u);}); } } },
    { cmd: "/clear",       icon: "✕", desc: "Clear the current mission and start fresh", accent: "#f87171", category: "mission" as const,
      run: () => { newMission(); } },
    { cmd: "/memo",        icon: "◈", desc: "Save a note about this mission",         accent: "#a7cadc", category: "memo"    as const,
      run: () => { const note = intent.replace(/^\/memo\s*/i, "").trim() || "No content"; void (async () => { for await (const _ of buildClient().execute(`Save a memo: "${note}"`)) { break; } })(); newMission(); } },
  ];

  const filteredSlash = slashQuery !== null
    ? SLASH_COMMANDS.filter((c) => c.cmd.includes(slashQuery.toLowerCase()))
    : [];

  // Discovery popover: show all commands when only "/" typed, filter when longer
  const filteredCommands = slashFilter.length > 1
    ? SLASH_COMMANDS.filter((c) =>
        c.cmd.toLowerCase().includes(slashFilter.toLowerCase()) ||
        c.desc.toLowerCase().includes(slashFilter.slice(1).toLowerCase())
      )
    : SLASH_COMMANDS;

  function executeSlashCommand(cmd: typeof SLASH_COMMANDS[0]) {
    setIntent("");
    setSlashQuery(null);
    setSlashIndex(0);
    cmd.run();
  }

  /** Insert a command from the discovery popover — arms slashQuery so Enter executes. */
  function selectSlashCommand(cmd: typeof SLASH_COMMANDS[0]) {
    setIntent(cmd.cmd);
    setSlashPopoverOpen(false);
    setSlashFilter("");
    setSlashPopoverIndex(0);
    // Prime the execution state: next Enter will run the command via existing handler
    setSlashQuery(cmd.cmd.slice(1));
    setSlashIndex(0);
    setTimeout(() => textareaRef.current?.focus(), 20);
  }

  // ── Shared command bar ───────────────────────────────────────────────────────
  // Always lives in the Control Zone at the bottom — never floats/overlaps.
  const commandBarContent = (
    <div style={{ position: "relative" }}>
    {/* ── Slash-command discovery popover ──────────────────────────────── */}
    <AnimatePresence>
      {slashPopoverOpen && filteredCommands.length > 0 && (
        <motion.div
          key="slash-discovery"
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 6 }}
          transition={{ duration: 0.15 }}
          style={{
            position: "absolute",
            bottom: "calc(100% + 8px)",
            left: 0, right: 0,
            background: "rgba(14,14,22,0.97)",
            border: "1px solid rgba(255,255,255,0.12)",
            borderRadius: 14,
            backdropFilter: "blur(40px)",
            WebkitBackdropFilter: "blur(40px)",
            overflow: "hidden",
            zIndex: 300,
            boxShadow: "0 16px 48px rgba(0,0,0,0.65)",
          }}
        >
          {/* Header */}
          <div style={{
            padding: "8px 14px 6px",
            fontSize: 10,
            color: "rgba(255,255,255,0.25)",
            letterSpacing: "0.08em",
            textTransform: "uppercase",
            borderBottom: "1px solid rgba(255,255,255,0.06)",
          }}>
            Commands
          </div>

          {/* Command rows */}
          {filteredCommands.map((c, i) => (
            <div
              key={c.cmd}
              onMouseDown={() => selectSlashCommand(c)}
              onMouseEnter={() => setSlashPopoverIndex(i)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 12,
                padding: "9px 14px",
                cursor: "pointer",
                background: i === slashPopoverIndex ? "rgba(255,255,255,0.07)" : "transparent",
                borderBottom: i < filteredCommands.length - 1
                  ? "1px solid rgba(255,255,255,0.04)"
                  : "none",
                transition: "background 0.1s",
              }}
            >
              {/* Command chip */}
              <span style={{
                fontSize: 12,
                fontWeight: 600,
                color: i === slashPopoverIndex
                  ? "var(--axion-accent, #a7cadc)"
                  : "rgba(255,255,255,0.75)",
                fontFamily: "var(--axion-font-mono, monospace)",
                minWidth: 110,
                flexShrink: 0,
              }}>
                {c.cmd}
              </span>
              {/* Description */}
              <span style={{
                fontSize: 12,
                color: "rgba(255,255,255,0.35)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                flex: 1,
              }}>
                {c.desc}
              </span>
              {/* Category tag */}
              <span style={{
                marginLeft: "auto",
                fontSize: 10,
                padding: "2px 7px",
                borderRadius: 6,
                background: "rgba(255,255,255,0.06)",
                color: "rgba(255,255,255,0.25)",
                flexShrink: 0,
              }}>
                {c.category}
              </span>
            </div>
          ))}

          {/* Footer hint */}
          <div style={{
            padding: "6px 14px",
            fontSize: 10,
            color: "rgba(255,255,255,0.18)",
            borderTop: "1px solid rgba(255,255,255,0.06)",
            display: "flex",
            gap: 12,
          }}>
            <span>↑↓ navigate</span>
            <span>↵ select</span>
            <span>esc dismiss</span>
          </div>
        </motion.div>
      )}
    </AnimatePresence>

    <div
      className={`relative p-[1.5px] rounded-2xl ${
        isStreaming
          ? "axion-light-trace-active axion-breathe-glow"
          : "axion-light-trace"
      }`}
      style={{
        boxShadow: "0 8px 50px rgba(0,0,0,0.70), 0 0 0 1px rgba(255,255,255,0.06)",
        border: "none",
      }}
    >
      {/* Glass inner — visibly lifted off the page bg */}
      <div
        className="relative rounded-2xl flex items-end gap-3 px-5 py-4"
        style={{
          background: "rgba(10,14,20,0.97)",
          backdropFilter: "blur(40px) saturate(150%)",
          WebkitBackdropFilter: "blur(40px) saturate(150%)",
          border: "0.5px solid rgba(255,255,255,0.08)",
          boxShadow: "0 1px 0 0 rgba(255,255,255,0.06) inset, 0 0 0 1px rgba(255,255,255,0.04)",
        }}
      >
        {/* Hidden file input — triggered by the upload icon button */}
        <input
          type="file"
          accept="image/*,.csv,.json,.txt,text/csv,application/json,text/plain"
          ref={fileInputRef}
          onChange={handleFileUpload}
          style={{ display: "none" }}
        />

        {/* Upload icon button — leading edge, aligned to bottom */}
        <button
          onClick={() => fileInputRef.current?.click()}
          disabled={isUploading || isStreaming}
          title="Attach image"
          className="shrink-0 flex items-center justify-center transition-colors"
          style={{
            width: 32,
            height: 32,
            borderRadius: 8,
            background: "transparent",
            border: "none",
            cursor: isUploading || isStreaming ? "not-allowed" : "pointer",
            color: isUploading || isStreaming
              ? "rgba(255,255,255,0.22)"
              : uploadedFile
              ? "var(--axion-accent, #a7cadc)"
              : "rgba(255,255,255,0.45)",
            padding: 0,
          }}
          onMouseEnter={(e) => {
            if (!isUploading && !isStreaming)
              (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.75)";
          }}
          onMouseLeave={(e) => {
            if (!isUploading && !isStreaming)
              (e.currentTarget as HTMLButtonElement).style.color = uploadedFile
                ? "var(--axion-accent, #a7cadc)"
                : "rgba(255,255,255,0.45)";
          }}
        >
          {isUploading ? (
            <div
              className="animate-spin"
              style={{
                width: 14,
                height: 14,
                borderRadius: "50%",
                border: "2px solid rgba(255,255,255,0.18)",
                borderTopColor: "rgba(255,255,255,0.65)",
              }}
            />
          ) : (
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <rect x="3" y="3" width="18" height="18" rx="2" />
              <circle cx="8.5" cy="8.5" r="1.5" />
              <polyline points="21 15 16 10 5 21" />
            </svg>
          )}
        </button>

        {/* Column: attachment chip + error + textarea */}
        <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 6 }}>
          {/* Thumbnail chip */}
          {uploadedFile && (
            <div
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: 8,
                background: "rgba(255,255,255,0.08)",
                border: "1px solid rgba(255,255,255,0.12)",
                borderRadius: 10,
                padding: "6px 10px",
                alignSelf: "flex-start",
              }}
            >
              {/* Image: thumbnail preview. Data: file-type icon. */}
              {uploadedFile.fileType === "image" && uploadedFile.previewUrl ? (
                <img
                  src={uploadedFile.previewUrl}
                  alt=""
                  style={{
                    width: 40,
                    height: 40,
                    borderRadius: 8,
                    objectFit: "cover",
                    flexShrink: 0,
                  }}
                />
              ) : (
                <span
                  style={{
                    width: 40,
                    height: 40,
                    borderRadius: 8,
                    background: "rgba(167,202,220,0.12)",
                    border: "1px solid rgba(167,202,220,0.20)",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    flexShrink: 0,
                    fontSize: 18,
                  }}
                >
                  {uploadedFile.filename.endsWith(".csv") ? "📊"
                    : uploadedFile.filename.endsWith(".json") ? "{ }"
                    : "📄"}
                </span>
              )}
              <div style={{ display: "flex", flexDirection: "column", gap: 1 }}>
                <span
                  style={{
                    fontSize: 12,
                    color: "rgba(255,255,255,0.75)",
                    fontFamily: "var(--font-mono, monospace)",
                    maxWidth: 180,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {(uploadedFile.originalName ?? uploadedFile.filename).length > 24
                    ? (uploadedFile.originalName ?? uploadedFile.filename).slice(0, 24) + "…"
                    : (uploadedFile.originalName ?? uploadedFile.filename)}
                </span>
                {uploadedFile.fileType === "data" && (
                  <span style={{ fontSize: 10, color: "rgba(167,202,220,0.60)", fontFamily: "var(--font-mono, monospace)" }}>
                    data file
                  </span>
                )}
              </div>
              <button
                onClick={() => {
                  if (uploadedFile.previewUrl) URL.revokeObjectURL(uploadedFile.previewUrl);
                  setUploadedFile(null);
                }}
                title="Remove attachment"
                style={{
                  background: "none",
                  border: "none",
                  color: "rgba(255,255,255,0.40)",
                  cursor: "pointer",
                  fontSize: 16,
                  lineHeight: 1,
                  padding: "0 2px",
                  flexShrink: 0,
                }}
              >
                ×
              </button>
            </div>
          )}

          {/* Inline upload error */}
          {uploadError && (
            <p style={{ fontSize: 11, color: "rgba(255,80,80,0.85)", margin: 0 }}>
              {uploadError}
            </p>
          )}

          {/* Textarea */}
          <textarea
            ref={textareaRef}
            rows={1}
            value={intent}
            onChange={(e) => {
              const v = e.target.value;
              setIntent(v);
              const el = e.target;
              el.style.height = "auto";
              el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
              // Slash-command detection: drive both execution state and discovery popover
              if (v.startsWith("/")) {
                setSlashQuery(v.slice(1));
                setSlashIndex(0);
                setSlashPopoverOpen(true);
                setSlashFilter(v);
                setSlashPopoverIndex(0);
              } else {
                setSlashQuery(null);
                setSlashPopoverOpen(false);
                setSlashFilter("");
              }
            }}
            onKeyDown={(e) => {
              // Discovery popover navigation / insertion
              if (slashPopoverOpen && filteredCommands.length > 0) {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setSlashPopoverIndex((i) => Math.min(i + 1, filteredCommands.length - 1));
                  return;
                }
                if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setSlashPopoverIndex((i) => Math.max(i - 1, 0));
                  return;
                }
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  selectSlashCommand(filteredCommands[slashPopoverIndex]);
                  return;
                }
                if (e.key === "Escape") {
                  e.preventDefault();
                  setSlashPopoverOpen(false);
                  setSlashFilter("");
                  return;
                }
              }
              // Execute inserted slash command (popover closed, slashQuery armed)
              if (!slashPopoverOpen && slashQuery !== null && filteredSlash.length > 0) {
                if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
                  e.preventDefault();
                  executeSlashCommand(filteredSlash[slashIndex]);
                  return;
                }
                if (e.key === "Escape") { e.preventDefault(); setSlashQuery(null); return; }
              }
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                runMission();
              }
            }}
            onBlur={() => {
              // Delay so onMouseDown on a popover row fires before the popover unmounts
              setTimeout(() => { setSlashPopoverOpen(false); setSlashFilter(""); }, 150);
            }}
            placeholder={
              intent === "" && history.length > 0
                ? `Continue: "${history[0].intent.slice(0, 55)}${history[0].intent.length > 55 ? "…" : ""}"`
                : "Describe your mission intent…"
            }
            disabled={isStreaming}
            className="bg-transparent text-lg text-gray-100 placeholder-gray-500
                       focus:outline-none resize-none leading-relaxed overflow-hidden
                       disabled:opacity-50"
            style={{ minHeight: "28px", maxHeight: "200px", width: "100%" }}
          />
        </div>

        {/* Execute button */}
        <button
          onClick={runMission}
          disabled={isStreaming || !intent.trim()}
          className="shrink-0 text-sm font-semibold px-5 py-2 rounded-xl transition-all
                     disabled:opacity-40 disabled:cursor-not-allowed"
          style={{
            background: isStreaming
              ? "rgba(255,255,255,0.06)"
              : "var(--axion-accent, #a7cadc)",
            color: isStreaming ? "rgba(255,255,255,0.4)" : "var(--axion-accent-fg, #07090c)",
            boxShadow: isStreaming
              ? "none"
              : "0 0 20px rgba(var(--axion-accent-rgb, 167,202,220), 0.30)",
          }}
        >
          {isStreaming ? "Running…" : "Execute"}
        </button>
      </div>
    </div>
    </div>
  );

  // ── Settings helpers ─────────────────────────────────────────────────────────

  function saveSettings() {
    const axionKeyTrimmed = draftAxionKey.trim();
    const openAITrimmed = draftOpenAI.trim();
    const tavilyTrimmed = draftTavily.trim();
    if (axionKeyTrimmed) {
      localStorage.setItem("axion_api_key", axionKeyTrimmed);
    } else {
      localStorage.removeItem("axion_api_key");
    }
    if (openAITrimmed) {
      localStorage.setItem("axion_openai_key", openAITrimmed);
    } else {
      localStorage.removeItem("axion_openai_key");
    }
    if (tavilyTrimmed) {
      localStorage.setItem("axion_tavily_key", tavilyTrimmed);
    } else {
      localStorage.removeItem("axion_tavily_key");
    }
    setSettingsSaved(true);
    setTimeout(() => setSettingsSaved(false), 2000);
    setSettingsOpen(false);
  }

  // Badge: shown when OpenAI is not configured via env AND not stored locally
  const showConfigBadge = configStatus?.openai === false && !draftOpenAI.trim();

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

      {/* ── Settings gear button ─────────────────────────────────────────── */}
      <button
        onClick={() => setSettingsOpen((v) => !v)}
        title="Settings"
        style={{
          position: "fixed",
          top: 20,
          right: 24,
          zIndex: 200,
          width: 36,
          height: 36,
          borderRadius: 10,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          background: "rgba(255,255,255,0.06)",
          border: "0.5px solid rgba(255,255,255,0.10)",
          backdropFilter: "blur(20px)",
          WebkitBackdropFilter: "blur(20px)",
          color: "rgba(255,255,255,0.35)",
          cursor: "pointer",
          transition: "color 0.15s, background 0.15s",
        }}
        onMouseEnter={(e) => {
          (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.70)";
          (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.10)";
        }}
        onMouseLeave={(e) => {
          (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.35)";
          (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.06)";
        }}
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none"
             stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="3"/>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06
                   a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09
                   A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83
                   l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09
                   A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83
                   l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09
                   a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83
                   l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09
                   a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
        {/* Red dot badge — shown when OpenAI is unconfigured */}
        {showConfigBadge && (
          <span style={{
            position: "absolute",
            top: 5,
            right: 5,
            width: 8,
            height: 8,
            borderRadius: "50%",
            background: "#f87171",
            pointerEvents: "none",
          }} />
        )}
      </button>

      {/* ── Settings backdrop ─────────────────────────────────────────────── */}
      <AnimatePresence>
        {settingsOpen && (
          <motion.div
            key="settings-backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={() => setSettingsOpen(false)}
            style={{
              position: "fixed", inset: 0,
              background: "rgba(0,0,0,0.30)",
              zIndex: 189,
            }}
          />
        )}
      </AnimatePresence>

      {/* ── Settings drawer ───────────────────────────────────────────────── */}
      <AnimatePresence>
        {settingsOpen && (
          <motion.div
            key="settings-drawer"
            initial={{ x: 360 }}
            animate={{ x: 0 }}
            exit={{ x: 360 }}
            transition={{ type: "spring", stiffness: 300, damping: 30 }}
            style={{
              position: "fixed",
              top: 0,
              right: 0,
              height: "100%",
              width: 360,
              zIndex: 190,
              background: "rgba(8,10,14,0.96)",
              backdropFilter: "blur(40px) saturate(150%)",
              WebkitBackdropFilter: "blur(40px) saturate(150%)",
              borderLeft: "1px solid rgba(255,255,255,0.07)",
              display: "flex",
              flexDirection: "column",
              padding: "28px 28px 32px",
              overflowY: "auto",
            }}
          >
            {/* Header row */}
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 28 }}>
              <p style={{ fontSize: 15, fontWeight: 700, color: "rgba(255,255,255,0.9)", margin: 0 }}>
                Settings
              </p>
              <button
                onClick={() => setSettingsOpen(false)}
                style={{
                  background: "none", border: "none",
                  color: "rgba(255,255,255,0.35)",
                  fontSize: 20, lineHeight: 1,
                  cursor: "pointer", padding: "0 2px",
                  transition: "color 0.15s",
                }}
                onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.80)"; }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.35)"; }}
              >
                ×
              </button>
            </div>

            {/* Axion API Key */}
            <div style={{ marginBottom: 24 }}>
              <label style={{ display: "block", fontSize: 11, fontWeight: 600, letterSpacing: "0.08em", textTransform: "uppercase", color: "rgba(255,255,255,0.45)", marginBottom: 8 }}>
                Axion API Key
              </label>
              <input
                type="password"
                value={draftAxionKey}
                onChange={(e) => setDraftAxionKey(e.target.value)}
                placeholder="axion_sk_..."
                style={{
                  width: "100%",
                  background: "rgba(255,255,255,0.07)",
                  border: "1px solid rgba(255,255,255,0.12)",
                  borderRadius: 10,
                  padding: "10px 14px",
                  color: "rgba(255,255,255,0.9)",
                  fontSize: 13,
                  outline: "none",
                  boxSizing: "border-box",
                  transition: "border-color 0.15s",
                }}
                onFocus={(e) => { e.currentTarget.style.borderColor = "var(--axion-accent, #a7cadc)"; }}
                onBlur={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.12)"; }}
              />
              <p style={{ fontSize: 11, color: "rgba(255,255,255,0.28)", marginTop: 6, margin: "6px 0 0" }}>
                Required when the server has AXION_API_KEY set. Leave blank for local dev.
              </p>
            </div>

            {/* OpenAI Key */}
            <div style={{ marginBottom: 24 }}>
              <label style={{ display: "block", fontSize: 11, fontWeight: 600, letterSpacing: "0.08em", textTransform: "uppercase", color: "rgba(255,255,255,0.45)", marginBottom: 8 }}>
                OpenAI API Key
              </label>
              <input
                type="password"
                value={draftOpenAI}
                onChange={(e) => setDraftOpenAI(e.target.value)}
                placeholder="sk-..."
                style={{
                  width: "100%",
                  background: "rgba(255,255,255,0.07)",
                  border: "1px solid rgba(255,255,255,0.12)",
                  borderRadius: 10,
                  padding: "10px 14px",
                  color: "rgba(255,255,255,0.9)",
                  fontSize: 13,
                  outline: "none",
                  boxSizing: "border-box",
                  transition: "border-color 0.15s",
                }}
                onFocus={(e) => { e.currentTarget.style.borderColor = "var(--axion-accent, #a7cadc)"; }}
                onBlur={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.12)"; }}
              />
              {configStatus?.openai && !draftOpenAI ? (
                <p style={{ fontSize: 11, color: "rgba(74,222,128,0.8)", marginTop: 6, margin: "6px 0 0" }}>
                  ✓ Configured via environment
                </p>
              ) : (
                <p style={{ fontSize: 11, color: "rgba(255,255,255,0.28)", marginTop: 6, margin: "6px 0 0" }}>
                  Required for all missions.
                </p>
              )}
            </div>

            {/* Tavily Key */}
            <div style={{ marginBottom: 24 }}>
              <label style={{ display: "block", fontSize: 11, fontWeight: 600, letterSpacing: "0.08em", textTransform: "uppercase", color: "rgba(255,255,255,0.45)", marginBottom: 8 }}>
                Tavily Search Key
              </label>
              <input
                type="password"
                value={draftTavily}
                onChange={(e) => setDraftTavily(e.target.value)}
                placeholder="tvly-..."
                style={{
                  width: "100%",
                  background: "rgba(255,255,255,0.07)",
                  border: "1px solid rgba(255,255,255,0.12)",
                  borderRadius: 10,
                  padding: "10px 14px",
                  color: "rgba(255,255,255,0.9)",
                  fontSize: 13,
                  outline: "none",
                  boxSizing: "border-box",
                  transition: "border-color 0.15s",
                }}
                onFocus={(e) => { e.currentTarget.style.borderColor = "var(--axion-accent, #a7cadc)"; }}
                onBlur={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.12)"; }}
              />
              {configStatus?.tavily && !draftTavily ? (
                <p style={{ fontSize: 11, color: "rgba(74,222,128,0.8)", marginTop: 6, margin: "6px 0 0" }}>
                  ✓ Configured via environment
                </p>
              ) : (
                <p style={{ fontSize: 11, color: "rgba(255,255,255,0.28)", marginTop: 6, margin: "6px 0 0" }}>
                  Optional — enables live web search.
                </p>
              )}
            </div>

            {/* Save button */}
            <button
              onClick={saveSettings}
              style={{
                width: "100%",
                background: "var(--axion-accent, #a7cadc)",
                color: "#000",
                fontWeight: 600,
                fontSize: 14,
                borderRadius: 10,
                padding: "11px 0",
                marginTop: 8,
                border: "none",
                cursor: "pointer",
                transition: "opacity 0.15s",
              }}
              onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.opacity = "0.85"; }}
              onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.opacity = "1"; }}
            >
              Save
            </button>

            {/* ── Notification permission row ──────────────────────────────── */}
            {notifPermission !== "unsupported" && (
              <div
                style={{
                  marginTop: 20,
                  paddingTop: 16,
                  borderTop: "1px solid rgba(255,255,255,0.07)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  gap: 12,
                }}
              >
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontSize: 13, fontWeight: 600, color: "rgba(255,255,255,0.8)" }}>
                    Mission notifications
                  </div>
                  <div style={{ fontSize: 11, color: "rgba(255,255,255,0.3)", marginTop: 2, lineHeight: 1.4 }}>
                    {notifPermission === "granted" && "Enabled — you’ll be notified when missions complete"}
                    {notifPermission === "denied"  && "Blocked — enable in your browser settings"}
                    {notifPermission === "default" && "Not yet requested — start a mission to prompt"}
                  </div>
                </div>
                {notifPermission === "default" && (
                  <button
                    onClick={() =>
                      Notification.requestPermission().then((perm) => setNotifPermission(perm))
                    }
                    style={{
                      flexShrink: 0,
                      fontSize: 11,
                      padding: "5px 12px",
                      borderRadius: 8,
                      cursor: "pointer",
                      background: "rgba(255,255,255,0.08)",
                      border: "1px solid rgba(255,255,255,0.12)",
                      color: "rgba(255,255,255,0.65)",
                      whiteSpace: "nowrap",
                    }}
                  >
                    Enable
                  </button>
                )}
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Saved toast ───────────────────────────────────────────────────── */}
      <AnimatePresence>
        {settingsSaved && (
          <motion.div
            key="settings-saved-toast"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 8 }}
            transition={{ type: "spring", stiffness: 360, damping: 28 }}
            style={{
              position: "fixed",
              bottom: 96,
              left: "50%",
              transform: "translateX(-50%)",
              zIndex: 300,
              background: "rgba(255,255,255,0.10)",
              border: "0.5px solid rgba(255,255,255,0.18)",
              backdropFilter: "blur(24px)",
              WebkitBackdropFilter: "blur(24px)",
              borderRadius: 10,
              padding: "8px 18px",
              fontSize: 13,
              fontWeight: 600,
              color: "rgba(255,255,255,0.85)",
              pointerEvents: "none",
            }}
          >
            ✓ Saved
          </motion.div>
        )}
      </AnimatePresence>

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
              background: "rgba(6,8,11,0.88)",
              backdropFilter: "blur(40px) saturate(150%)",
              WebkitBackdropFilter: "blur(40px) saturate(150%)",
              borderRight: "1px solid rgba(255,255,255,0.07)",
              boxShadow: "4px 0 60px rgba(0,0,0,0.60), inset -1px 0 0 rgba(255,255,255,0.04)",
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
                  background: "linear-gradient(rgba(255,255,255,0.07), rgba(255,255,255,0.03))",
                  color: "rgba(235,239,242,0.92)",
                  fontWeight: 600,
                  fontSize: 14,
                  fontFamily: "var(--axion-font-main)",
                  textAlign: "left",
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  border: "1px solid rgba(255,255,255,0.11)",
                  backdropFilter: "blur(28px) saturate(130%)",
                  WebkitBackdropFilter: "blur(28px) saturate(130%)",
                  boxShadow: "inset 0 1px 0 rgba(255,255,255,0.08), 0 0 0 1px rgba(255,255,255,0.04)",
                  transition: "background 0.2s, border-color 0.2s",
                }}
                onMouseEnter={(e) => {
                  const el = e.currentTarget as HTMLButtonElement;
                  el.style.background = "linear-gradient(rgba(255,255,255,0.10), rgba(255,255,255,0.05))";
                  el.style.borderColor = "rgba(255,255,255,0.16)";
                }}
                onMouseLeave={(e) => {
                  const el = e.currentTarget as HTMLButtonElement;
                  el.style.background = "linear-gradient(rgba(255,255,255,0.07), rgba(255,255,255,0.03))";
                  el.style.borderColor = "rgba(255,255,255,0.11)";
                }}
              >
                <span style={{ fontSize: 18, lineHeight: 1, opacity: 0.80 }}>＋</span>
                New Mission
              </button>
            </div>

            {/* History list */}
            <div style={{ flex: 1, overflowY: "auto", padding: "6px 8px 20px", display: "flex", flexDirection: "column" }}>

              {/* Search input */}
              {history.length > 0 && (
                <div style={{ position: "relative", marginBottom: 8, flexShrink: 0 }}>
                  {/* Magnifier icon */}
                  <span
                    style={{
                      position: "absolute", left: 10, top: "50%", transform: "translateY(-50%)",
                      fontSize: 12, color: "rgba(255,255,255,0.28)", pointerEvents: "none",
                      lineHeight: 1,
                    }}
                  >
                    ⌕
                  </span>
                  <input
                    type="text"
                    value={historyQuery}
                    onChange={(e) => setHistoryQuery(e.target.value)}
                    onFocus={() => setHistoryQueryFocused(true)}
                    onBlur={() => setHistoryQueryFocused(false)}
                    placeholder="Search missions…"
                    style={{
                      width: "100%",
                      padding: "7px 28px 7px 28px",
                      borderRadius: 10,
                      background: historyQueryFocused
                        ? "rgba(255,255,255,0.08)"
                        : "rgba(255,255,255,0.05)",
                      border: `0.5px solid ${historyQueryFocused ? "rgba(var(--axion-accent-rgb, 167,202,220), 0.45)" : "rgba(255,255,255,0.10)"}`,
                      color: "rgba(255,255,255,0.82)",
                      fontSize: 12,
                      outline: "none",
                      transition: "background 0.15s, border-color 0.15s",
                      boxSizing: "border-box",
                    }}
                  />
                  {/* Clear ×  */}
                  {historyQuery && (
                    <button
                      onClick={() => setHistoryQuery("")}
                      style={{
                        position: "absolute", right: 8, top: "50%", transform: "translateY(-50%)",
                        fontSize: 13, lineHeight: 1, color: "rgba(255,255,255,0.35)",
                        background: "none", border: "none", cursor: "pointer", padding: 2,
                      }}
                    >
                      ×
                    </button>
                  )}
                </div>
              )}

              {history.length === 0 ? (
                <div style={{ padding: "36px 16px", textAlign: "center" }}>
                  <div style={{ fontSize: 24, opacity: 0.18, marginBottom: 8 }}>◈</div>
                  <p style={{ fontSize: 11, color: "rgba(255,255,255,0.22)", lineHeight: 1.6 }}>
                    No missions yet.<br />Run your first to begin.
                  </p>
                </div>
              ) : filteredHistory.length === 0 ? (
                <div style={{ padding: "28px 16px", textAlign: "center" }}>
                  <div style={{ fontSize: 20, opacity: 0.20, marginBottom: 6 }}>◈</div>
                  <p style={{ fontSize: 11, color: "rgba(255,255,255,0.22)", lineHeight: 1.6 }}>
                    No matches for<br />
                    <span style={{ color: "rgba(255,255,255,0.40)", fontStyle: "italic" }}>
                      &ldquo;{historyQuery}&rdquo;
                    </span>
                  </p>
                </div>
              ) : (
                <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
                  <AnimatePresence initial={false}>
                  {filteredHistory.map((m) => {
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

              {/* Clear all — only when there are missions */}
              {history.length > 0 && (
                <div style={{ marginTop: "auto", paddingTop: 14, flexShrink: 0 }}>
                  <button
                    onClick={() => {
                      if (confirmClear) {
                        clearAllHistory();
                      } else {
                        setConfirmClear(true);
                        // Auto-reset confirmation after 3 s
                        setTimeout(() => setConfirmClear(false), 3000);
                      }
                    }}
                    style={{
                      width: "100%",
                      padding: "8px 12px",
                      borderRadius: 10,
                      fontSize: 11,
                      fontWeight: 600,
                      letterSpacing: "0.04em",
                      cursor: "pointer",
                      background: confirmClear
                        ? "rgba(239,68,68,0.14)"
                        : "rgba(255,255,255,0.04)",
                      border: `0.5px solid ${confirmClear ? "rgba(239,68,68,0.40)" : "rgba(255,255,255,0.10)"}`,
                      color: confirmClear ? "#f87171" : "rgba(255,255,255,0.28)",
                      transition: "background 0.15s, border-color 0.15s, color 0.15s",
                    }}
                    onMouseEnter={(e) => {
                      if (!confirmClear) {
                        (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.50)";
                        (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.07)";
                      }
                    }}
                    onMouseLeave={(e) => {
                      if (!confirmClear) {
                        (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.28)";
                        (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.04)";
                      }
                    }}
                  >
                    {confirmClear ? "⚠ Confirm — delete all missions" : "Clear all history"}
                  </button>
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
          background: "linear-gradient(rgba(255,255,255,0.05), rgba(255,255,255,0.02))",
          border: "1px solid rgba(255,255,255,0.08)",
          backdropFilter: "blur(28px) saturate(130%)",
          WebkitBackdropFilter: "blur(28px) saturate(130%)",
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
                padding: "0 24px",
              }}
            >
              {/* Version badge — mono, uppercase, accent border (landing page style) */}
              <motion.span
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.1, duration: 0.3 }}
                className="axion-mono-badge"
                style={{ marginBottom: 28 }}
              >
                Kernel v0.1 · Alpha
              </motion.span>

              {/* Main wordmark — Space Grotesk display font */}
              <motion.h1
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.18, duration: 0.4 }}
                style={{
                  fontFamily: "var(--axion-font-display)",
                  fontSize: "clamp(4rem, 10vw, 7rem)",
                  fontWeight: 600,
                  letterSpacing: "-0.03em",
                  lineHeight: 1.02,
                  background: "linear-gradient(135deg, #ebeff2 20%, #9399a0 55%, #c8d4da 85%)",
                  WebkitBackgroundClip: "text",
                  WebkitTextFillColor: "transparent",
                  backgroundClip: "text",
                  marginBottom: 18,
                }}
              >
                Axion
              </motion.h1>

              {/* Tagline — muted, tracking-wide */}
              <motion.p
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ delay: 0.28, duration: 0.4 }}
                style={{
                  fontSize: 15,
                  color: "rgba(147,153,160,0.85)",
                  letterSpacing: "-0.01em",
                  fontWeight: 400,
                  maxWidth: 380,
                  lineHeight: 1.55,
                  fontFamily: "var(--axion-font-main)",
                }}
              >
                The headless intelligence kernel. Synthesize intent into verified, structured state.
              </motion.p>
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
              <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: 3 }}>
                <h1
                  style={{
                    fontFamily: "var(--axion-font-display)",
                    fontSize: "clamp(1.5rem, 3vw, 2rem)",
                    fontWeight: 600,
                    letterSpacing: "-0.03em",
                    lineHeight: 1.05,
                    background: "linear-gradient(135deg, #ebeff2 20%, #9399a0 60%, #c8d4da 90%)",
                    WebkitBackgroundClip: "text",
                    WebkitTextFillColor: "transparent",
                    backgroundClip: "text",
                  }}
                >
                  Axion
                </h1>
                <span className="axion-mono-badge" style={{ fontSize: 8 }}>
                  Kernel v0.1
                </span>
              </div>
            </motion.header>
          )}
        </AnimatePresence>

        {/* Idle: template gallery */}
        <AnimatePresence>
          {isIdle && (
            <motion.div
              key="idle-widgets"
              exit={{ opacity: 0, scale: 0.96, transition: { duration: 0.18 } }}
            >
              <TemplateGallery
                onSelect={(selectedIntent) => {
                  setIntent(selectedIntent);
                  // Delay focus by one tick so the textarea has re-rendered
                  // with the new value before we move the cursor into it.
                  setTimeout(() => textareaRef.current?.focus(), 50);
                }}
              />
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
                    background: "linear-gradient(rgba(255,255,255,0.04), rgba(255,255,255,0.016))",
                    border: "1px solid rgba(255,255,255,0.08)",
                    backdropFilter: "blur(28px) saturate(130%)",
                    WebkitBackdropFilter: "blur(28px) saturate(130%)",
                    boxShadow: "inset 0 1px 0 0 rgba(255,255,255,0.06), 0 0 18px var(--axion-glow, rgba(167,202,220,0.15))",
                  }}
                >
                  <div
                    className="w-4 h-4 rounded-full animate-spin shrink-0"
                    style={{
                      border: "2px solid var(--axion-accent, #a7cadc)",
                      borderTopColor: "transparent",
                    }}
                  />
                  <div className="min-w-0">
                    <p className="text-xs font-semibold" style={{ color: "var(--axion-accent, #a7cadc)" }}>
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
                      border: "2px solid var(--axion-accent, #a7cadc)",
                      borderTopColor: "transparent",
                      boxShadow: "0 0 16px var(--axion-glow, rgba(167,202,220,0.25))",
                    }}
                  />
                  <p className="text-gray-400 text-sm">🤖 Axion swarm initializing…</p>
                </div>
              )}

              {/* Governor expand banner */}
              {governorBanner && (
                <div className="rounded-xl px-4 py-2.5 text-xs mb-4" style={{ background: "rgba(167,202,220,0.06)", border: "1px solid rgba(167,202,220,0.18)", color: "rgba(167,202,220,0.80)" }}>
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
                    <p className="text-[10px] font-semibold uppercase tracking-[0.14em]" style={{ color: "rgba(147,153,160,0.70)", fontFamily: "var(--axion-font-mono)" }}>
                      {missionStatus === "complete"
                        ? activeMissionId ? "Loaded from history" : "Mission complete"
                        : "Mission in progress"}{" "}
                      {missionMeta && `— ${missionMeta.task_count} task${missionMeta.task_count !== 1 ? "s" : ""}`}
                    </p>
                    {missionMeta?.expanded_task_count != null && missionMeta.expanded_task_count > 0 && (
                      <span className="inline-flex items-center gap-1 text-xs font-semibold rounded-full px-2.5 py-0.5" style={{ background: "rgba(167,202,220,0.08)", border: "1px solid rgba(167,202,220,0.22)", color: "rgba(167,202,220,0.80)" }}>
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

                  {/* ── Refinement history timeline ──────────────────────────── */}
                  {refinementHistory.length > 0 && (
                    <div style={{
                      borderRadius: 12,
                      background: "rgba(167,202,220,0.05)",
                      border: "0.5px solid rgba(167,202,220,0.18)",
                      overflow: "hidden",
                    }}>
                      {/* Toggle header */}
                      <button
                        onClick={() => setShowRefinementHistory(v => !v)}
                        style={{
                          width: "100%",
                          padding: "9px 14px",
                          display: "flex",
                          alignItems: "center",
                          gap: 8,
                          background: "transparent",
                          cursor: "pointer",
                          textAlign: "left",
                        }}
                      >
                        <span style={{ fontSize: 9, fontWeight: 700, letterSpacing: "0.10em", textTransform: "uppercase", color: "var(--axion-accent, rgba(167,202,220,0.70))" }}>
                          Refinement history ({refinementHistory.length})
                        </span>
                        <span style={{ flex: 1 }} />
                        <span style={{ fontSize: 10, color: "rgba(255,255,255,0.28)" }}>
                          {showRefinementHistory ? "▲" : "▼"}
                        </span>
                      </button>

                      {/* Collapsible rows */}
                      <AnimatePresence>
                        {showRefinementHistory && (
                          <motion.div
                            key="refine-history"
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: "auto", opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            transition={{ type: "spring", stiffness: 360, damping: 32 }}
                            style={{ overflow: "hidden", borderTop: "0.5px solid rgba(167,202,220,0.12)" }}
                          >
                            <div style={{ padding: "8px 10px 10px", display: "flex", flexDirection: "column", gap: 6 }}>
                              {refinementHistory.map((round, ri) => (
                                <div
                                  key={ri}
                                  style={{
                                    display: "flex",
                                    gap: 10,
                                    alignItems: "flex-start",
                                    padding: "8px 10px",
                                    borderRadius: 8,
                                    background: "var(--axion-glass-bg, rgba(255,255,255,0.04))",
                                    borderLeft: "2px solid var(--axion-accent, #a7cadc)",
                                  }}
                                >
                                  {/* R{n} badge */}
                                  <span style={{
                                    flexShrink: 0,
                                    fontSize: 9,
                                    fontWeight: 800,
                                    letterSpacing: "0.04em",
                                    padding: "2px 6px",
                                    borderRadius: 5,
                                    background: "rgba(167,202,220,0.18)",
                                    border: "0.5px solid rgba(167,202,220,0.35)",
                                    color: "var(--axion-accent, #a7cadc)",
                                    lineHeight: 1.5,
                                    marginTop: 1,
                                  }}>
                                    R{ri + 1}
                                  </span>

                                  {/* Intent + meta row */}
                                  <div style={{ flex: 1, minWidth: 0 }}>
                                    <p style={{
                                      fontSize: 12,
                                      color: "rgba(255,255,255,0.78)",
                                      fontWeight: 500,
                                      lineHeight: 1.4,
                                      marginBottom: 4,
                                      // 2-line clamp
                                      display: "-webkit-box",
                                      WebkitLineClamp: 2,
                                      WebkitBoxOrient: "vertical",
                                      overflow: "hidden",
                                    } as React.CSSProperties}>
                                      {round.intent}
                                    </p>
                                    <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                                      {/* Relative timestamp */}
                                      <span style={{ fontSize: 9, color: "rgba(255,255,255,0.28)", fontFamily: "var(--font-mono, monospace)" }}>
                                        {timeAgo(round.timestamp)}
                                      </span>
                                      {/* Key chip: "no new data" or "+N new keys" */}
                                      {round.newPayloadKeys.length === 0 ? (
                                        <span style={{
                                          fontSize: 9, padding: "1px 6px", borderRadius: 4,
                                          background: "rgba(255,255,255,0.05)",
                                          border: "0.5px solid rgba(255,255,255,0.10)",
                                          color: "rgba(255,255,255,0.28)",
                                          fontWeight: 600,
                                        }}>
                                          no new data
                                        </span>
                                      ) : (
                                        <span style={{
                                          fontSize: 9, padding: "1px 6px", borderRadius: 4,
                                          background: "rgba(167,202,220,0.12)",
                                          border: "0.5px solid rgba(167,202,220,0.25)",
                                          color: "var(--axion-accent, #a7cadc)",
                                          fontWeight: 600,
                                        }}>
                                          +{round.newPayloadKeys.length} new key{round.newPayloadKeys.length !== 1 ? "s" : ""}
                                        </span>
                                      )}
                                    </div>
                                  </div>
                                </div>
                              ))}
                            </div>
                          </motion.div>
                        )}
                      </AnimatePresence>
                    </div>
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
                        <div className={missionStatus === "complete" && missionState && !isRefining ? "axion-sheen-wrapper" : ""}>
                          <Renderer
                            blueprint={blueprint}
                            pinnedCards={pinnedCards}
                            dismissedCards={dismissedCards}
                            refinedIndices={(() => {
                              // Map refined payload-keys → component indices
                              const idxSet = new Set<number>();
                              if (refinedPayloadKeys.size > 0) {
                                blueprint.components.forEach((c, idx) => {
                                  const title = (c.props.title as string | undefined) ?? "";
                                  // Check if the component's data key was refined
                                  // by matching formatted key → title heuristic
                                  for (const key of refinedPayloadKeys) {
                                    const formattedKey = key.replace(/_/g, " ").replace(/\b\w/g, ch => ch.toUpperCase());
                                    if (title === formattedKey || title.toLowerCase().includes(key.toLowerCase())) {
                                      idxSet.add(idx);
                                    }
                                  }
                                });
                              }
                              return idxSet;
                            })()}
                            onPin={(i) => setPinnedCards((prev) => { const n = new Set(prev); n.has(i) ? n.delete(i) : n.add(i); return n; })}
                            onDismiss={(i) => {
                              if (i < 0) {
                                setDismissedCards(new Set());
                              } else {
                                setDismissedCards((prev) => { const n = new Set(prev); n.add(i); return n; });
                              }
                            }}
                          />
                        </div>

                        {/* Conflict badge — when Analyst flagged conflicting data */}
                        {Array.isArray((missionState.data_payload as Record<string,unknown>).data_conflicts) && (() => {
                          const conflicts = (missionState.data_payload as Record<string,unknown>).data_conflicts as {field:string; values:string[]; sources:string[]}[];
                          const conflictSummary = conflicts.map(c => `${c.field}: ${c.values.join(" vs ")}`).join("; ");
                          return (
                            <motion.div
                              initial={{ opacity: 0, y: 8 }}
                              animate={{ opacity: 1, y: 0 }}
                              style={{
                                marginTop: 16,
                                padding: "14px 16px",
                                borderRadius: 14,
                                background: "rgba(251,191,36,0.07)",
                                border: "1px solid rgba(251,191,36,0.25)",
                                backdropFilter: "blur(20px)",
                              }}
                            >
                              <div style={{ display: "flex", alignItems: "flex-start", gap: 10, marginBottom: 10 }}>
                                <span style={{ fontSize: 15, flexShrink: 0, marginTop: 1 }}>⚠️</span>
                                <div style={{ flex: 1 }}>
                                  <p style={{ fontSize: 12, fontWeight: 700, color: "rgba(251,191,36,0.90)", marginBottom: 6 }}>
                                    Data Conflicts Detected
                                  </p>
                                  <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "6px 12px" }}>
                                    {conflicts.map((c, i) => (
                                      <div key={i} style={{
                                        padding: "8px 10px",
                                        borderRadius: 8,
                                        background: "rgba(251,191,36,0.06)",
                                        border: "0.5px solid rgba(251,191,36,0.15)",
                                      }}>
                                        <p style={{ fontSize: 10, fontWeight: 700, color: "rgba(251,191,36,0.80)", marginBottom: 3, textTransform: "uppercase", letterSpacing: "0.06em" }}>{c.field}</p>
                                        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                                          {c.values.map((v, vi) => (
                                            <span key={vi} style={{ fontSize: 11, padding: "2px 7px", borderRadius: 4, background: vi === 0 ? "rgba(251,191,36,0.15)" : "rgba(255,255,255,0.07)", color: vi === 0 ? "rgba(251,191,36,0.85)" : "rgba(255,255,255,0.55)", fontWeight: 600 }}>{v}</span>
                                          ))}
                                        </div>
                                        {c.sources?.length > 0 && (
                                          <p style={{ fontSize: 9, color: "rgba(255,255,255,0.28)", marginTop: 3 }}>{c.sources.join(", ")}</p>
                                        )}
                                      </div>
                                    ))}
                                  </div>
                                </div>
                              </div>
                              <button
                                onClick={() => {
                                  const resolveIntent = `Verify and resolve these conflicting data points: ${conflictSummary}. Find authoritative sources and return the correct values.`;
                                  setIntent(resolveIntent);
                                  setSidebarOpen(false);
                                  setTimeout(() => textareaRef.current?.focus(), 80);
                                }}
                                style={{
                                  marginTop: 4,
                                  padding: "7px 14px",
                                  borderRadius: 8,
                                  background: "rgba(251,191,36,0.12)",
                                  border: "1px solid rgba(251,191,36,0.30)",
                                  color: "rgba(251,191,36,0.85)",
                                  fontSize: 11, fontWeight: 700,
                                  cursor: "pointer",
                                  letterSpacing: "0.04em",
                                  transition: "background 0.15s",
                                }}
                                onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.background = "rgba(251,191,36,0.20)"; }}
                                onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.background = "rgba(251,191,36,0.12)"; }}
                              >
                                🔍 Launch Conflict Resolution Mission →
                              </button>
                            </motion.div>
                          );
                        })()}

                        {/* Action Bar — completed missions only */}
                        {(missionStatus === "complete" || isRefining) && (
                          <ActionBar
                            intent={missionMeta?.intent ?? intent}
                            missionState={missionState}
                            missionId={activeMissionId ?? missionMeta?.mission_id}
                            onRefine={runRefinement}
                            isRefining={isRefining}
                            refineError={refineError}
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
                          <p className="text-[10px] font-semibold uppercase tracking-[0.14em]" style={{ color: "rgba(147,153,160,0.50)", fontFamily: "var(--axion-font-mono)" }}>
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
                                  ? "linear-gradient(rgba(255,255,255,0.05), rgba(255,255,255,0.02))"
                                  : "linear-gradient(rgba(255,255,255,0.04), rgba(255,255,255,0.016))",
                                border: isRunning
                                  ? `1px solid rgba(var(--axion-accent-rgb, 167,202,220), 0.35)`
                                  : "1px solid rgba(255,255,255,0.08)",
                                backdropFilter: "blur(var(--axion-blur, 28px)) saturate(130%)",
                                WebkitBackdropFilter: "blur(var(--axion-blur, 28px)) saturate(130%)",
                                padding: "var(--axion-pad, 20px)",
                                boxShadow: "inset 0 1px 0 0 rgba(255,255,255,0.06), 0 0 0 1px rgba(255,255,255,0.04), 0 20px 40px -15px rgba(0,0,0,0.50)",
                              }}
                            >
                              <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-200 mb-3">
                                <span>{icon}</span>
                                <span>{label}</span>
                                {isRunning && (
                                  <span className="ml-auto flex items-center gap-1.5 text-xs font-normal"
                                    style={{ color: "var(--axion-accent, #a7cadc)" }}>
                                    <span className="w-2 h-2 rounded-full inline-block"
                                      style={{ background: "var(--axion-accent, #a7cadc)", animation: "neural-pulse 1.4s ease-in-out infinite" }} />
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
                                (() => {
                                  const displayed = displayedResults[slug];
                                  const isAnimating =
                                    card.status === "completed" &&
                                    displayed !== undefined &&
                                    displayed.length < card.result.length;
                                  const isDone =
                                    card.status === "completed" &&
                                    displayed === card.result;

                                  if (isDone) {
                                    // Animation finished — render full markdown
                                    return (
                                      <ReactMarkdown
                                        remarkPlugins={[remarkGfm]}
                                        components={mdComponents}
                                      >
                                        {card.result}
                                      </ReactMarkdown>
                                    );
                                  }

                                  if (card.status === "completed" && displayed !== undefined) {
                                    // Animating — raw text + blinking cursor
                                    return (
                                      <p className="text-sm text-gray-300 whitespace-pre-wrap leading-relaxed break-words">
                                        {displayed}
                                        <span className="axion-cursor" />
                                      </p>
                                    );
                                  }

                                  // Fallback (no displayedResults entry yet) — full result
                                  return (
                                    <ReactMarkdown
                                      remarkPlugins={[remarkGfm]}
                                      components={mdComponents}
                                    >
                                      {card.result}
                                    </ReactMarkdown>
                                  );
                                })()
                              ) : null}
                            </motion.article>
                          );
                        })}
                      </div>
                    );
                  })()}

                  {/* ── Execution Trace Panel ───────────────────────────── */}
                  {traceEvents.length > 0 && (
                    <div style={{ marginTop: 24, paddingBottom: 8 }}>
                      {/* Toggle header */}
                      <div
                        onClick={() => setTraceOpen((o) => !o)}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 8,
                          cursor: "pointer",
                          padding: "10px 0",
                          borderTop: "1px solid rgba(255,255,255,0.07)",
                          userSelect: "none",
                        }}
                      >
                        <span style={{ fontSize: 11, color: "rgba(255,255,255,0.35)", letterSpacing: "0.08em", textTransform: "uppercase" }}>
                          Execution trace ({traceEvents.length} events)
                        </span>
                        <span style={{
                          marginLeft: "auto",
                          fontSize: 11,
                          color: "rgba(255,255,255,0.25)",
                          display: "inline-block",
                          transform: traceOpen ? "rotate(180deg)" : "none",
                          transition: "transform 0.2s",
                        }}>
                          ▾
                        </span>
                      </div>

                      {/* Collapsible body */}
                      {traceOpen && (
                        <div style={{ paddingTop: 4 }}>
                          {traceEvents.map((event) => (
                            <div
                              key={event.id}
                              style={{
                                display: "flex",
                                alignItems: "flex-start",
                                gap: 12,
                                padding: "5px 0",
                                borderBottom: "1px solid rgba(255,255,255,0.04)",
                              }}
                            >
                              {/* Timestamp */}
                              <span style={{
                                fontSize: 10,
                                color: "rgba(255,255,255,0.2)",
                                minWidth: 52,
                                fontVariantNumeric: "tabular-nums",
                                paddingTop: 1,
                                fontFamily: "monospace",
                              }}>
                                {event.timestamp < 10
                                  ? `#${event.timestamp}`    // synthetic from loadMission
                                  : formatTraceTime(event.timestamp)}
                              </span>
                              {/* Dot */}
                              <div style={{
                                width: 6,
                                height: 6,
                                borderRadius: "50%",
                                marginTop: 4,
                                flexShrink: 0,
                                background: dotColorForType(event.type),
                              }} />
                              {/* Label + optional duration */}
                              <span style={{
                                fontSize: 12,
                                color: "rgba(255,255,255,0.55)",
                                lineHeight: 1.4,
                                flex: 1,
                              }}>
                                {event.label}
                                {event.durationMs !== undefined && (
                                  <span style={{ marginLeft: 8, fontSize: 10, color: "rgba(255,255,255,0.25)" }}>
                                    {event.durationMs}ms
                                  </span>
                                )}
                              </span>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

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
          background: "rgba(6,8,11,1)",
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
            background: "linear-gradient(to bottom, transparent, rgba(6,8,11,0.96))",
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
