export type MissionStatus = "idle" | "running" | "complete" | "failed";
export interface Task {
    slug: string;
    role: string;
    intent: string;
    status: "pending" | "running" | "completed" | "failed";
    result?: string;
}
export interface MissionState {
    data_payload: Record<string, unknown>;
    design_tokens?: Record<string, string>;
    layout_hint?: string;
    suggested_widgets?: string[];
}
export interface MissionSnapshot {
    mission_id: string;
    intent: string;
    plan: Task[];
    mission_state: MissionState;
    status: MissionStatus;
    created_at?: string;
}
export interface MissionSummary {
    id: string;
    intent: string;
    status: MissionStatus;
    created_at?: string;
}
export interface TaskStartedEvent {
    type: "task_started";
    slug: string;
    role: string;
    intent: string;
}
export interface TaskCompletedEvent {
    type: "task_completed";
    slug: string;
    role: string;
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
    type: "governor_expand";
    new_task_count: number;
    descriptions: string[];
}
export interface MissionCompleteEvent {
    type: "mission_complete";
    mission_id: string;
    intent: string;
    task_count: number;
    expanded_task_count: number;
    layout_hint: string;
    mission_state?: MissionState;
}
export interface MissionFailedEvent {
    type: "mission_failed";
    error: string;
}
/** Emitted when the `feedback` tool pauses a mission for human input. */
export interface MissionPausedEvent {
    type: "mission_paused";
    question: string;
    mission_id: string;
}
export interface AwaitingFeedbackEvent {
    type: "awaiting_feedback";
    slug: string;
    question: string;
}
export interface UnknownEvent {
    type: string;
    [key: string]: unknown;
}
export type MissionEvent = TaskStartedEvent | TaskCompletedEvent | TaskFailedEvent | GovernorExpandEvent | MissionCompleteEvent | MissionFailedEvent | MissionPausedEvent | AwaitingFeedbackEvent | UnknownEvent;
export interface UploadResult {
    filename: string;
    file_type: "image" | "data";
    original_name: string;
}
export interface ConfigStatus {
    openai: boolean;
    tavily: boolean;
}
export interface AxionClientConfig {
    /** e.g. "http://localhost:8080" */
    baseUrl: string;
    /** X-Axion-Key — omit for local dev (when AXION_API_KEY is not set on the server) */
    apiKey?: string;
    /** X-OpenAI-Key — optional per-request override */
    openAiKey?: string;
    /** X-Tavily-Key — optional per-request override */
    tavilyKey?: string;
}
