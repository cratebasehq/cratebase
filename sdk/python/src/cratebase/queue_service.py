"""Client for the `queue` plugin (`crates/server/src/plugins/queue.rs`):
durable background job processing backed by a collection, not a separate
broker. Requires an authenticated caller (superuser or an auth-collection
record) -- anonymous enqueueing is rejected server-side."""

from __future__ import annotations

from typing import Any, Optional, TYPE_CHECKING

from .types import QueueJob

if TYPE_CHECKING:
    from .client import Cratebase


class QueueService:
    def __init__(self, client: "Cratebase") -> None:
        self._client = client

    def enqueue(self, queue: str, payload: Optional[Any] = None, max_attempts: Optional[int] = None) -> QueueJob:
        return self._client.send(
            "/api/plugins/queue/enqueue",
            method="POST",
            body={"queue": queue, "payload": payload, "maxAttempts": max_attempts},
        )
