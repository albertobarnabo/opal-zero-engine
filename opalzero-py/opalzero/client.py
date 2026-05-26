from __future__ import annotations

from pathlib import Path
from typing import AsyncGenerator, Literal

import httpx

from .stream import stream_mission_events
from .types import (
    ConfigStatus,
    MissionEvent,
    MissionSnapshot,
    MissionSummary,
    UploadResult,
)


class OpalZeroError(Exception):
    """Raised when the OpalZero server returns a non-2xx response."""

    def __init__(self, message: str, code: str, status: int) -> None:
        super().__init__(message)
        self.code   = code
        self.status = status


class OpalZeroClient:
    """
    Async client for the OpalZero Intelligence Kernel API.

    Always use as an async context manager so the underlying
    :class:`httpx.AsyncClient` is properly closed::

        async with OpalZeroClient(base_url="http://localhost:8000") as opalzero:
            async for event in opalzero.execute("Analyse the EV market"):
                if event.type == "task_completed":
                    print(event.role, event.result)
                if event.type == "mission_complete":
                    print("Done:", event.mission_state.data_payload)
    """

    def __init__(
        self,
        base_url:   str        = "http://localhost:8000",
        api_key:    str | None = None,
        openai_key: str | None = None,
        tavily_key: str | None = None,
        timeout:    float      = 120.0,
    ) -> None:
        self._base    = base_url.rstrip("/")
        self._timeout = timeout
        self._http: httpx.AsyncClient | None = None

        # Auth / provider-key headers (no Content-Type — set per-request).
        self._extra_headers: dict[str, str] = {}
        if api_key:    self._extra_headers["X-OpalZero-Key"]  = api_key
        if openai_key: self._extra_headers["X-OpenAI-Key"] = openai_key
        if tavily_key: self._extra_headers["X-Tavily-Key"] = tavily_key

    # ── Context manager ───────────────────────────────────────────────────────

    async def __aenter__(self) -> OpalZeroClient:
        self._http = httpx.AsyncClient(
            base_url=self._base,
            headers=self._extra_headers,
            timeout=self._timeout,
        )
        return self

    async def __aexit__(self, *_: object) -> None:
        if self._http is not None:
            await self._http.aclose()
            self._http = None

    def _client(self) -> httpx.AsyncClient:
        """Return the underlying httpx client, or raise if not entered."""
        if self._http is None:
            raise RuntimeError(
                "OpalZeroClient must be used as an async context manager: "
                "`async with OpalZeroClient(...) as opalzero:`"
            )
        return self._http

    async def _raise_for_error(self, res: httpx.Response) -> None:
        if res.is_success:
            return
        code, message = "UNKNOWN_ERROR", f"HTTP {res.status_code}"
        try:
            body = res.json()
            code    = body.get("code",  code)
            message = body.get("error", message)
        except Exception:
            pass
        raise OpalZeroError(message, code, res.status_code)

    # ── Execute ───────────────────────────────────────────────────────────────

    async def execute(self, intent: str) -> AsyncGenerator[MissionEvent, None]:
        """Start a new mission and stream :class:`MissionEvent` objects."""
        async for event in stream_mission_events(
            self._client(), "POST", "/api/v1/execute",
            json={"intent": intent},
        ):
            yield event

    # ── Missions ──────────────────────────────────────────────────────────────

    async def list_missions(self) -> list[MissionSummary]:
        res = await self._client().get("/api/v1/missions")
        await self._raise_for_error(res)
        return [MissionSummary.model_validate(m) for m in res.json()]

    async def get_mission(self, mission_id: str) -> MissionSnapshot:
        res = await self._client().get(f"/api/v1/missions/{mission_id}")
        await self._raise_for_error(res)
        return MissionSnapshot.model_validate(res.json())

    async def delete_mission(self, mission_id: str) -> None:
        res = await self._client().delete(f"/api/v1/missions/{mission_id}")
        await self._raise_for_error(res)

    async def refine(
        self, mission_id: str, intent: str
    ) -> AsyncGenerator[MissionEvent, None]:
        """Refine an existing mission and stream new :class:`MissionEvent` objects."""
        async for event in stream_mission_events(
            self._client(), "POST", f"/api/v1/missions/{mission_id}/refine",
            json={"intent": intent},
        ):
            yield event

    async def export_mission(
        self, mission_id: str, fmt: Literal["md", "csv", "html"]
    ) -> bytes:
        """Download a mission export as raw bytes (Markdown, CSV, or HTML)."""
        res = await self._client().get(
            f"/api/v1/missions/{mission_id}/export",
            params={"format": fmt},
        )
        await self._raise_for_error(res)
        return res.content

    # ── Upload ────────────────────────────────────────────────────────────────

    async def upload(self, path: str | Path) -> UploadResult:
        """
        Upload a file (image, CSV, JSON, or TXT) to the server.

        Content-Type is intentionally omitted from the request so that httpx
        can set the correct ``multipart/form-data; boundary=...`` header.
        """
        p = Path(path)
        with p.open("rb") as fh:
            res = await self._client().post(
                "/api/v1/upload",
                files={"file": (p.name, fh)},
            )
        await self._raise_for_error(res)
        return UploadResult.model_validate(res.json())

    # ── Config ────────────────────────────────────────────────────────────────

    async def config_status(self) -> ConfigStatus:
        res = await self._client().get("/api/v1/config/status")
        await self._raise_for_error(res)
        return ConfigStatus.model_validate(res.json())
