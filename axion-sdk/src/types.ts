import type { AxionClient } from './client';

// ── Mission lifecycle ────────────────────────────────────────────────────────

export type MissionStatus = "idle" | "running" | "complete" | "failed";

export interface Task {
  slug:   string;
  role:   string;
  intent: string;
  status: "pending" | "running" | "completed" | "failed";
  result?: string;
}

export interface MissionState {
  data_payload:       Record<string, unknown>;
  design_tokens?:     Record<string, string>;
  layout_hint?:       string;
  suggested_widgets?: string[];
}

export interface MissionSnapshot {
  mission_id:    string;
  intent:        string;
  plan:          Task[];
  mission_state: MissionState;
  status:        MissionStatus;
  created_at?:   string;
}

export interface MissionSummary {
  id:          string;
  intent:      string;
  status:      MissionStatus;
  created_at?: string;
}

// ── SSE event union ──────────────────────────────────────────────────────────
// Mirrors axion-core's MissionUpdate enum (#[serde(tag = "type", rename_all = "snake_case")])

export interface TaskStartedEvent {
  type:   "task_started";
  slug:   string;
  role:   string;
  intent: string;
}

export interface TaskCompletedEvent {
  type:   "task_completed";
  slug:   string;
  role:   string;
  result: string;
}

/** Note: the Rust TaskFailed variant carries only slug + role, no error message. */
export interface TaskFailedEvent {
  type: "task_failed";
  slug: string;
  role: string;
}

/** Emitted when the Governor expands the mission plan mid-run. */
export interface GovernorExpandEvent {
  type:           "governor_expand";
  new_task_count: number;
  descriptions:   string[];
}

export interface MissionCompleteEvent {
  type:                "mission_complete";
  mission_id:          string;
  intent:              string;
  task_count:          number;
  expanded_task_count: number;
  layout_hint:         string;
  mission_state?:      MissionState;
}

export interface MissionFailedEvent {
  type:  "mission_failed";
  error: string;
}

/** Emitted when the `feedback` tool pauses a mission for human input. */
export interface MissionPausedEvent {
  type:       "mission_paused";
  question:   string;
  mission_id: string;
}

export interface AwaitingFeedbackEvent {
  type:     "awaiting_feedback";
  slug:     string;
  question: string;
}

export interface UnknownEvent {
  type: string;
  [key: string]: unknown;
}

export type MissionEvent =
  | TaskStartedEvent
  | TaskCompletedEvent
  | TaskFailedEvent
  | GovernorExpandEvent
  | MissionCompleteEvent
  | MissionFailedEvent
  | MissionPausedEvent
  | AwaitingFeedbackEvent
  | UnknownEvent;

// ── Upload ───────────────────────────────────────────────────────────────────

export interface UploadResult {
  filename:      string;
  file_type:     "image" | "data";
  original_name: string;
}

// ── Config ───────────────────────────────────────────────────────────────────

export interface ConfigStatus {
  openai:         boolean;
  tavily:         boolean;
  alpha_vantage?: boolean;
}

// ── Client config ────────────────────────────────────────────────────────────

export interface AxionClientConfig {
  /** e.g. "http://localhost:8080" */
  baseUrl:          string;
  /** X-Axion-Key — omit for local dev (when AXION_API_KEY is not set on the server) */
  apiKey?:          string;
  /** X-OpenAI-Key — optional per-request override */
  openAiKey?:       string;
  /** X-Tavily-Key — optional per-request override */
  tavilyKey?:       string;
  /** X-Alpha-Vantage-Key — optional per-request override for financial data tools */
  alphaVantageKey?: string;
}

// ── Bento card (parsed from MissionState) ────────────────────────────────────

export interface BentoCard {
  /** The key from data_payload, e.g. "cheapest_flight_usd" */
  key: string;
  /** Component type, e.g. "MetricCard", "ChartCard", "ComparisonTable", "Timeline", "ImageCard" */
  widget: string;
  /** The raw props object to pass to the rendering component */
  props: Record<string, unknown>;
  /** True if this card was added/updated by a refinement round */
  isRefined?: boolean;
}

// ── useMission ────────────────────────────────────────────────────────────────

export interface UseMissionOptions {
  client: AxionClient;
  /** Default model to use. Can be overridden per-call in run(). */
  model?: string;
  /** Called for every SSE event before the hook updates its own state. Use this for side effects (trace logs, banners, etc.) that the hook doesn't need to manage. */
  onEvent?: (event: MissionEvent) => void;
}

export interface UseMissionReturn {
  /** Execute a new mission. Clears previous state before starting. */
  run: (intent: string, model?: string) => Promise<void>;
  /** Refine an existing mission without clearing the card grid. */
  refine: (missionId: string, intent: string, model?: string) => Promise<void>;
  /** Current lifecycle state. */
  status: MissionStatus;
  /** Derived bento cards from the latest MissionState. */
  cards: BentoCard[];
  /** The agent currently executing (null when idle or complete). */
  activeAgent: { role: string; intent: string } | null;
  /** Non-null when status === "failed". */
  error: string | null;
  /** Set after mission_complete. Needed to call refine(). */
  missionId: string | null;
  /** Raw MissionState — use this if you want to build your own renderer. */
  missionState: MissionState | null;
  /** Reset all state back to idle. Does not abort an in-flight stream. */
  reset: () => void;
}
