"use client";

import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { motion, AnimatePresence } from "framer-motion";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Renderer, UIBlueprint, UIComponent } from "@/components/Renderer";
import { ActionBar } from "@/components/ActionBar";
import { TemplateGallery } from "@/components/TemplateGallery";
import { ModelSelector } from "@/components/ModelSelector";
import { MODEL_CATALOG, type ModelId } from "@/data/models";
import { ClarifyModal } from "@/components/ClarifyModal";
import { ModeSelector } from "@/components/ModeSelector";
import { OrchestrationGraph } from "@/components/OrchestrationGraph";
import { OpalZeroClient } from "@opalzero/sdk";
import { useMission } from "@opalzero/sdk/react";
import type {
  MissionEvent,
  TaskStartedEvent,
  TaskCompletedEvent,
  TaskFailedEvent,
  GovernorExpandEvent,
  MissionCompleteEvent,
  MissionFailedEvent,
  AwaitingFeedbackEvent,
} from "@opalzero/sdk";

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


// ── Clarification ─────────────────────────────────────────────────────────────

interface ClarifyQuestion {
  key:         string;
  question:    string;
  required:    boolean;
  placeholder: string;
}

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

  root.style.setProperty("--opalzero-accent",       tokens.primary_accent);
  root.style.setProperty("--opalzero-accent-rgb",   `${r},${g},${b}`);
  root.style.setProperty("--opalzero-accent-fg",    accentFg);
  root.style.setProperty("--opalzero-blur",         `${blur}px`);
  root.style.setProperty("--opalzero-glass-bg",     `rgba(255,255,255,${bgAlpha})`);
  root.style.setProperty("--opalzero-glass-border", `rgba(255,255,255,${borderAlpha})`);
  root.style.setProperty("--opalzero-glass-inset",  `0 1px 0 0 rgba(255,255,255,0.06) inset, 0 0 0 1px rgba(255,255,255,0.05), 0 40px 80px -30px rgba(0,0,0,0.70)`);
  root.style.setProperty("--opalzero-glow",         `rgba(${r},${g},${b},0.35)`);
  root.style.setProperty("--opalzero-pad",          compact ? "16px" : "24px");
  root.style.setProperty("--opalzero-gap",          compact ? "20px" : "36px");
  root.style.setProperty("--opalzero-radius",       `${radius}px`);
}

function resetDesignTokens() {
  const props = [
    "--opalzero-accent", "--opalzero-accent-rgb", "--opalzero-accent-fg", "--opalzero-blur",
    "--opalzero-glass-bg", "--opalzero-glass-border", "--opalzero-glass-inset", "--opalzero-glow",
    "--opalzero-pad", "--opalzero-gap", "--opalzero-radius",
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

/** Humanize a snake_case payload key into a readable label with unit suffixes. */
function humanizeKey(key: string): string {
  const unitMap: Record<string, string> = {
    "_usd": " (USD)", "_eur": " (EUR)", "_pct": " (%)",
    "_gbp": " (GBP)", "_count": "", "_rate": " Rate",
  };
  let label = key.toLowerCase();
  let unit = "";
  for (const [suffix, replacement] of Object.entries(unitMap)) {
    if (label.endsWith(suffix)) {
      label = label.slice(0, -suffix.length);
      unit = replacement;
      break;
    }
  }
  return label
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ") + unit;
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
    // Skip special keys handled outside the grid (sources pills, conflict banner)
    if (key === "sources" || key === "data_conflicts") continue;
    const label = humanizeKey(key);
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
          props: { title: label, headers: headers.map(humanizeKey), rows },
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
          metric: humanizeKey(k),
          value: typeof v === "number" ? v : Number(v),
        }));
        components.push({
          component_type: "ChartCard",
          props: { title: label, data: chartData, chartType: "bar", xKey: "metric", dataKeys: ["value"] },
        });
        continue;
      }

      const rows = Object.entries(obj).map(([k, v]) => [humanizeKey(k), String(v ?? "")]);
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
  const onEventRef = useRef<(e: MissionEvent) => void>(() => {});
  const keysBeforeRef = useRef<Set<string>>(new Set());
  const refinementIntentRef = useRef<string>("");
  const pendingAttachmentRef = useRef<typeof uploadedFile>(null);
  const attachmentReleasedRef = useRef(false);
  const [streamCards, setStreamCards] = useState<Record<string, StreamCard>>({});
  const [cardOrder, setCardOrder] = useState<string[]>([]);
  /** Tracks the animated (partially-revealed) portion of each completed task result. */
  const [displayedResults, setDisplayedResults] = useState<Record<string, string>>({});
  const [missionMeta, setMissionMeta] = useState<MissionMeta | null>(null);
  const [showDetails, setShowDetails] = useState(false);
  const [governorBanner, setGovernorBanner] = useState<string | null>(null);
  const [history, setHistory] = useState<MissionSummary[]>([]);
  // State for historical missions loaded via loadMission() — hook state wins during live runs
  const [histMissionState, setHistMissionState] = useState<MissionState | null>(null);
  const [histActiveMissionId, setHistActiveMissionId] = useState<string | null>(null);
  const [localFetchError, setLocalFetchError] = useState<string | null>(null);
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
  const [selectedModel, setSelectedModel] = useState<ModelId>("gpt-4o-mini");

  // ── Clarification pre-flight ───────────────────────────────────────────────
  const [clarifyQuestions, setClarifyQuestions]   = useState<ClarifyQuestion[]>([]);
  const [clarifyAnswers, setClarifyAnswers]       = useState<Record<string, string>>({});
  const [showClarifyModal, setShowClarifyModal]   = useState(false);
  const [isClarifying, setIsClarifying]           = useState(false);
  /** Set to true before closing the modal so the post-close useEffect fires runMission(). */
  const pendingRunRef = useRef(false);
  /** Set to true once the user has passed the clarify gate — prevents re-checking on the second runMission() call. */
  const clarifyPassedRef = useRef(false);

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

  // ── SDK hook ─────────────────────────────────────────────────────────────────
  // buildClient() is a function declaration below — hoisted to top of Home() scope.
  // useMemo rebuilds when settingsOpen closes so API key changes take effect.
  const hookClient = useMemo(
    () => typeof window !== "undefined"
      ? buildClient()
      : new OpalZeroClient({ baseUrl: "http://localhost:8080" }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [settingsOpen]
  );
  const missionHook = useMission({ client: hookClient, onEvent: (e) => onEventRef.current(e) });
  // Derived: hook state wins during live runs; hist* state is used for loaded historical missions
  const missionState = missionHook.missionState ?? histMissionState;
  const activeMissionId = missionHook.missionId ?? histActiveMissionId;
  const activeAgent = missionHook.activeAgent;
  const fetchError = missionHook.error ?? localFetchError;

  const [configStatus, setConfigStatus] = useState<{ openai: boolean; tavily: boolean; alpha_vantage?: boolean } | null>(null);
  const [draftOpalzeroKey, setDraftOpalzeroKey] = useState("");
  const [draftOpenAI, setDraftOpenAI] = useState("");
  const [draftTavily, setDraftTavily] = useState("");
  const [draftAlphaVantage, setDraftAlphaVantage] = useState("");
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

  // ── Refine-nav dropdown ────────────────────────────────────────────────────
  const [refineNavOpen, setRefineNavOpen] = useState(false);
  const [refineNavText, setRefineNavText] = useState("");
  const refineNavInputRef = useRef<HTMLTextAreaElement>(null);

  // ── History search & clear ─────────────────────────────────────────────────
  const [historyQuery, setHistoryQuery] = useState("");
  const [historyQueryFocused, setHistoryQueryFocused] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  // Auto-focus the refine-nav textarea when the panel opens
  useEffect(() => {
    if (refineNavOpen) setTimeout(() => refineNavInputRef.current?.focus(), 80);
  }, [refineNavOpen]);

  // Close refine-nav panel when a refinement starts streaming
  useEffect(() => {
    if (isRefining) setRefineNavOpen(false);
  }, [isRefining]);

  // After the clarify modal closes with pendingRunRef set, fire runMission().
  useEffect(() => {
    if (!showClarifyModal && pendingRunRef.current) {
      pendingRunRef.current = false;
      runMission();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showClarifyModal]);

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
    missionHook.reset();
    setHistMissionState(null);
    setHistActiveMissionId(null);
    setLocalFetchError(null);
    setGovernorBanner(null);
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
    if (missionState?.design_tokens) applyDesignTokens(missionState.design_tokens as DesignTokens);
  }, [missionState]);

  // Sync missionStatus from hook lifecycle
  useEffect(() => {
    if (missionHook.status === "running")  setMissionStatus("streaming");
    if (missionHook.status === "complete") setMissionStatus("complete");
    if (missionHook.status === "failed")   setMissionStatus("failed");
  }, [missionHook.status]);

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
      const storedOpalzeroKey = localStorage.getItem("opalzero_api_key") ?? "";
      const storedOpenAI = localStorage.getItem("opalzero_openai_key") ?? "";
      const storedTavily       = localStorage.getItem("opalzero_tavily_key")        ?? "";
      const storedAlphaVantage = localStorage.getItem("opalzero_alpha_vantage_key") ?? "";
      if (storedOpalzeroKey)    setDraftOpalzeroKey(storedOpalzeroKey);
      if (storedOpenAI)      setDraftOpenAI(storedOpenAI);
      if (storedTavily)      setDraftTavily(storedTavily);
      if (storedAlphaVantage) setDraftAlphaVantage(storedAlphaVantage);
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
    missionHook.reset();
    setHistActiveMissionId(id);
    setLocalFetchError(null);
    setGovernorBanner(null);
    setHistMissionState(null);
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
        setHistMissionState(data.mission_state as MissionState);
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
    } catch (e) {
      setLocalFetchError(e instanceof Error ? e.message : "Could not load mission.");
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
    const icon  = "/opalzero-logo.png";

    try {
      const n = new Notification(title, { body, icon });
      setTimeout(() => n.close(), 6000);
      n.onclick = () => { window.focus(); n.close(); };
    } catch {
      // OS may block even with permission granted — ignore silently
    }
  }

  /** Builds a fully-configured OpalZeroClient from current localStorage keys. */
  function buildClient(): OpalZeroClient {
    return new OpalZeroClient({
      baseUrl:   "http://localhost:8080",
      apiKey:    localStorage.getItem("opalzero_api_key")    ?? undefined,
      openAiKey: localStorage.getItem("opalzero_openai_key") ?? undefined,
      tavilyKey:       localStorage.getItem("opalzero_tavily_key")        ?? undefined,
      alphaVantageKey: localStorage.getItem("opalzero_alpha_vantage_key") ?? undefined,
    });
  }

  async function runMission() {
    if (!intent.trim() || missionStatus === "streaming") return;

    // ── Pre-flight clarification check ──────────────────────────────────────
    // Skip if the user has already passed the clarify gate for this execution.
    if (!clarifyPassedRef.current) {
      setIsClarifying(true);
      try {
        const clarifyHeaders: Record<string, string> = { "Content-Type": "application/json" };
        const storedKey     = localStorage.getItem("opalzero_api_key");
        const storedOpenAI  = localStorage.getItem("opalzero_openai_key");
        if (storedKey)    clarifyHeaders["X-Axion-Key"]  = storedKey;
        if (storedOpenAI) clarifyHeaders["X-OpenAI-Key"] = storedOpenAI;

        const res = await fetch("http://localhost:8080/api/v1/clarify", {
          method:  "POST",
          headers: clarifyHeaders,
          body:    JSON.stringify({ intent }),
          signal:  AbortSignal.timeout(8000), // fail open after 8 s
        });
        if (res.ok) {
          const data = await res.json() as { complete: boolean; questions?: ClarifyQuestion[] };
          if (!data.complete && data.questions && data.questions.length > 0) {
            setClarifyQuestions(data.questions);
            setClarifyAnswers({});
            setShowClarifyModal(true);
            setIsClarifying(false);
            return; // wait for user to answer or skip
          }
        }
      } catch {
        // Fail open — proceed to execute as normal
      }
      setIsClarifying(false);
    }
    // ────────────────────────────────────────────────────────────────────────

    // Request notification permission on first Execute — fire-and-forget, non-blocking
    if (typeof window !== "undefined" && "Notification" in window
        && Notification.permission === "default") {
      Notification.requestPermission().then((perm) => setNotifPermission(perm));
    }

    clarifyPassedRef.current = false; // reset for the next fresh execution
    setMissionStatus("streaming");
    setStreamCards({});
    setCardOrder([]);
    setMissionMeta(null);
    setHistMissionState(null);
    setHistActiveMissionId(null);
    setLocalFetchError(null);
    setShowDetails(false);
    setGovernorBanner(null);
    userScrolledRef.current = false;
    setTraceEvents([]);
    setTraceOpen(false);
    taskStartTimes.current = {};

    // Inject vision / data-file tool hints when an attachment is present
    const effectiveIntent = uploadedFile
      ? uploadedFile.fileType === "image"
        ? `${intent}\n\n[Image attached: ${uploadedFile.filename}. Use the vision tool to analyse this image as part of your research.]`
        : `${intent}\n\n[Data file attached: ${uploadedFile.filename} (original: ${uploadedFile.originalName}). Use the read_file tool with filename="${uploadedFile.filename}" to load its contents before analysis.]`
      : intent;

    // Snapshot attachment so onEvent callback can release it on the first event
    pendingAttachmentRef.current = uploadedFile;
    attachmentReleasedRef.current = false;

    try {
      await missionHook.run(effectiveIntent, selectedModel);
    } catch (e) {
      setMissionStatus("failed");
      showMissionNotification("failed", intent);
    }
  }

  // ── Clarification submit ──────────────────────────────────────────────────────

  function runMissionWithAnswers() {
    // Append non-empty answers to the intent string.
    const answerLines = clarifyQuestions
      .filter((q) => clarifyAnswers[q.key]?.trim())
      .map((q) => `${q.question} ${clarifyAnswers[q.key].trim()}`);

    if (answerLines.length > 0) {
      setIntent((prev) => `${prev}. Additional context: ${answerLines.join(". ")}`);
    }

    // Mark clarify as passed so the second runMission() call skips the check.
    clarifyPassedRef.current = true;
    pendingRunRef.current = true;
    setClarifyQuestions([]);
    setShowClarifyModal(false);
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

  // ── SSE event handler (shared by run + refine via onEventRef) ────────────────

  function handleSSEEvent(event: MissionEvent) {
    const now = Date.now();

    // Release pending attachment on the first event (server accepted the request)
    if (!attachmentReleasedRef.current && pendingAttachmentRef.current) {
      attachmentReleasedRef.current = true;
      if (pendingAttachmentRef.current.previewUrl) URL.revokeObjectURL(pendingAttachmentRef.current.previewUrl);
      setUploadedFile(null);
      pendingAttachmentRef.current = null;
    }

    switch (event.type) {
      case "task_started": {
        const e = event as TaskStartedEvent;
        setStreamCards((prev) => ({
          ...prev,
          [e.slug]: { slug: e.slug, role: e.role, intent: e.intent, status: "running" },
        }));
        setCardOrder((prev) => (prev.includes(e.slug) ? prev : [...prev, e.slug]));
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
          setShowDetails(false);
          // If this is a refinement, compute and store new/changed keys
          if (isRefining && e.mission_state.data_payload) {
            const keysAfter = new Set(Object.keys(e.mission_state.data_payload));
            const newKeys = new Set([...keysAfter].filter(k => !keysBeforeRef.current.has(k)));
            setRefinedPayloadKeys(newKeys);
            setRefinementHistory((prev) => [
              ...prev,
              { intent: refinementIntentRef.current, timestamp: now, newPayloadKeys: [...newKeys] },
            ]);
          }
        } else {
          setShowDetails(true);
        }
        showMissionNotification("complete", isRefining ? refinementIntentRef.current : intent);
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
        showMissionNotification("failed", isRefining ? refinementIntentRef.current : intent);
        if (isRefining) setRefineError((event as MissionFailedEvent).error ?? "Refinement failed.");
        setTraceEvents((prev) => [...prev, {
          id: `${now}-mission-failed`,
          timestamp: now,
          type: "mission_failed",
          label: "◉ Mission failed",
        }]);
        break;
      }
      case "awaiting_feedback":
        setTraceEvents((prev) => [...prev, {
          id: `${now}-feedback`,
          timestamp: now,
          type: "awaiting_feedback",
          label: "⏸ Awaiting human input",
        }]);
        break;
      default:
        setTraceEvents((prev) => [...prev, {
          id: `${now}-${event.type}`,
          timestamp: now,
          type: event.type as string,
          label: `· ${event.type}`,
        }]);
    }
  }
  // Always keep the ref pointing to the latest closure
  onEventRef.current = handleSSEEvent;

  // ── Refinement ────────────────────────────────────────────────────────────────

  async function runRefinement(refinementIntent: string) {
    const targetId = activeMissionId ?? missionMeta?.mission_id;
    if (!targetId || isRefining || missionStatus === "streaming") return;

    setIsRefining(true);
    setRefineError(null);

    // Snapshot payload keys BEFORE refinement so onEvent can diff on mission_complete
    keysBeforeRef.current = new Set(Object.keys(missionState?.data_payload ?? {}));
    refinementIntentRef.current = refinementIntent;

    try {
      await missionHook.refine(targetId, refinementIntent, selectedModel);
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
    if (type === "mission_complete") return "var(--opalzero-accent, #a7cadc)";
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
      run: () => { const id = activeMissionId ?? missionMeta?.mission_id; if (id) { buildClient().missions.export(id, "md").then(b=>{const u=URL.createObjectURL(b);const a=document.createElement("a");a.href=u;a.download=`opalzero-${id}.md`;a.click();URL.revokeObjectURL(u);}); } } },
    { cmd: "/export csv",  icon: "↓", desc: "Download mission results as CSV",        accent: "#6ee7b7", category: "export"   as const,
      run: () => { const id = activeMissionId ?? missionMeta?.mission_id; if (id) { buildClient().missions.export(id, "csv").then(b=>{const u=URL.createObjectURL(b);const a=document.createElement("a");a.href=u;a.download=`opalzero-${id}.csv`;a.click();URL.revokeObjectURL(u);}); } } },
    { cmd: "/export html", icon: "↓", desc: "Download mission results as HTML page",  accent: "#6ee7b7", category: "export"   as const,
      run: () => { const id = activeMissionId ?? missionMeta?.mission_id; if (id) { buildClient().missions.export(id, "html").then(b=>{const u=URL.createObjectURL(b);const a=document.createElement("a");a.href=u;a.download=`opalzero-${id}.html`;a.click();URL.revokeObjectURL(u);}); } } },
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
                  ? "var(--opalzero-accent, #a7cadc)"
                  : "rgba(255,255,255,0.75)",
                fontFamily: "var(--opalzero-font-mono, monospace)",
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
          ? "opalzero-light-trace-active opalzero-breathe-glow"
          : "opalzero-light-trace"
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
              ? "var(--opalzero-accent, #a7cadc)"
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
                ? "var(--opalzero-accent, #a7cadc)"
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
          disabled={isStreaming || isClarifying || !intent.trim()}
          className="shrink-0 text-sm font-semibold px-5 py-2 rounded-xl transition-all
                     disabled:opacity-40 disabled:cursor-not-allowed"
          style={{
            background: isStreaming || isClarifying
              ? "rgba(255,255,255,0.06)"
              : "var(--opalzero-accent, #a7cadc)",
            color: isStreaming || isClarifying ? "rgba(255,255,255,0.4)" : "var(--opalzero-accent-fg, #07090c)",
            boxShadow: isStreaming || isClarifying
              ? "none"
              : "0 0 20px rgba(var(--opalzero-accent-rgb, 167,202,220), 0.30)",
          }}
        >
          {isStreaming ? "Running…" : isClarifying ? "Checking…" : "Execute"}
        </button>
      </div>
    </div>
    </div>
  );

  // ── Settings helpers ─────────────────────────────────────────────────────────

  function saveSettings() {
    const opalzeroKeyTrimmed      = draftOpalzeroKey.trim();
    const openAITrimmed        = draftOpenAI.trim();
    const tavilyTrimmed        = draftTavily.trim();
    const alphaVantageTrimmed  = draftAlphaVantage.trim();
    if (opalzeroKeyTrimmed) {
      localStorage.setItem("opalzero_api_key", opalzeroKeyTrimmed);
    } else {
      localStorage.removeItem("opalzero_api_key");
    }
    if (openAITrimmed) {
      localStorage.setItem("opalzero_openai_key", openAITrimmed);
    } else {
      localStorage.removeItem("opalzero_openai_key");
    }
    if (tavilyTrimmed) {
      localStorage.setItem("opalzero_tavily_key", tavilyTrimmed);
    } else {
      localStorage.removeItem("opalzero_tavily_key");
    }
    if (alphaVantageTrimmed) {
      localStorage.setItem("opalzero_alpha_vantage_key", alphaVantageTrimmed);
    } else {
      localStorage.removeItem("opalzero_alpha_vantage_key");
    }
    setSettingsSaved(true);
    setTimeout(() => setSettingsSaved(false), 2000);
    setSettingsOpen(false);
  }

  // Badge: shown when OpenAI is not configured via env AND not stored locally
  const showConfigBadge = configStatus?.openai === false && !draftOpenAI.trim();

  // ── Streaming status label (inferred from active agent) ───────────────────
  // ── Helper: export mission ───────────────────────────────────────────────
  function handleExport() {
    const id = activeMissionId ?? missionMeta?.mission_id;
    if (id) {
      buildClient().missions.export(id, "md").then(b => {
        const u = URL.createObjectURL(b);
        const a = document.createElement("a");
        a.href = u; a.download = `opalzero-${id}.md`; a.click(); URL.revokeObjectURL(u);
      }).catch(() => missionState && (downloadMarkdownFallback()));
    }
  }
  function downloadMarkdownFallback() {
    if (!missionState) return;
    const lines: string[] = [`# Mission Report: ${intent}`, "", `_Generated by Axion — ${new Date().toLocaleDateString()}_`, ""];
    for (const [key, value] of Object.entries(missionState.data_payload)) {
      const title = key.replace(/_/g, " ").replace(/\b\w/g, c => c.toUpperCase());
      lines.push(`## ${title}`, "", Array.isArray(value) ? value.map(v => `- ${JSON.stringify(v)}`).join("\n") : String(value), "");
    }
    const blob = new Blob([lines.join("\n")], { type: "text/markdown;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a"); a.href = url; a.download = `opalzero-report.md`; a.click(); URL.revokeObjectURL(url);
  }


  return (
    <div
      className="text-gray-100"
      style={{ display: "flex", flexDirection: "column", height: "100vh", overflow: "hidden", background: "transparent" }}
    >

      {/* ── Fixed glass nav ───────────────────────────────────────────────── */}
      <div
        style={{
          position: "fixed",
          top: 16,
          left: "50%",
          transform: "translateX(-50%)",
          width: "min(960px, calc(100% - 2rem))",
          zIndex: 100,
          pointerEvents: "none",
        }}
      >
        <div
          className="opalzero-glass"
          style={{
            borderRadius: 999,
            height: 54,
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "0 8px 0 20px",
            pointerEvents: "auto",
          }}
        >
          {/* Left: back link + wordmark */}
          <a
            href="https://albertobarnabo.it/axion/"
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              color: "rgba(255,255,255,0.82)",
              textDecoration: "none",
              fontFamily: "var(--opalzero-font-display)",
              fontWeight: 600,
              fontSize: 15,
              letterSpacing: "-0.02em",
              flexShrink: 0,
              transition: "color 0.15s",
            }}
            onMouseEnter={e => { (e.currentTarget as HTMLAnchorElement).style.color = "rgba(255,255,255,1)"; }}
            onMouseLeave={e => { (e.currentTarget as HTMLAnchorElement).style.color = "rgba(255,255,255,0.82)"; }}
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" style={{ opacity: 0.55 }}>
              <path d="M19 12H5M12 5l-7 7 7 7"/>
            </svg>
            Axion
          </a>

          {/* Centre: intent text (non-idle) */}
          <AnimatePresence mode="wait">
            {!isIdle && intent && (
              <motion.p
                key="nav-intent"
                initial={{ opacity: 0, y: -4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 4 }}
                transition={{ duration: 0.2 }}
                style={{
                  fontSize: 13,
                  color: "rgba(255,255,255,0.40)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  flex: "0 1 auto",
                  maxWidth: 340,
                  letterSpacing: "-0.01em",
                }}
              >
                {intent.length > 58 ? intent.slice(0, 58) + "…" : intent}
              </motion.p>
            )}
          </AnimatePresence>

          {/* Right: action icons */}
          <div style={{ display: "flex", gap: 6, alignItems: "center", flexShrink: 0 }}>

            {/* Model selector — always visible as a persistent model indicator */}
            <ModelSelector
              value={selectedModel}
              onChange={setSelectedModel}
              disabled={missionStatus === "streaming"}
            />

            {/* Refine — complete state only */}
            {missionStatus === "complete" && missionState && (
              <motion.button
                onClick={() => setRefineNavOpen(v => !v)}
                disabled={isRefining}
                whileHover={{ scale: 1.04 }}
                whileTap={{ scale: 0.96 }}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 5,
                  padding: "5px 14px",
                  borderRadius: 999,
                  background: refineNavOpen
                    ? "rgba(var(--opalzero-accent-rgb,167,202,220),0.15)"
                    : "rgba(255,255,255,0.07)",
                  border: `0.5px solid ${refineNavOpen
                    ? "rgba(var(--opalzero-accent-rgb,167,202,220),0.45)"
                    : "rgba(255,255,255,0.12)"}`,
                  color: isRefining
                    ? "var(--opalzero-accent,#a7cadc)"
                    : "rgba(255,255,255,0.72)",
                  fontSize: 12,
                  fontWeight: 500,
                  cursor: isRefining ? "default" : "pointer",
                  transition: "background 0.15s, border-color 0.15s",
                  marginRight: 2,
                }}
              >
                {isRefining ? (
                  <div style={{
                    width: 10, height: 10, borderRadius: "50%",
                    border: "1.5px solid var(--opalzero-accent,#a7cadc)",
                    borderTopColor: "transparent",
                    animation: "spin 0.8s linear infinite",
                  }} />
                ) : "⟳"}
                {isRefining ? "Refining…" : "Refine"}
              </motion.button>
            )}

            {/* Export — complete state only */}
            {missionStatus === "complete" && missionState && (
              <button
                onClick={handleExport}
                title="Export as Markdown"
                style={{
                  width: 34, height: 34, borderRadius: 10,
                  display: "flex", alignItems: "center", justifyContent: "center",
                  background: "rgba(255,255,255,0.06)",
                  border: "0.5px solid rgba(255,255,255,0.10)",
                  color: "rgba(255,255,255,0.48)",
                  cursor: "pointer", fontSize: 14,
                  transition: "background 0.15s, color 0.15s",
                }}
                onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.80)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.10)"; }}
                onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.48)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.06)"; }}
              >
                ↓
              </button>
            )}

            {/* History */}
            <button
              onClick={() => setSidebarOpen(v => !v)}
              title="Mission history"
              style={{
                width: 34, height: 34, borderRadius: 10,
                display: "flex", alignItems: "center", justifyContent: "center",
                background: sidebarOpen ? "rgba(255,255,255,0.10)" : "rgba(255,255,255,0.06)",
                border: "0.5px solid rgba(255,255,255,0.10)",
                color: sidebarOpen ? "rgba(255,255,255,0.80)" : "rgba(255,255,255,0.48)",
                cursor: "pointer", fontSize: 15,
                transition: "background 0.15s, color 0.15s",
              }}
            >
              ◫
            </button>

            {/* Settings */}
            <button
              onClick={() => setSettingsOpen(v => !v)}
              title="Settings"
              style={{
                width: 34, height: 34, borderRadius: 10,
                display: "flex", alignItems: "center", justifyContent: "center",
                background: "rgba(255,255,255,0.06)",
                border: "0.5px solid rgba(255,255,255,0.10)",
                color: "rgba(255,255,255,0.48)",
                cursor: "pointer", fontSize: 14,
                position: "relative",
                transition: "background 0.15s, color 0.15s",
              }}
              onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.80)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.10)"; }}
              onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.48)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.06)"; }}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
              </svg>
              {showConfigBadge && (
                <span style={{
                  position: "absolute", top: 5, right: 5,
                  width: 6, height: 6, borderRadius: "50%",
                  background: "#f87171", pointerEvents: "none",
                }} />
              )}
            </button>
          </div>
        </div>
      </div>

      {/* ── Refine nav dropdown ───────────────────────────────────────────── */}
      <AnimatePresence>
        {refineNavOpen && missionState && (
          <motion.div
            key="refine-nav"
            initial={{ opacity: 0, y: -10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.18 }}
            style={{
              position: "fixed",
              top: 80,
              left: "50%",
              transform: "translateX(-50%)",
              width: "min(640px, calc(100% - 2rem))",
              zIndex: 99,
              background: "rgba(7,10,16,0.97)",
              backdropFilter: "blur(40px) saturate(150%)",
              WebkitBackdropFilter: "blur(40px) saturate(150%)",
              border: "1px solid rgba(255,255,255,0.11)",
              borderRadius: 20,
              padding: "16px 20px 18px",
              boxShadow: "0 24px 60px rgba(0,0,0,0.65), 0 0 0 1px rgba(255,255,255,0.04)",
            }}
          >
            {/* Close on outside click */}
            <div
              style={{ position: "fixed", inset: 0, zIndex: -1 }}
              onClick={() => setRefineNavOpen(false)}
            />
            <div style={{ display: "flex", gap: 10 }}>
              <textarea
                ref={refineNavInputRef}
                rows={1}
                value={refineNavText}
                onChange={e => {
                  setRefineNavText(e.target.value);
                  const el = e.target; el.style.height = "auto";
                  el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
                }}
                onKeyDown={e => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    const t = refineNavText.trim();
                    if (t) { runRefinement(t); setRefineNavText(""); setRefineNavOpen(false); }
                  }
                  if (e.key === "Escape") setRefineNavOpen(false);
                }}
                placeholder="What else should this mission explore?"
                style={{
                  flex: 1, background: "transparent",
                  color: "rgba(255,255,255,0.90)", fontSize: 14,
                  lineHeight: "1.55", outline: "none", resize: "none",
                  minHeight: 24, maxHeight: 120, overflow: "hidden",
                  fontFamily: "var(--opalzero-font-main)",
                }}
                className="placeholder-gray-500"
              />
              <button
                onClick={() => {
                  const t = refineNavText.trim();
                  if (t) { runRefinement(t); setRefineNavText(""); setRefineNavOpen(false); }
                }}
                disabled={!refineNavText.trim()}
                style={{
                  alignSelf: "flex-end", padding: "6px 16px", borderRadius: 9,
                  background: refineNavText.trim() ? "var(--opalzero-accent,#a7cadc)" : "rgba(255,255,255,0.08)",
                  color: refineNavText.trim() ? "var(--opalzero-accent-fg,#07090c)" : "rgba(255,255,255,0.28)",
                  fontSize: 12, fontWeight: 700,
                  cursor: refineNavText.trim() ? "pointer" : "default",
                  transition: "background 0.15s, color 0.15s",
                  letterSpacing: "0.02em",
                }}
              >
                Refine →
              </button>
            </div>
            <div style={{ marginTop: 10, display: "flex", flexWrap: "wrap", gap: 6 }}>
              {["Compare alternatives side-by-side", "Add cost breakdown", "Include recent news", "Summarise key risks"].map(s => (
                <button key={s} onClick={() => setRefineNavText(s)}
                  style={{
                    fontSize: 11, padding: "4px 10px", borderRadius: 999,
                    background: "rgba(167,202,220,0.08)",
                    border: "0.5px solid rgba(167,202,220,0.22)",
                    color: "rgba(167,202,220,0.70)", cursor: "pointer",
                    transition: "background 0.12s",
                  }}
                  onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.background = "rgba(167,202,220,0.15)"; }}
                  onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.background = "rgba(167,202,220,0.08)"; }}
                >
                  {s}
                </button>
              ))}
            </div>
            {refineError && (
              <p style={{ marginTop: 8, fontSize: 11, color: "#f87171" }}>⚠ {refineError}</p>
            )}
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Main scrollable stage ─────────────────────────────────────────── */}
      <div
        ref={stageRef}
        style={{ flex: 1, minHeight: 0, overflowY: "auto", overflowX: "hidden", position: "relative", zIndex: 1 }}
      >

        {/* ── IDLE STATE ──────────────────────────────────────────────────── */}
        <AnimatePresence>
          {isIdle && (
            <motion.div
              key="idle-root"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0, scale: 0.97 }}
              transition={{ duration: 0.28 }}
              style={{
                minHeight: "100vh",
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                padding: "80px 24px 48px",
                textAlign: "center",
                position: "relative",
              }}
            >
              {/* Grid overlay — exact pattern from landing page Hero */}
              <div className="grid-faint" style={{ position: "absolute", inset: 0, pointerEvents: "none" }} />
              {/* Version badge */}
              <motion.span
                className="opalzero-mono-badge"
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.06, duration: 0.3 }}
                style={{ marginBottom: 28 }}
              >
                Kernel v0.1 · Alpha
              </motion.span>

              {/* Wordmark — landing page spec: Space Grotesk 400 */}
              <motion.h1
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.12, duration: 0.4 }}
                style={{
                  fontFamily: "var(--opalzero-font-display)",
                  fontSize: "clamp(4rem, 10vw, 7rem)",
                  fontWeight: 400,
                  letterSpacing: "-0.025em",
                  lineHeight: 1.02,
                  color: "rgba(235,239,242,0.96)",
                  marginBottom: 16,
                }}
              >
                Axion
              </motion.h1>

              {/* Tagline */}
              <motion.p
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ delay: 0.20, duration: 0.4 }}
                style={{
                  fontSize: 18,
                  color: "rgba(147,153,160,0.78)",
                  letterSpacing: "-0.01em",
                  fontWeight: 400,
                  maxWidth: 440,
                  lineHeight: 1.55,
                  marginBottom: 40,
                }}
              >
                The headless intelligence kernel. Synthesize intent into verified, structured state.
              </motion.p>

              {/* Command bar — centered, max-width 680 */}
              <motion.div
                initial={{ opacity: 0, y: 14 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.26, duration: 0.38 }}
                style={{ width: "100%", maxWidth: 680 }}
              >
                {commandBarContent}
              </motion.div>

              {/* Mode selector — 2×2 grid of vertical intelligence modes */}
              <motion.div
                initial={{ opacity: 0, y: 16 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.34, duration: 0.42 }}
                style={{ width: "100%", maxWidth: 840, marginTop: 44 }}
              >
                <ModeSelector
                  onSelect={(q) => {
                    // Fill the textarea with the selected query template.
                    // runMission() reads intent from its closure (React state), so we do
                    // NOT auto-submit here — the user edits the [bracket slots] first.
                    setIntent(q);
                    setTimeout(() => textareaRef.current?.focus(), 50);
                  }}
                />
              </motion.div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* ── STREAMING STATE ─────────────────────────────────────────────── */}
        <AnimatePresence>
          {isStreaming && (
            <motion.div
              key="streaming-root"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.22 }}
              style={{
                minHeight: "100vh",
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "flex-start",
                gap: 0,
                padding: "0 24px 48px",
                paddingTop: 100,
                textAlign: "center",
              }}
            >
              {/* Intent text */}
              <p style={{
                fontSize: "clamp(0.95rem, 2vw, 1.2rem)",
                fontWeight: 500,
                color: "rgba(235,239,242,0.82)",
                maxWidth: 560,
                lineHeight: 1.45,
                letterSpacing: "-0.02em",
                fontFamily: "var(--opalzero-font-display)",
                marginBottom: 8,
              }}>
                {intent}
              </p>

              {/* Governor expand banner */}
              {governorBanner && (
                <p style={{ fontSize: 12, color: "rgba(167,202,220,0.55)", maxWidth: 380, marginBottom: 12 }}>
                  {governorBanner}
                </p>
              )}

              {/* Live orchestration graph */}
              <div style={{ width: "100%", maxWidth: 760 }}>
                <OrchestrationGraph
                  cards={streamCards}
                  cardOrder={cardOrder}
                  activeAgent={activeAgent}
                  intent={intent}
                />
              </div>
            </motion.div>
          )}
        </AnimatePresence>

        {/* ── COMPLETE STATE ──────────────────────────────────────────────── */}
        <AnimatePresence>
          {missionStatus === "complete" && (
            <motion.div
              key="complete-root"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.35 }}
              style={{ paddingTop: 86, paddingBottom: 72 }}
            >
              {/* Fetch error */}
              {fetchError && (
                <div style={{ maxWidth: 720, margin: "0 auto 24px", padding: "0 24px" }}>
                  <div style={{
                    borderRadius: 14,
                    background: "rgba(220,38,38,0.08)",
                    border: "1px solid rgba(220,38,38,0.25)",
                    padding: "14px 18px",
                  }}>
                    <p style={{ fontSize: 13, color: "#fca5a5" }}>❌ {fetchError}</p>
                  </div>
                </div>
              )}

              {/* ── Mission trace — graph survives completion ──────────────────── */}
              {cardOrder.length > 0 && (() => {
                // slug → durationMs from traceEvents
                const timings: Record<string, number> = {};
                for (const e of traceEvents) {
                  if (e.slug && e.durationMs !== undefined) timings[e.slug] = e.durationMs;
                }
                const agentCount  = cardOrder.length;
                const totalMs     = Object.values(timings).reduce((a, b) => a + b, 0);
                const totalSec    = totalMs > 0 ? (totalMs / 1000).toFixed(1) : null;
                return (
                  <div style={{ padding: "0 24px", maxWidth: 1280, margin: "0 auto 32px" }}>
                    <div style={{
                      borderRadius: 14,
                      background: "rgba(167,202,220,0.03)",
                      border: "0.5px solid rgba(167,202,220,0.12)",
                      overflow: "hidden",
                    }}>
                      {/* Header row */}
                      <div style={{
                        padding: "10px 16px",
                        display: "flex",
                        alignItems: "center",
                        gap: 10,
                        borderBottom: "0.5px solid rgba(167,202,220,0.08)",
                      }}>
                        <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: "0.10em", textTransform: "uppercase", color: "rgba(110,231,183,0.70)" }}>
                          Mission Trace
                        </span>
                        <span style={{ fontSize: 10, color: "rgba(167,202,220,0.45)", fontFamily: "var(--opalzero-font-mono, monospace)" }}>
                          {agentCount} agent{agentCount !== 1 ? "s" : ""}
                          {totalSec ? ` · ${totalSec}s total` : ""}
                        </span>
                      </div>
                      {/* Graph */}
                      <div style={{ padding: "4px 16px 12px" }}>
                        <OrchestrationGraph
                          cards={streamCards}
                          cardOrder={cardOrder}
                          activeAgent={null}
                          intent={intent}
                          timings={timings}
                          traceMode
                        />
                      </div>
                    </div>
                  </div>
                );
              })()}

              {/* Bento grid */}
              {missionState && (() => {
                const blueprint = applicationMapper(
                  missionState.data_payload,
                  missionState.suggested_widgets ?? [],
                  missionState.design_tokens?.layout_strategy,
                );
                return blueprint.components.length > 0 ? (
                  <div
                    className={missionStatus === "complete" && !isRefining ? "opalzero-sheen-wrapper" : ""}
                    style={{ padding: "0 24px", maxWidth: 1280, margin: "0 auto" }}
                  >
                    {/* Sources pill row */}
                    {Array.isArray((missionState.data_payload as Record<string, unknown>).sources) && (() => {
                      const sources = (missionState.data_payload as Record<string, unknown>).sources as { label: string; url: string }[];
                      if (sources.length === 0) return null;
                      return (
                        <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginBottom: 20 }}>
                          {sources.map((s, si) => (
                            <a
                              key={si}
                              href={s.url}
                              target="_blank"
                              rel="noopener noreferrer"
                              className="opalzero-source-pill"
                            >
                              {s.label} ↗
                            </a>
                          ))}
                        </div>
                      );
                    })()}

                    <Renderer
                      blueprint={blueprint}
                      pinnedCards={pinnedCards}
                      dismissedCards={dismissedCards}
                      refinedIndices={(() => {
                        const idxSet = new Set<number>();
                        if (refinedPayloadKeys.size > 0) {
                          blueprint.components.forEach((c, idx) => {
                            const title = (c.props.title as string | undefined) ?? "";
                            for (const key of refinedPayloadKeys) {
                              const fk = key.replace(/_/g, " ").replace(/\b\w/g, ch => ch.toUpperCase());
                              if (title === fk || title.toLowerCase().includes(key.toLowerCase())) idxSet.add(idx);
                            }
                          });
                        }
                        return idxSet;
                      })()}
                      onPin={i => setPinnedCards(prev => { const n = new Set(prev); n.has(i) ? n.delete(i) : n.add(i); return n; })}
                      onDismiss={i => {
                        if (i < 0) setDismissedCards(new Set());
                        else setDismissedCards(prev => { const n = new Set(prev); n.add(i); return n; });
                      }}
                    />

                    {/* Data conflicts banner */}
                    {Array.isArray((missionState.data_payload as Record<string, unknown>).data_conflicts) && (() => {
                      const conflicts = (missionState.data_payload as Record<string, unknown>).data_conflicts as { field: string; values: string[]; sources: string[] }[];
                      const conflictSummary = conflicts.map(c => `${c.field}: ${c.values.join(" vs ")}`).join("; ");
                      return (
                        <motion.div
                          initial={{ opacity: 0, y: 8 }}
                          animate={{ opacity: 1, y: 0 }}
                          style={{
                            marginTop: 20,
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
                              <p style={{ fontSize: 12, fontWeight: 700, color: "rgba(251,191,36,0.90)", marginBottom: 6 }}>Data Conflicts Detected</p>
                              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "6px 12px" }}>
                                {conflicts.map((c, i) => (
                                  <div key={i} style={{ padding: "8px 10px", borderRadius: 8, background: "rgba(251,191,36,0.06)", border: "0.5px solid rgba(251,191,36,0.15)" }}>
                                    <p style={{ fontSize: 10, fontWeight: 700, color: "rgba(251,191,36,0.80)", marginBottom: 3, textTransform: "uppercase", letterSpacing: "0.06em" }}>{c.field}</p>
                                    <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                                      {c.values.map((v, vi) => (
                                        <span key={vi} style={{ fontSize: 11, padding: "2px 7px", borderRadius: 4, background: vi === 0 ? "rgba(251,191,36,0.15)" : "rgba(255,255,255,0.07)", color: vi === 0 ? "rgba(251,191,36,0.85)" : "rgba(255,255,255,0.55)", fontWeight: 600 }}>{v}</span>
                                      ))}
                                    </div>
                                    {c.sources?.length > 0 && <p style={{ fontSize: 9, color: "rgba(255,255,255,0.28)", marginTop: 3 }}>{c.sources.join(", ")}</p>}
                                  </div>
                                ))}
                              </div>
                            </div>
                          </div>
                          <button
                            onClick={() => { setRefineNavText(`Verify and resolve these conflicting data points: ${conflictSummary}. Find authoritative sources and return the correct values.`); setRefineNavOpen(true); }}
                            style={{ padding: "7px 14px", borderRadius: 8, background: "rgba(251,191,36,0.12)", border: "1px solid rgba(251,191,36,0.30)", color: "rgba(251,191,36,0.85)", fontSize: 11, fontWeight: 700, cursor: "pointer", transition: "background 0.15s" }}
                            onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.background = "rgba(251,191,36,0.20)"; }}
                            onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.background = "rgba(251,191,36,0.12)"; }}
                          >
                            🔍 Launch Conflict Resolution Mission →
                          </button>
                        </motion.div>
                      );
                    })()}

                    {/* Refinement history */}
                    {refinementHistory.length > 0 && (
                      <div style={{ marginTop: 20, borderRadius: 12, background: "rgba(167,202,220,0.05)", border: "0.5px solid rgba(167,202,220,0.18)", overflow: "hidden" }}>
                        <button
                          onClick={() => setShowRefinementHistory(v => !v)}
                          style={{ width: "100%", padding: "9px 14px", display: "flex", alignItems: "center", gap: 8, background: "transparent", cursor: "pointer", textAlign: "left" }}
                        >
                          <span style={{ fontSize: 9, fontWeight: 700, letterSpacing: "0.10em", textTransform: "uppercase", color: "var(--opalzero-accent,rgba(167,202,220,0.70))" }}>
                            Refinement history ({refinementHistory.length})
                          </span>
                          <span style={{ flex: 1 }} />
                          <span style={{ fontSize: 10, color: "rgba(255,255,255,0.28)" }}>{showRefinementHistory ? "▲" : "▼"}</span>
                        </button>
                        <AnimatePresence>
                          {showRefinementHistory && (
                            <motion.div
                              key="refine-hist"
                              initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }}
                              transition={{ type: "spring", stiffness: 360, damping: 32 }}
                              style={{ overflow: "hidden", borderTop: "0.5px solid rgba(167,202,220,0.12)" }}
                            >
                              <div style={{ padding: "8px 10px 10px", display: "flex", flexDirection: "column", gap: 6 }}>
                                {refinementHistory.map((round, ri) => (
                                  <div key={ri} style={{ display: "flex", gap: 10, alignItems: "flex-start", padding: "8px 10px", borderRadius: 8, background: "var(--opalzero-glass-bg,rgba(255,255,255,0.04))", borderLeft: "2px solid var(--opalzero-accent,#a7cadc)" }}>
                                    <span style={{ flexShrink: 0, fontSize: 9, fontWeight: 800, padding: "2px 6px", borderRadius: 5, background: "rgba(167,202,220,0.18)", border: "0.5px solid rgba(167,202,220,0.35)", color: "var(--opalzero-accent,#a7cadc)", lineHeight: 1.5, marginTop: 1 }}>R{ri + 1}</span>
                                    <div style={{ flex: 1, minWidth: 0 }}>
                                      <p style={{ fontSize: 12, color: "rgba(255,255,255,0.78)", fontWeight: 500, lineHeight: 1.4, marginBottom: 4, display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical", overflow: "hidden" } as React.CSSProperties}>{round.intent}</p>
                                      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                                        <span style={{ fontSize: 9, color: "rgba(255,255,255,0.28)", fontFamily: "var(--font-mono,monospace)" }}>{timeAgo(round.timestamp)}</span>
                                        {round.newPayloadKeys.length === 0 ? (
                                          <span style={{ fontSize: 9, padding: "1px 6px", borderRadius: 4, background: "rgba(255,255,255,0.05)", border: "0.5px solid rgba(255,255,255,0.10)", color: "rgba(255,255,255,0.28)", fontWeight: 600 }}>no new data</span>
                                        ) : (
                                          <span style={{ fontSize: 9, padding: "1px 6px", borderRadius: 4, background: "rgba(167,202,220,0.12)", border: "0.5px solid rgba(167,202,220,0.25)", color: "var(--opalzero-accent,#a7cadc)", fontWeight: 600 }}>+{round.newPayloadKeys.length} new key{round.newPayloadKeys.length !== 1 ? "s" : ""}</span>
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

                    {/* Show/hide agent reasoning toggle */}
                    <div style={{ marginTop: 20, display: "flex", justifyContent: "center" }}>
                      <button
                        onClick={() => setShowDetails(v => !v)}
                        style={{
                          fontSize: 11, padding: "5px 14px", borderRadius: 999,
                          background: "rgba(255,255,255,0.04)",
                          border: "0.5px solid rgba(255,255,255,0.09)",
                          color: "rgba(255,255,255,0.30)",
                          cursor: "pointer",
                          transition: "background 0.15s, color 0.15s",
                        }}
                        onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.55)"; }}
                        onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.30)"; }}
                      >
                        {showDetails ? "Hide agent reasoning" : "Show agent reasoning"}
                      </button>
                    </div>
                  </div>
                ) : null;
              })()}

              {/* Fallback / agent cards (when no bento state, or showDetails) */}
              {(!missionState || showDetails) && (() => {
                const visibleSlugs = cardOrder.filter(slug => {
                  const result = streamCards[slug]?.result;
                  if (!result) return true;
                  try {
                    const parsed = JSON.parse(result);
                    return !(parsed && typeof parsed.data_payload === "object" && parsed.data_payload !== null);
                  } catch { return true; }
                });
                if (visibleSlugs.length === 0) return null;
                return (
                  <div style={{ padding: "0 24px", maxWidth: 960, margin: "0 auto", marginTop: 24 }}>
                    {missionState && (
                      <p style={{ fontSize: 10, fontWeight: 700, textTransform: "uppercase", letterSpacing: "0.14em", color: "rgba(147,153,160,0.45)", fontFamily: "var(--opalzero-font-mono)", marginBottom: 12 }}>
                        Agent Reasoning
                      </p>
                    )}
                    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
                      {visibleSlugs.map(slug => {
                        const card = streamCards[slug];
                        if (!card) return null;
                        const { label, icon, accent } = cardMeta(slug, card.role);
                        return (
                          <motion.article
                            key={slug}
                            layout
                            initial={{ opacity: 0, y: 10 }}
                            animate={{ opacity: 1, y: 0 }}
                            transition={{ duration: 0.25 }}
                            className="rounded-2xl opalzero-grain"
                            style={{
                              background: "linear-gradient(rgba(255,255,255,0.04), rgba(255,255,255,0.016))",
                              border: `1px solid rgba(255,255,255,0.08)`,
                              borderLeft: `3px solid ${accent.replace("border-","").replace("-700","") || "rgba(255,255,255,0.18)"}`,
                              backdropFilter: "blur(28px) saturate(130%)",
                              WebkitBackdropFilter: "blur(28px) saturate(130%)",
                              padding: "var(--opalzero-pad,20px)",
                            }}
                          >
                            <h2 style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, fontWeight: 600, color: "rgba(255,255,255,0.80)", marginBottom: 10 }}>
                              <span>{icon}</span><span>{label}</span>
                              {card.status === "failed" && <span style={{ marginLeft: "auto", fontSize: 11, color: "#f87171" }}>✗ Failed</span>}
                            </h2>
                            {card.result ? (
                              <ReactMarkdown remarkPlugins={[remarkGfm]} components={mdComponents}>
                                {card.result}
                              </ReactMarkdown>
                            ) : null}
                          </motion.article>
                        );
                      })}
                    </div>
                  </div>
                );
              })()}
            </motion.div>
          )}
        </AnimatePresence>

        {/* ── FAILED STATE ────────────────────────────────────────────────── */}
        <AnimatePresence>
          {missionStatus === "failed" && (
            <motion.div
              key="failed-root"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.25 }}
              style={{
                minHeight: "100vh",
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                gap: 16,
                padding: "86px 24px 48px",
                textAlign: "center",
              }}
            >
              <div style={{ fontSize: 36, opacity: 0.35 }}>✗</div>
              <p style={{ fontSize: 16, fontWeight: 500, color: "rgba(255,255,255,0.70)" }}>Mission failed</p>
              {fetchError && (
                <p style={{ fontSize: 13, color: "#fca5a5", maxWidth: 480, lineHeight: 1.5 }}>{fetchError}</p>
              )}
              <button
                onClick={newMission}
                style={{
                  marginTop: 8, padding: "9px 22px", borderRadius: 999,
                  background: "var(--opalzero-accent,#a7cadc)",
                  color: "var(--opalzero-accent-fg,#07090c)",
                  fontWeight: 600, fontSize: 13, cursor: "pointer",
                  boxShadow: "0 0 20px rgba(167,202,220,0.25)",
                }}
              >
                New Mission
              </button>
            </motion.div>
          )}
        </AnimatePresence>

      </div>{/* end stage */}

      {/* ── Floating trace button (bottom-right) ─────────────────────────── */}
      <AnimatePresence>
        {traceEvents.length > 0 && !isIdle && (
          <motion.button
            key="trace-btn"
            initial={{ opacity: 0, scale: 0.80 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.80 }}
            transition={{ type: "spring", stiffness: 300, damping: 22 }}
            onClick={() => setTraceOpen(v => !v)}
            title="Execution trace"
            style={{
              position: "fixed",
              bottom: 24,
              right: 24,
              zIndex: 60,
              width: 44,
              height: 44,
              borderRadius: 12,
              background: traceOpen ? "rgba(255,255,255,0.10)" : "rgba(255,255,255,0.06)",
              border: `0.5px solid ${traceOpen ? "rgba(255,255,255,0.20)" : "rgba(255,255,255,0.10)"}`,
              backdropFilter: "blur(20px)",
              WebkitBackdropFilter: "blur(20px)",
              color: traceOpen ? "rgba(255,255,255,0.80)" : "rgba(255,255,255,0.35)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              cursor: "pointer",
              fontSize: 13,
              fontFamily: "var(--opalzero-font-mono)",
              letterSpacing: "-0.02em",
              transition: "background 0.15s, border-color 0.15s, color 0.15s",
            }}
          >
            ⟨⟩
          </motion.button>
        )}
      </AnimatePresence>

      {/* ── Trace drawer (slides up from bottom-right) ────────────────────── */}
      <AnimatePresence>
        {traceOpen && (
          <motion.div
            key="trace-drawer"
            initial={{ opacity: 0, y: 18, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 14, scale: 0.95 }}
            transition={{ type: "spring", stiffness: 320, damping: 28 }}
            style={{
              position: "fixed",
              bottom: 78,
              right: 24,
              width: 400,
              maxHeight: "56vh",
              zIndex: 55,
              background: "rgba(6,9,14,0.97)",
              backdropFilter: "blur(40px) saturate(150%)",
              WebkitBackdropFilter: "blur(40px) saturate(150%)",
              border: "1px solid rgba(255,255,255,0.10)",
              borderRadius: 18,
              boxShadow: "0 24px 60px rgba(0,0,0,0.65), 0 0 0 1px rgba(255,255,255,0.04)",
              display: "flex",
              flexDirection: "column",
              overflow: "hidden",
            }}
          >
            {/* Header */}
            <div style={{ padding: "11px 16px 9px", borderBottom: "1px solid rgba(255,255,255,0.07)", display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
              <span style={{ fontSize: 10, fontWeight: 700, letterSpacing: "0.10em", textTransform: "uppercase", color: "rgba(255,255,255,0.30)" }}>
                Execution Trace
              </span>
              <span style={{ fontSize: 9, color: "rgba(255,255,255,0.18)" }}>— {traceEvents.length} events</span>
              <span style={{ flex: 1 }} />
              <button onClick={() => setTraceOpen(false)} style={{ color: "rgba(255,255,255,0.28)", background: "none", border: "none", fontSize: 18, cursor: "pointer", lineHeight: 1, padding: "0 2px", transition: "color 0.15s" }}
                onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.70)"; }}
                onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.28)"; }}>×</button>
            </div>
            {/* Events list */}
            <div style={{ flex: 1, overflowY: "auto", padding: "4px 0 10px" }}>
              {traceEvents.map(event => (
                <div key={event.id} style={{ display: "flex", alignItems: "flex-start", gap: 12, padding: "4px 16px", borderBottom: "1px solid rgba(255,255,255,0.03)" }}>
                  <span style={{ fontSize: 9, color: "rgba(255,255,255,0.18)", minWidth: 52, fontVariantNumeric: "tabular-nums", paddingTop: 2, fontFamily: "monospace" }}>
                    {event.timestamp < 10 ? `#${event.timestamp}` : formatTraceTime(event.timestamp)}
                  </span>
                  <div style={{ width: 6, height: 6, borderRadius: "50%", marginTop: 4, flexShrink: 0, background: dotColorForType(event.type) }} />
                  <span style={{ fontSize: 11, color: "rgba(255,255,255,0.50)", lineHeight: 1.4, flex: 1 }}>
                    {event.label}
                    {event.durationMs !== undefined && (
                      <span style={{ marginLeft: 8, fontSize: 9, color: "rgba(255,255,255,0.22)" }}>{event.durationMs}ms</span>
                    )}
                  </span>
                </div>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>

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
            style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.30)", zIndex: 189 }}
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
              position: "fixed", top: 0, right: 0, height: "100%", width: 360, zIndex: 190,
              background: "rgba(8,10,14,0.96)", backdropFilter: "blur(40px) saturate(150%)",
              WebkitBackdropFilter: "blur(40px) saturate(150%)",
              borderLeft: "1px solid rgba(255,255,255,0.07)",
              display: "flex", flexDirection: "column",
              padding: "28px 28px 32px", overflowY: "auto",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: 28 }}>
              <p style={{ fontSize: 15, fontWeight: 700, color: "rgba(255,255,255,0.9)", margin: 0 }}>Settings</p>
              <button onClick={() => setSettingsOpen(false)} style={{ background: "none", border: "none", color: "rgba(255,255,255,0.35)", fontSize: 20, lineHeight: 1, cursor: "pointer", padding: "0 2px", transition: "color 0.15s" }}
                onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.80)"; }}
                onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.35)"; }}>×</button>
            </div>

            {[
              { label: "OpalZero API Key", value: draftOpalzeroKey, setter: setDraftOpalzeroKey, placeholder: "opalzero_sk_...", hint: "Required when the server has OPALZERO_API_KEY set. Leave blank for local dev.", success: null },
              { label: "OpenAI API Key", value: draftOpenAI, setter: setDraftOpenAI, placeholder: "sk-...", hint: configStatus?.openai && !draftOpenAI ? "✓ Configured via environment" : "Required for all missions.", success: configStatus?.openai && !draftOpenAI ? true : null },
              { label: "Tavily Search Key", value: draftTavily, setter: setDraftTavily, placeholder: "tvly-...", hint: configStatus?.tavily && !draftTavily ? "✓ Configured via environment" : "Optional — enables live web search.", success: configStatus?.tavily && !draftTavily ? true : null },
              { label: "Alpha Vantage Key", value: draftAlphaVantage, setter: setDraftAlphaVantage, placeholder: "e.g. A1B2C3D4E5F6G7H8", hint: configStatus?.alpha_vantage && !draftAlphaVantage ? "✓ Configured via environment" : "Optional — enables real-time stock data (get_company_overview, get_price_history, etc.).", success: configStatus?.alpha_vantage && !draftAlphaVantage ? true : null },
            ].map(field => (
              <div key={field.label} style={{ marginBottom: 24 }}>
                <label style={{ display: "block", fontSize: 11, fontWeight: 600, letterSpacing: "0.08em", textTransform: "uppercase", color: "rgba(255,255,255,0.45)", marginBottom: 8 }}>{field.label}</label>
                <input
                  type="password"
                  value={field.value}
                  onChange={e => field.setter(e.target.value)}
                  placeholder={field.placeholder}
                  style={{ width: "100%", background: "rgba(255,255,255,0.07)", border: "1px solid rgba(255,255,255,0.12)", borderRadius: 10, padding: "10px 14px", color: "rgba(255,255,255,0.9)", fontSize: 13, outline: "none", boxSizing: "border-box", transition: "border-color 0.15s" }}
                  onFocus={e => { e.currentTarget.style.borderColor = "var(--opalzero-accent,#a7cadc)"; }}
                  onBlur={e => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.12)"; }}
                />
                <p style={{ fontSize: 11, color: field.success ? "rgba(74,222,128,0.8)" : "rgba(255,255,255,0.28)", marginTop: 6, margin: "6px 0 0" }}>{field.hint}</p>
              </div>
            ))}

            <button onClick={saveSettings} style={{ width: "100%", background: "var(--opalzero-accent,#a7cadc)", color: "#000", fontWeight: 600, fontSize: 14, borderRadius: 10, padding: "11px 0", marginTop: 8, border: "none", cursor: "pointer", transition: "opacity 0.15s" }}
              onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.opacity = "0.85"; }}
              onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.opacity = "1"; }}>
              Save
            </button>

            {notifPermission !== "unsupported" && (
              <div style={{ marginTop: 20, paddingTop: 16, borderTop: "1px solid rgba(255,255,255,0.07)", display: "flex", alignItems: "center", justifyContent: "space-between", gap: 12 }}>
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontSize: 13, fontWeight: 600, color: "rgba(255,255,255,0.8)" }}>Mission notifications</div>
                  <div style={{ fontSize: 11, color: "rgba(255,255,255,0.3)", marginTop: 2, lineHeight: 1.4 }}>
                    {notifPermission === "granted" && "Enabled — you'll be notified when missions complete"}
                    {notifPermission === "denied"  && "Blocked — enable in your browser settings"}
                    {notifPermission === "default" && "Not yet requested — start a mission to prompt"}
                  </div>
                </div>
                {notifPermission === "default" && (
                  <button onClick={() => Notification.requestPermission().then(p => setNotifPermission(p))}
                    style={{ flexShrink: 0, fontSize: 11, padding: "5px 12px", borderRadius: 8, cursor: "pointer", background: "rgba(255,255,255,0.08)", border: "1px solid rgba(255,255,255,0.12)", color: "rgba(255,255,255,0.65)", whiteSpace: "nowrap" }}>
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
              position: "fixed", bottom: 80, left: "50%", transform: "translateX(-50%)", zIndex: 300,
              background: "rgba(255,255,255,0.10)", border: "0.5px solid rgba(255,255,255,0.18)",
              backdropFilter: "blur(24px)", WebkitBackdropFilter: "blur(24px)",
              borderRadius: 10, padding: "8px 18px",
              fontSize: 13, fontWeight: 600, color: "rgba(255,255,255,0.85)",
              pointerEvents: "none",
            }}
          >
            ✓ Saved
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── History sidebar backdrop ──────────────────────────────────────── */}
      <AnimatePresence>
        {sidebarOpen && (
          <motion.div
            key="sidebar-backdrop"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={() => setSidebarOpen(false)}
            style={{ position: "fixed", inset: 0, background: "rgba(0,0,0,0.45)", backdropFilter: "blur(4px)", WebkitBackdropFilter: "blur(4px)", zIndex: 30 }}
          />
        )}
      </AnimatePresence>

      {/* ── History sidebar drawer ────────────────────────────────────────── */}
      <AnimatePresence>
        {sidebarOpen && (
          <motion.aside
            key="sidebar-drawer"
            initial={{ x: -296 }}
            animate={{ x: 0 }}
            exit={{ x: -296 }}
            transition={{ type: "spring", stiffness: 300, damping: 30 }}
            style={{
              position: "fixed", top: 0, left: 0, bottom: 0, width: 288, zIndex: 40,
              display: "flex", flexDirection: "column",
              background: "rgba(6,8,11,0.88)",
              backdropFilter: "blur(40px) saturate(150%)",
              WebkitBackdropFilter: "blur(40px) saturate(150%)",
              borderRight: "1px solid rgba(255,255,255,0.07)",
              boxShadow: "4px 0 60px rgba(0,0,0,0.60), inset -1px 0 0 rgba(255,255,255,0.04)",
            }}
          >
            {/* Drawer header */}
            <div style={{ padding: "20px 16px 14px", borderBottom: "0.5px solid rgba(255,255,255,0.07)", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
              <span style={{ fontSize: 10, fontWeight: 600, letterSpacing: "0.12em", textTransform: "uppercase", color: "rgba(255,255,255,0.30)" }}>Mission Gallery</span>
              <button onClick={() => setSidebarOpen(false)} style={{ color: "rgba(255,255,255,0.35)", fontSize: 20, lineHeight: 1, width: 28, height: 28, display: "flex", alignItems: "center", justifyContent: "center", borderRadius: 8, background: "none", border: "none", cursor: "pointer", transition: "color 0.2s, background 0.2s" }}
                onMouseEnter={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.75)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.06)"; }}
                onMouseLeave={e => { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.35)"; (e.currentTarget as HTMLButtonElement).style.background = "transparent"; }}>×</button>
            </div>

            {/* New Mission button */}
            <div style={{ padding: "12px 12px 6px", flexShrink: 0 }}>
              <button onClick={newMission} style={{ width: "100%", padding: "12px 16px", borderRadius: 14, background: "linear-gradient(rgba(255,255,255,0.07), rgba(255,255,255,0.03))", color: "rgba(235,239,242,0.92)", fontWeight: 600, fontSize: 14, fontFamily: "var(--opalzero-font-main)", textAlign: "left", display: "flex", alignItems: "center", gap: 10, border: "1px solid rgba(255,255,255,0.11)", backdropFilter: "blur(28px) saturate(130%)", WebkitBackdropFilter: "blur(28px) saturate(130%)", boxShadow: "inset 0 1px 0 rgba(255,255,255,0.08), 0 0 0 1px rgba(255,255,255,0.04)", transition: "background 0.2s, border-color 0.2s", cursor: "pointer" }}
                onMouseEnter={e => { const el = e.currentTarget as HTMLButtonElement; el.style.background = "linear-gradient(rgba(255,255,255,0.10), rgba(255,255,255,0.05))"; el.style.borderColor = "rgba(255,255,255,0.16)"; }}
                onMouseLeave={e => { const el = e.currentTarget as HTMLButtonElement; el.style.background = "linear-gradient(rgba(255,255,255,0.07), rgba(255,255,255,0.03))"; el.style.borderColor = "rgba(255,255,255,0.11)"; }}>
                <span style={{ fontSize: 18, lineHeight: 1, opacity: 0.80 }}>＋</span>
                New Mission
              </button>
            </div>

            {/* History list */}
            <div style={{ flex: 1, overflowY: "auto", padding: "6px 8px 20px", display: "flex", flexDirection: "column" }}>
              {history.length > 0 && (
                <div style={{ position: "relative", marginBottom: 8, flexShrink: 0 }}>
                  <span style={{ position: "absolute", left: 10, top: "50%", transform: "translateY(-50%)", fontSize: 12, color: "rgba(255,255,255,0.28)", pointerEvents: "none", lineHeight: 1 }}>⌕</span>
                  <input type="text" value={historyQuery} onChange={e => setHistoryQuery(e.target.value)}
                    onFocus={() => setHistoryQueryFocused(true)} onBlur={() => setHistoryQueryFocused(false)}
                    placeholder="Search missions…"
                    style={{ width: "100%", padding: "7px 28px", borderRadius: 10, background: historyQueryFocused ? "rgba(255,255,255,0.08)" : "rgba(255,255,255,0.05)", border: `0.5px solid ${historyQueryFocused ? "rgba(var(--opalzero-accent-rgb,167,202,220),0.45)" : "rgba(255,255,255,0.10)"}`, color: "rgba(255,255,255,0.82)", fontSize: 12, outline: "none", transition: "background 0.15s, border-color 0.15s", boxSizing: "border-box" }}
                  />
                  {historyQuery && (
                    <button onClick={() => setHistoryQuery("")} style={{ position: "absolute", right: 8, top: "50%", transform: "translateY(-50%)", fontSize: 13, lineHeight: 1, color: "rgba(255,255,255,0.35)", background: "none", border: "none", cursor: "pointer", padding: 2 }}>×</button>
                  )}
                </div>
              )}

              {history.length === 0 ? (
                <div style={{ padding: "36px 16px", textAlign: "center" }}>
                  <div style={{ fontSize: 24, opacity: 0.18, marginBottom: 8 }}>◈</div>
                  <p style={{ fontSize: 11, color: "rgba(255,255,255,0.22)", lineHeight: 1.6 }}>No missions yet.<br />Run your first to begin.</p>
                </div>
              ) : filteredHistory.length === 0 ? (
                <div style={{ padding: "28px 16px", textAlign: "center" }}>
                  <p style={{ fontSize: 11, color: "rgba(255,255,255,0.22)", lineHeight: 1.6 }}>No matches for<br /><span style={{ color: "rgba(255,255,255,0.40)", fontStyle: "italic" }}>&ldquo;{historyQuery}&rdquo;</span></p>
                </div>
              ) : (
                <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
                  <AnimatePresence initial={false}>
                    {filteredHistory.map(m => {
                      const accent = accentForMission(m);
                      const isActive = activeMissionId === m.id;
                      const heights = sparklineHeights(m.id, m.task_count);
                      const isConfirming = confirmDeleteId === m.id;
                      return (
                        <motion.div key={m.id} layout initial={{ opacity: 0, x: -16 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -48, height: 0, marginBottom: 0, overflow: "hidden" }} transition={{ duration: 0.22 }} style={{ position: "relative" }}
                          onMouseLeave={() => { if (isConfirming) setConfirmDeleteId(null); }}>
                          <motion.button onClick={() => { loadMission(m.id); setSidebarOpen(false); }} whileHover={{ boxShadow: `0 0 18px ${accent}2a, 0 2px 8px rgba(0,0,0,0.30)` }}
                            style={{ width: "100%", textAlign: "left", borderRadius: 12, padding: "12px 14px", paddingRight: 40, cursor: "pointer", background: isActive ? `linear-gradient(135deg, ${accent}20 0%, rgba(255,255,255,0.04) 100%)` : `linear-gradient(135deg, ${accent}0d 0%, transparent 70%)`, border: `0.5px solid ${isActive ? accent + "44" : "rgba(255,255,255,0.07)"}` }}>
                            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                              <span style={{ width: 16, height: 16, borderRadius: "50%", flexShrink: 0, background: m.status === "completed" ? `${accent}28` : "rgba(248,113,113,0.15)", color: m.status === "completed" ? accent : "#f87171", fontSize: 9, fontWeight: 700, display: "flex", alignItems: "center", justifyContent: "center" }}>{m.status === "completed" ? "✓" : "✕"}</span>
                              <span style={{ fontSize: 10, fontFamily: "monospace", color: "rgba(255,255,255,0.25)", marginLeft: "auto" }}>{formatDate(m.timestamp)}</span>
                            </div>
                            <p style={{ fontSize: "1.05rem", fontWeight: isActive ? 600 : 500, lineHeight: 1.4, color: isActive ? "rgba(255,255,255,0.92)" : "rgba(255,255,255,0.65)", display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical", overflow: "hidden" }}>{m.intent}</p>
                            <div style={{ marginTop: 8, display: "flex", alignItems: "flex-end", gap: 2, height: 14 }}>
                              {heights.map((h, i) => (<span key={i} style={{ display: "inline-block", width: 3, height: h, borderRadius: 2, background: accent, opacity: 0.38 + i * 0.045 }} />))}
                              <span style={{ marginLeft: 4, fontSize: 9, fontFamily: "monospace", color: `${accent}90`, lineHeight: "13px" }}>{m.task_count}t</span>
                            </div>
                          </motion.button>
                          <motion.button onClick={(e: React.MouseEvent) => { e.stopPropagation(); if (isConfirming) deleteMission(m.id); else setConfirmDeleteId(m.id); }} title={isConfirming ? "Click again to confirm" : "Delete mission"}
                            animate={{ opacity: isConfirming ? 1 : 0 }} whileHover={{ opacity: 1 }} transition={{ duration: 0.15 }}
                            style={{ position: "absolute", top: "50%", right: 10, transform: "translateY(-50%)", width: 26, height: 26, borderRadius: 7, display: "flex", alignItems: "center", justifyContent: "center", background: isConfirming ? "rgba(239,68,68,0.18)" : "rgba(255,255,255,0.07)", border: `1px solid ${isConfirming ? "rgba(239,68,68,0.45)" : "rgba(255,255,255,0.10)"}`, color: isConfirming ? "#f87171" : "rgba(255,255,255,0.40)", fontSize: 12, cursor: "pointer", transition: "background 0.15s, border-color 0.15s, color 0.15s" }}>
                            {isConfirming ? "✕" : "🗑"}
                          </motion.button>
                        </motion.div>
                      );
                    })}
                  </AnimatePresence>
                </div>
              )}

              {history.length > 0 && (
                <div style={{ marginTop: "auto", paddingTop: 14, flexShrink: 0 }}>
                  <button
                    onClick={() => { if (confirmClear) clearAllHistory(); else { setConfirmClear(true); setTimeout(() => setConfirmClear(false), 3000); } }}
                    style={{ width: "100%", padding: "8px 12px", borderRadius: 10, fontSize: 11, fontWeight: 600, letterSpacing: "0.04em", cursor: "pointer", background: confirmClear ? "rgba(239,68,68,0.14)" : "rgba(255,255,255,0.04)", border: `0.5px solid ${confirmClear ? "rgba(239,68,68,0.40)" : "rgba(255,255,255,0.10)"}`, color: confirmClear ? "#f87171" : "rgba(255,255,255,0.28)", transition: "background 0.15s, border-color 0.15s, color 0.15s" }}
                    onMouseEnter={e => { if (!confirmClear) { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.50)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.07)"; } }}
                    onMouseLeave={e => { if (!confirmClear) { (e.currentTarget as HTMLButtonElement).style.color = "rgba(255,255,255,0.28)"; (e.currentTarget as HTMLButtonElement).style.background = "rgba(255,255,255,0.04)"; } }}>
                    {confirmClear ? "⚠ Confirm — delete all missions" : "Clear all history"}
                  </button>
                </div>
              )}
            </div>
          </motion.aside>
        )}
      </AnimatePresence>

      {/* ── Clarification modal ───────────────────────────────────────────── */}
      {showClarifyModal && (
        <ClarifyModal
          intent={intent}
          questions={clarifyQuestions}
          answers={clarifyAnswers}
          onAnswerChange={(key, val) =>
            setClarifyAnswers((prev) => ({ ...prev, [key]: val }))
          }
          onSubmit={runMissionWithAnswers}
          onSkip={() => {
            clarifyPassedRef.current = true;
            pendingRunRef.current = true;
            setClarifyQuestions([]);
            setShowClarifyModal(false);
          }}
        />
      )}

    </div>
  );
}
