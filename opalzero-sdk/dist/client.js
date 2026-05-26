import { parseSSEStream } from "./stream";
export class AxionClient {
    constructor(config) {
        // ── Missions ──────────────────────────────────────────────────────────────
        this.missions = {
            list: async () => {
                const res = await fetch(`${this.base}/api/v1/missions`, {
                    headers: this._headers,
                });
                if (!res.ok)
                    await this._throwApiError(res);
                return res.json();
            },
            get: async (id) => {
                const res = await fetch(`${this.base}/api/v1/missions/${id}`, {
                    headers: this._headers,
                });
                if (!res.ok)
                    await this._throwApiError(res);
                return res.json();
            },
            delete: async (id) => {
                const res = await fetch(`${this.base}/api/v1/missions/${id}`, {
                    method: "DELETE", headers: this._headers,
                });
                if (!res.ok)
                    await this._throwApiError(res);
            },
            /** Streams refinement events for an existing mission. */
            refine: (id, intent, model) => {
                return this._refine(id, intent, model);
            },
            export: async (id, format) => {
                const res = await fetch(`${this.base}/api/v1/missions/${id}/export?format=${format}`, { headers: this._headers });
                if (!res.ok)
                    await this._throwApiError(res);
                return res.blob();
            },
        };
        this.base = config.baseUrl.replace(/\/$/, "");
        this._headers = { "Content-Type": "application/json" };
        if (config.apiKey)
            this._headers["X-Axion-Key"] = config.apiKey;
        if (config.openAiKey)
            this._headers["X-OpenAI-Key"] = config.openAiKey;
        if (config.tavilyKey)
            this._headers["X-Tavily-Key"] = config.tavilyKey;
        if (config.alphaVantageKey)
            this._headers["X-Alpha-Vantage-Key"] = config.alphaVantageKey;
    }
    // ── Core execution ────────────────────────────────────────────────────────
    async *execute(intent, model) {
        const body = { intent };
        if (model)
            body.model = model;
        const res = await fetch(`${this.base}/api/v1/execute`, {
            method: "POST",
            headers: this._headers,
            body: JSON.stringify(body),
        });
        if (!res.ok)
            await this._throwApiError(res);
        yield* parseSSEStream(res);
    }
    // ── Upload ────────────────────────────────────────────────────────────────
    async upload(file) {
        const form = new FormData();
        form.append("file", file);
        // Omit Content-Type so the browser sets the correct multipart boundary.
        const { "Content-Type": _ct, ...headersWithoutCT } = this._headers;
        const res = await fetch(`${this.base}/api/v1/upload`, {
            method: "POST", headers: headersWithoutCT, body: form,
        });
        if (!res.ok)
            await this._throwApiError(res);
        return res.json();
    }
    // ── Config ────────────────────────────────────────────────────────────────
    async configStatus() {
        const res = await fetch(`${this.base}/api/v1/config/status`, {
            headers: this._headers,
        });
        if (!res.ok)
            await this._throwApiError(res);
        return res.json();
    }
    // ── Internal ──────────────────────────────────────────────────────────────
    async *_refine(id, intent, model) {
        const body = { intent };
        if (model)
            body.model = model;
        const res = await fetch(`${this.base}/api/v1/missions/${id}/refine`, {
            method: "POST",
            headers: this._headers,
            body: JSON.stringify(body),
        });
        if (!res.ok)
            await this._throwApiError(res);
        yield* parseSSEStream(res);
    }
    async _throwApiError(res) {
        let message = `HTTP ${res.status}`;
        let code = "UNKNOWN_ERROR";
        try {
            const body = await res.json();
            if (typeof body.error === "string")
                message = body.error;
            if (typeof body.code === "string")
                code = body.code;
        }
        catch { /* non-JSON body */ }
        const err = new Error(message);
        err.code = code;
        err.status = res.status;
        throw err;
    }
}
