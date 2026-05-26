from __future__ import annotations

import json
from typing import Any, AsyncGenerator

import httpx
from httpx_sse import aconnect_sse
from pydantic import TypeAdapter

from .types import MissionEvent, UnknownEvent

_adapter: TypeAdapter[MissionEvent] = TypeAdapter(MissionEvent)


async def stream_mission_events(
    client: httpx.AsyncClient,
    method: str,
    url: str,
    **kwargs: Any,
) -> AsyncGenerator[MissionEvent, None]:
    """
    Open an SSE connection and yield typed :class:`MissionEvent` objects.

    The OpalZero server uses ``#[serde(tag = "type")]`` so every ``data:`` line
    is self-describing JSON.  The ``event:`` header is redundant and ignored.
    """
    async with aconnect_sse(client, method, url, **kwargs) as event_source:
        async for sse in event_source.aiter_sse():
            raw = sse.data.strip()
            if not raw or raw == "[DONE]":
                continue

            # Parse JSON first — keep `payload` in scope for the except branch.
            payload: Any = None
            try:
                payload = json.loads(raw)
                yield _adapter.validate_python(payload)
            except Exception:
                # Determine the best event type label from whatever we parsed.
                event_type = "parse_error"
                if isinstance(payload, dict):
                    event_type = payload.get("type", "unknown") or "unknown"
                yield UnknownEvent(type=event_type)
