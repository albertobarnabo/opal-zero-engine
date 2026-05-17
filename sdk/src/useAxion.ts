import { useEffect, useRef, useState } from "react";
import { streamMission } from "./stream";
import type { AgentState, MissionStatus, UseAxionOptions, UseAxionResult } from "./types";

export function useAxion<T = Record<string, unknown>>(
  intent: string,
  options?: UseAxionOptions,
): UseAxionResult<T> {
  const [agents, setAgents] = useState<AgentState[]>([]);
  const [data,   setData]   = useState<T | null>(null);
  const [status, setStatus] = useState<MissionStatus>("idle");
  const [error,  setError]  = useState<string | null>(null);

  const slugToRole = useRef<Map<string, string>>(new Map());
  const serverUrl  = options?.config?.serverUrl ?? "http://localhost:8080";

  useEffect(() => {
    if (!intent) return;
    setAgents([]);
    setData(null);
    setStatus("running");
    setError(null);
    slugToRole.current = new Map();

    return streamMission(serverUrl, intent, options?.schema, {
      onTaskStarted(slug, role) {
        slugToRole.current.set(slug, role);
        setAgents((prev) =>
          prev.some((a) => a.slug === slug)
            ? prev
            : [...prev, { slug, role, status: "running" }],
        );
      },
      onTaskCompleted(slug) {
        setAgents((prev) =>
          prev.map((a) => a.slug === slug ? { ...a, status: "completed" as const } : a),
        );
      },
      onTaskFailed(slug) {
        setAgents((prev) =>
          prev.map((a) => a.slug === slug ? { ...a, status: "failed" as const } : a),
        );
      },
      onComplete(payload) {
        setData(payload as T);
        setStatus("complete");
      },
      onError(message) {
        setError(message);
        setStatus("failed");
      },
    }, options?.config);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [intent, serverUrl]);

  return { agents, data, status, error };
}
