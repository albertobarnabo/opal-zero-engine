import { useState, useRef, useCallback } from 'react';
import type {
  MissionStatus,
  MissionState,
  BentoCard,
  UseOpalZeroOptions,
  UseOpalZeroReturn,
  MissionEvent,
  TaskStartedEvent,
  MissionCompleteEvent,
  MissionFailedEvent,
} from '../types';
import { parseBentoCards } from '../parseBentoCards';

export function useOpalZero({ client, model: defaultModel, onEvent }: UseOpalZeroOptions): UseOpalZeroReturn {
  const [status,       setStatus]       = useState<MissionStatus>('idle');
  const [cards,        setCards]        = useState<BentoCard[]>([]);
  const [activeAgent,  setActiveAgent]  = useState<{ role: string; intent: string } | null>(null);
  const [error,        setError]        = useState<string | null>(null);
  const [missionId,    setMissionId]    = useState<string | null>(null);
  const [missionState, setMissionState] = useState<MissionState | null>(null);

  const refinedKeysRef = useRef<Set<string>>(new Set());

  function reset() {
    setStatus('idle');
    setCards([]);
    setActiveAgent(null);
    setError(null);
    setMissionId(null);
    setMissionState(null);
    refinedKeysRef.current = new Set();
  }

  async function drainStream(
    stream: AsyncGenerator<MissionEvent>,
    isRefinement: boolean,
    previousKeys: Set<string>
  ) {
    setStatus('running');
    setActiveAgent(null);
    setError(null);
    try {
      for await (const event of stream) {
        onEvent?.(event);
        switch (event.type) {
          case 'task_started': {
            const e = event as TaskStartedEvent;
            setActiveAgent({ role: e.role, intent: e.intent });
            break;
          }
          case 'task_completed':
          case 'task_failed':
            setActiveAgent(null);
            break;
          case 'mission_complete': {
            const e = event as MissionCompleteEvent;
            setMissionId(e.mission_id);
            const state = e.mission_state ?? null;
            setMissionState(state as MissionState | null);
            if (state) {
              const ms = state as MissionState;
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
                    if (idx >= 0) merged[idx] = card;
                    else merged.push(card);
                  }
                  return merged;
                });
              } else {
                setCards(parsed);
              }
            }
            setStatus('complete');
            break;
          }
          case 'mission_failed': {
            const e = event as MissionFailedEvent;
            setError(e.error);
            setStatus('failed');
            break;
          }
        }
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus('failed');
    }
  }

  const run = useCallback(async (intent: string, model?: string) => {
    reset();
    refinedKeysRef.current = new Set();
    const stream = client.execute(intent, model ?? defaultModel);
    await drainStream(stream, false, new Set());
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, defaultModel]);

  const refine = useCallback(async (id: string, intent: string, model?: string) => {
    if (status === 'running') return;
    const previousKeys = new Set(Object.keys(missionState?.data_payload ?? {}));
    const stream = client.missions.refine(id, intent, model ?? defaultModel);
    await drainStream(stream, true, previousKeys);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, defaultModel, status, missionState]);

  return { run, refine, status, cards, activeAgent, error, missionId, missionState, reset };
}
