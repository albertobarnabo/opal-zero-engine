import type { MissionEvent } from "./types";

/**
 * Turns a streaming `Response` body into an `AsyncGenerator<MissionEvent>`.
 *
 * The Axion server sends SSE with `#[serde(tag = "type")]` JSON payloads,
 * so every `data:` line is self-describing — the `event:` header is redundant
 * and can be safely ignored.
 */
export async function* parseSSEStream(
  response: Response
): AsyncGenerator<MissionEvent> {
  if (!response.body) throw new Error("Response has no body");

  const reader  = response.body.getReader();
  const decoder = new TextDecoder();
  let   buffer  = "";

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        if (!line.startsWith("data:")) continue;
        const raw = line.slice(5).trim();
        if (!raw || raw === "[DONE]") continue;
        try {
          yield JSON.parse(raw) as MissionEvent;
        } catch {
          // malformed line — skip
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}
