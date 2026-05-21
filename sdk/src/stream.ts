import type { AxionConfig, AxionSchema } from "./types.js";

export interface StreamHandlers {
  onTaskStarted:   (slug: string, role: string, intent: string) => void;
  onTaskCompleted: (slug: string, role: string) => void;
  onTaskFailed:    (slug: string, role: string) => void;
  onComplete:      (payload: Record<string, unknown>) => void;
  onError:         (message: string) => void;
}

export function streamMission(
  serverUrl: string,
  intent: string,
  schema: AxionSchema | undefined,
  handlers: StreamHandlers,
  config?: AxionConfig,
): () => void {
  const controller = new AbortController();

  void (async () => {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (config?.alphaVantageKey) {
      headers["x-alpha-vantage-key"] = config.alphaVantageKey;
    }

    const body: Record<string, unknown> = { intent };
    if (schema) body["schema"] = schema;

    let response: Response;
    try {
      response = await fetch(`${serverUrl}/api/v1/execute`, {
        method: "POST",
        headers,
        body: JSON.stringify(body),
        signal: controller.signal,
      });
    } catch (err) {
      if ((err as Error).name === "AbortError") return;
      handlers.onError((err as Error).message ?? "Network error connecting to Axion");
      return;
    }

    if (!response.ok) {
      handlers.onError(`Axion server returned ${response.status} ${response.statusText}`);
      return;
    }

    const reader = response.body?.getReader();
    if (!reader) {
      handlers.onError("Response body is empty — expected an SSE stream");
      return;
    }

    const decoder = new TextDecoder();
    let lineBuffer = "";
    let currentEvent = "";
    let currentData = "";

    const flush = () => {
      if (!currentEvent || !currentData) { currentEvent = ""; currentData = ""; return; }
      try { dispatch(currentEvent, JSON.parse(currentData) as Record<string, unknown>); }
      catch { /* malformed JSON — skip */ }
      currentEvent = "";
      currentData = "";
    };

    const dispatch = (event: string, data: Record<string, unknown>) => {
      switch (event) {
        case "task_started":
          handlers.onTaskStarted(String(data["slug"] ?? ""), String(data["role"] ?? ""), String(data["intent"] ?? ""));
          break;
        case "task_completed":
          handlers.onTaskCompleted(String(data["slug"] ?? ""), String(data["role"] ?? ""));
          break;
        case "task_failed":
          handlers.onTaskFailed(String(data["slug"] ?? ""), String(data["role"] ?? ""));
          break;
        case "mission_complete": {
          const ms = data["mission_state"] as Record<string, unknown> | undefined;
          const payload = (ms?.["data_payload"] ?? {}) as Record<string, unknown>;
          handlers.onComplete(payload);
          break;
        }
        case "mission_failed":
          handlers.onError(String(data["error"] ?? "Mission failed"));
          break;
      }
    };

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        lineBuffer += decoder.decode(value, { stream: true });
        const lines = lineBuffer.split("\n");
        lineBuffer = lines.pop() ?? "";
        for (const line of lines) {
          if (line.startsWith("event:")) { currentEvent = line.slice(6).trim(); }
          else if (line.startsWith("data:")) { currentData = line.slice(5).trim(); }
          else if (line === "" || line === "\r") { flush(); }
        }
      }
      flush();
    } catch (err) {
      if ((err as Error).name === "AbortError") return;
      handlers.onError((err as Error).message ?? "SSE stream interrupted");
    }
  })();

  return () => controller.abort();
}
