export type AgentStatus = "running" | "completed" | "failed";
export interface AgentState {
    slug: string;
    role: string;
    status: AgentStatus;
}
export type MissionStatus = "idle" | "running" | "complete" | "failed";
/** Declare the shape of data_payload the app expects from Axion. */
export type AxionSchema = Record<string, "number" | "string" | "array" | "object" | "boolean">;
export interface AxionConfig {
    /** Base URL of the Axion server. Defaults to http://localhost:8080 */
    serverUrl?: string;
    /** Alpha Vantage API key, forwarded as x-alpha-vantage-key */
    alphaVantageKey?: string;
}
export interface UseAxionOptions {
    schema?: AxionSchema;
    config?: AxionConfig;
}
export interface UseAxionResult<T = Record<string, unknown>> {
    agents: AgentState[];
    data: T | null;
    status: MissionStatus;
    error: string | null;
}
