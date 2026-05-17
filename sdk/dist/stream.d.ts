import type { AxionConfig, AxionSchema } from "./types";
export interface StreamHandlers {
    onTaskStarted: (slug: string, role: string, intent: string) => void;
    onTaskCompleted: (slug: string, role: string) => void;
    onTaskFailed: (slug: string, role: string) => void;
    onComplete: (payload: Record<string, unknown>) => void;
    onError: (message: string) => void;
}
export declare function streamMission(serverUrl: string, intent: string, schema: AxionSchema | undefined, handlers: StreamHandlers, config?: AxionConfig): () => void;
