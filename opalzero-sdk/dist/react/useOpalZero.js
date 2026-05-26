import { useState, useRef, useCallback } from 'react';
import { parseBentoCards } from '../parseBentoCards';
export function useOpalZero({ client, model: defaultModel, onEvent }) {
    const [status, setStatus] = useState('idle');
    const [cards, setCards] = useState([]);
    const [activeAgent, setActiveAgent] = useState(null);
    const [error, setError] = useState(null);
    const [missionId, setMissionId] = useState(null);
    const [missionState, setMissionState] = useState(null);
    const refinedKeysRef = useRef(new Set());
    function reset() {
        setStatus('idle');
        setCards([]);
        setActiveAgent(null);
        setError(null);
        setMissionId(null);
        setMissionState(null);
        refinedKeysRef.current = new Set();
    }
    async function drainStream(stream, isRefinement, previousKeys) {
        setStatus('running');
        setActiveAgent(null);
        setError(null);
        try {
            for await (const event of stream) {
                onEvent?.(event);
                switch (event.type) {
                    case 'task_started': {
                        const e = event;
                        setActiveAgent({ role: e.role, intent: e.intent });
                        break;
                    }
                    case 'task_completed':
                    case 'task_failed':
                        setActiveAgent(null);
                        break;
                    case 'mission_complete': {
                        const e = event;
                        setMissionId(e.mission_id);
                        const state = e.mission_state ?? null;
                        setMissionState(state);
                        if (state) {
                            const ms = state;
                            if (isRefinement) {
                                const newKeys = new Set(Object.keys(ms.data_payload ?? {}));
                                previousKeys.forEach(k => newKeys.delete(k));
                                newKeys.forEach(k => refinedKeysRef.current.add(k));
                            }
                            const parsed = parseBentoCards(ms, { refinedKeys: refinedKeysRef.current });
                            if (isRefinement) {
                                setCards(prev => {
                                    const merged = [...prev];
                                    for (const card of parsed) {
                                        const idx = merged.findIndex(c => c.key === card.key);
                                        if (idx >= 0)
                                            merged[idx] = card;
                                        else
                                            merged.push(card);
                                    }
                                    return merged;
                                });
                            }
                            else {
                                setCards(parsed);
                            }
                        }
                        setStatus('complete');
                        break;
                    }
                    case 'mission_failed': {
                        const e = event;
                        setError(e.error);
                        setStatus('failed');
                        break;
                    }
                }
            }
        }
        catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            setError(msg);
            setStatus('failed');
        }
    }
    const run = useCallback(async (intent, model) => {
        reset();
        refinedKeysRef.current = new Set();
        const stream = client.execute(intent, model ?? defaultModel);
        await drainStream(stream, false, new Set());
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [client, defaultModel]);
    const refine = useCallback(async (id, intent, model) => {
        if (status === 'running')
            return;
        const previousKeys = new Set(Object.keys(missionState?.data_payload ?? {}));
        const stream = client.missions.refine(id, intent, model ?? defaultModel);
        await drainStream(stream, true, previousKeys);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [client, defaultModel, status, missionState]);
    return { run, refine, status, cards, activeAgent, error, missionId, missionState, reset };
}
