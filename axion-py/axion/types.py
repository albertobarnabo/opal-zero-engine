from __future__ import annotations

from typing import Annotated, Any, Literal, Union
from pydantic import BaseModel, ConfigDict, Field

_extra = ConfigDict(extra="allow")

# ── Tasks & mission lifecycle ─────────────────────────────────────────────────

class Task(BaseModel):
    model_config = _extra
    slug:   str
    role:   str
    intent: str
    status: Literal["pending", "running", "completed", "failed"]
    result: str | None = None


class MissionState(BaseModel):
    model_config = _extra
    data_payload:      dict[str, Any] = {}
    design_tokens:     dict[str, str] | None = None
    layout_hint:       str | None = None
    suggested_widgets: list[str] | None = None


class MissionSnapshot(BaseModel):
    model_config = _extra
    mission_id:    str
    intent:        str
    plan:          list[Task]
    mission_state: MissionState
    status:        str


class MissionSummary(BaseModel):
    model_config = _extra
    id:         str
    intent:     str
    status:     str
    created_at: str | None = None


# ── SSE event models ──────────────────────────────────────────────────────────
# Field names and types are derived from axion-core/src/protocol/mod.rs
# (MissionUpdate enum, #[serde(tag = "type", rename_all = "snake_case")]).

class TaskStartedEvent(BaseModel):
    model_config = _extra
    type:   Literal["task_started"]
    slug:   str
    role:   str
    intent: str


class TaskCompletedEvent(BaseModel):
    model_config = _extra
    type:   Literal["task_completed"]
    slug:   str
    role:   str
    result: str


class TaskFailedEvent(BaseModel):
    model_config = _extra
    # Note: Rust's TaskFailed carries only slug + role — no error message.
    type: Literal["task_failed"]
    slug: str
    role: str


class GovernorExpandEvent(BaseModel):
    model_config = _extra
    type:           Literal["governor_expand"]
    new_task_count: int
    descriptions:   list[str]


class MissionCompleteEvent(BaseModel):
    model_config = _extra
    type:                Literal["mission_complete"]
    mission_id:          str
    intent:              str
    task_count:          int
    expanded_task_count: int
    layout_hint:         str
    mission_state:       MissionState | None = None


class MissionFailedEvent(BaseModel):
    model_config = _extra
    type:  Literal["mission_failed"]
    error: str


class MissionPausedEvent(BaseModel):
    model_config = _extra
    type:       Literal["mission_paused"]
    question:   str
    mission_id: str


class AwaitingFeedbackEvent(BaseModel):
    model_config = _extra
    type:     Literal["awaiting_feedback"]
    slug:     str
    question: str | None = None


class UnknownEvent(BaseModel):
    model_config = _extra
    type: str


# Discriminated union — pydantic uses the `type` field to pick the right model.
# UnknownEvent has no Literal, so it must stay outside the discriminator and
# be handled as a fallback after the known variants.
_KnownEvent = Annotated[
    Union[
        TaskStartedEvent,
        TaskCompletedEvent,
        TaskFailedEvent,
        GovernorExpandEvent,
        MissionCompleteEvent,
        MissionFailedEvent,
        MissionPausedEvent,
        AwaitingFeedbackEvent,
    ],
    Field(discriminator="type"),
]

MissionEvent = Union[_KnownEvent, UnknownEvent]


# ── Upload & config ───────────────────────────────────────────────────────────

class UploadResult(BaseModel):
    model_config = _extra
    filename:      str
    file_type:     str
    original_name: str


class ConfigStatus(BaseModel):
    model_config = _extra
    openai: bool
    tavily: bool
