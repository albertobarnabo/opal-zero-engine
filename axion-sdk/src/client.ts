import type {
  AxionClientConfig,
  MissionEvent,
  MissionSnapshot,
  MissionSummary,
  UploadResult,
  ConfigStatus,
} from "./types";
import { parseSSEStream } from "./stream";

export class AxionClient {
  private readonly base:    string;
  /** @internal */ readonly _headers: Record<string, string>;

  constructor(config: AxionClientConfig) {
    this.base = config.baseUrl.replace(/\/$/, "");
    this._headers = { "Content-Type": "application/json" };
    if (config.apiKey)    this._headers["X-Axion-Key"]   = config.apiKey;
    if (config.openAiKey) this._headers["X-OpenAI-Key"]  = config.openAiKey;
    if (config.tavilyKey) this._headers["X-Tavily-Key"]  = config.tavilyKey;
  }

  // ── Core execution ────────────────────────────────────────────────────────

  async *execute(intent: string, model?: string): AsyncGenerator<MissionEvent> {
    const body: Record<string, unknown> = { intent };
    if (model) body.model = model;
    const res = await fetch(`${this.base}/api/v1/execute`, {
      method:  "POST",
      headers: this._headers,
      body:    JSON.stringify(body),
    });
    if (!res.ok) await this._throwApiError(res);
    yield* parseSSEStream(res);
  }

  // ── Missions ──────────────────────────────────────────────────────────────

  readonly missions = {
    list: async (): Promise<MissionSummary[]> => {
      const res = await fetch(`${this.base}/api/v1/missions`, {
        headers: this._headers,
      });
      if (!res.ok) await this._throwApiError(res);
      return res.json() as Promise<MissionSummary[]>;
    },

    get: async (id: string): Promise<MissionSnapshot> => {
      const res = await fetch(`${this.base}/api/v1/missions/${id}`, {
        headers: this._headers,
      });
      if (!res.ok) await this._throwApiError(res);
      return res.json() as Promise<MissionSnapshot>;
    },

    delete: async (id: string): Promise<void> => {
      const res = await fetch(`${this.base}/api/v1/missions/${id}`, {
        method: "DELETE", headers: this._headers,
      });
      if (!res.ok) await this._throwApiError(res);
    },

    /** Streams refinement events for an existing mission. */
    refine: (id: string, intent: string, model?: string): AsyncGenerator<MissionEvent> => {
      return this._refine(id, intent, model);
    },

    export: async (id: string, format: "md" | "csv" | "html"): Promise<Blob> => {
      const res = await fetch(
        `${this.base}/api/v1/missions/${id}/export?format=${format}`,
        { headers: this._headers }
      );
      if (!res.ok) await this._throwApiError(res);
      return res.blob();
    },
  };

  // ── Upload ────────────────────────────────────────────────────────────────

  async upload(file: File): Promise<UploadResult> {
    const form = new FormData();
    form.append("file", file);
    // Omit Content-Type so the browser sets the correct multipart boundary.
    const { "Content-Type": _ct, ...headersWithoutCT } = this._headers;
    const res = await fetch(`${this.base}/api/v1/upload`, {
      method: "POST", headers: headersWithoutCT, body: form,
    });
    if (!res.ok) await this._throwApiError(res);
    return res.json() as Promise<UploadResult>;
  }

  // ── Config ────────────────────────────────────────────────────────────────

  async configStatus(): Promise<ConfigStatus> {
    const res = await fetch(`${this.base}/api/v1/config/status`, {
      headers: this._headers,
    });
    if (!res.ok) await this._throwApiError(res);
    return res.json() as Promise<ConfigStatus>;
  }

  // ── Internal ──────────────────────────────────────────────────────────────

  private async *_refine(id: string, intent: string, model?: string): AsyncGenerator<MissionEvent> {
    const body: Record<string, unknown> = { intent };
    if (model) body.model = model;
    const res = await fetch(`${this.base}/api/v1/missions/${id}/refine`, {
      method:  "POST",
      headers: this._headers,
      body:    JSON.stringify(body),
    });
    if (!res.ok) await this._throwApiError(res);
    yield* parseSSEStream(res);
  }

  private async _throwApiError(res: Response): Promise<never> {
    let message = `HTTP ${res.status}`;
    let code    = "UNKNOWN_ERROR";
    try {
      const body = await res.json() as Record<string, unknown>;
      if (typeof body.error === "string") message = body.error;
      if (typeof body.code  === "string") code    = body.code;
    } catch { /* non-JSON body */ }
    const err = new Error(message) as Error & { code: string; status: number };
    err.code   = code;
    err.status = res.status;
    throw err;
  }
}
