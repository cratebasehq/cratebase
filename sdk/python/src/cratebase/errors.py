"""Errors raised by the Cratebase client."""

from __future__ import annotations

from typing import Any, Dict, Optional, TypedDict


class FieldError(TypedDict):
    """A single field's validation failure: a stable machine-readable
    `code` (e.g. `"value_too_short"`) alongside a human-readable
    `message`."""

    code: str
    message: str


class ApiErrorBody(TypedDict, total=False):
    """Shape of every non-2xx JSON response from the Cratebase API."""

    code: int
    message: str
    data: Dict[str, FieldError]


class ClientResponseError(Exception):
    """Raised for any non-2xx response. `data` carries per-field
    validation errors when `status == 400`."""

    def __init__(self, url: str, status: int, body: Optional[Dict[str, Any]]) -> None:
        body = body or {}
        message = body.get("message") or f"request to {url} failed with status {status}"
        super().__init__(message)
        self.url: str = url
        self.status: int = status
        self.data: Dict[str, FieldError] = body.get("data") or {}

    def __repr__(self) -> str:  # pragma: no cover - trivial
        return f"ClientResponseError(status={self.status!r}, url={self.url!r}, data={self.data!r})"
