export function streamMission(serverUrl, intent, schema, handlers, config) {
    const controller = new AbortController();
    void (async () => {
        const headers = { "Content-Type": "application/json" };
        if (config?.alphaVantageKey) {
            headers["x-alpha-vantage-key"] = config.alphaVantageKey;
        }
        const body = { intent };
        if (schema)
            body["schema"] = schema;
        let response;
        try {
            response = await fetch(`${serverUrl}/api/v1/execute`, {
                method: "POST",
                headers,
                body: JSON.stringify(body),
                signal: controller.signal,
            });
        }
        catch (err) {
            if (err.name === "AbortError")
                return;
            handlers.onError(err.message ?? "Network error connecting to Axion");
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
            if (!currentEvent || !currentData) {
                currentEvent = "";
                currentData = "";
                return;
            }
            try {
                dispatch(currentEvent, JSON.parse(currentData));
            }
            catch { /* malformed JSON — skip */ }
            currentEvent = "";
            currentData = "";
        };
        const dispatch = (event, data) => {
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
                    const ms = data["mission_state"];
                    const payload = (ms?.["data_payload"] ?? {});
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
                if (done)
                    break;
                lineBuffer += decoder.decode(value, { stream: true });
                const lines = lineBuffer.split("\n");
                lineBuffer = lines.pop() ?? "";
                for (const line of lines) {
                    if (line.startsWith("event:")) {
                        currentEvent = line.slice(6).trim();
                    }
                    else if (line.startsWith("data:")) {
                        currentData = line.slice(5).trim();
                    }
                    else if (line === "" || line === "\r") {
                        flush();
                    }
                }
            }
            flush();
        }
        catch (err) {
            if (err.name === "AbortError")
                return;
            handlers.onError(err.message ?? "SSE stream interrupted");
        }
    })();
    return () => controller.abort();
}
