import type { MissionEvent } from "./types";
/**
 * Turns a streaming `Response` body into an `AsyncGenerator<MissionEvent>`.
 *
 * The Axion server sends SSE with `#[serde(tag = "type")]` JSON payloads,
 * so every `data:` line is self-describing — the `event:` header is redundant
 * and can be safely ignored.
 */
export declare function parseSSEStream(response: Response): AsyncGenerator<MissionEvent>;
