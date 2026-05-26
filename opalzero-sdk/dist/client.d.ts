import type { AxionClientConfig, MissionEvent, MissionSnapshot, MissionSummary, UploadResult, ConfigStatus } from "./types";
export declare class AxionClient {
    private readonly base;
    /** @internal */ readonly _headers: Record<string, string>;
    constructor(config: AxionClientConfig);
    execute(intent: string, model?: string): AsyncGenerator<MissionEvent>;
    readonly missions: {
        list: () => Promise<MissionSummary[]>;
        get: (id: string) => Promise<MissionSnapshot>;
        delete: (id: string) => Promise<void>;
        /** Streams refinement events for an existing mission. */
        refine: (id: string, intent: string, model?: string) => AsyncGenerator<MissionEvent>;
        export: (id: string, format: "md" | "csv" | "html") => Promise<Blob>;
    };
    upload(file: File): Promise<UploadResult>;
    configStatus(): Promise<ConfigStatus>;
    private _refine;
    private _throwApiError;
}
