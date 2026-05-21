import { useEffect, useRef, useState } from "react";
import { streamMission } from "./stream.js";
export function useAxion(intent, options) {
    const [agents, setAgents] = useState([]);
    const [data, setData] = useState(null);
    const [status, setStatus] = useState("idle");
    const [error, setError] = useState(null);
    const slugToRole = useRef(new Map());
    const serverUrl = options?.config?.serverUrl ?? "http://localhost:8080";
    useEffect(() => {
        if (!intent)
            return;
        setAgents([]);
        setData(null);
        setStatus("running");
        setError(null);
        slugToRole.current = new Map();
        return streamMission(serverUrl, intent, options?.schema, {
            onTaskStarted(slug, role) {
                slugToRole.current.set(slug, role);
                setAgents((prev) => prev.some((a) => a.slug === slug)
                    ? prev
                    : [...prev, { slug, role, status: "running" }]);
            },
            onTaskCompleted(slug) {
                setAgents((prev) => prev.map((a) => a.slug === slug ? { ...a, status: "completed" } : a));
            },
            onTaskFailed(slug) {
                setAgents((prev) => prev.map((a) => a.slug === slug ? { ...a, status: "failed" } : a));
            },
            onComplete(payload) {
                setData(payload);
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
