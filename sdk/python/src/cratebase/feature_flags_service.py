"""Client for the `feature-flags` plugin (`crates/server/src/plugins/feature_flags.rs`)."""

from __future__ import annotations

from typing import TYPE_CHECKING
from urllib.parse import quote

if TYPE_CHECKING:
    from .client import Cratebase


class FeatureFlagsService:
    def __init__(self, client: "Cratebase") -> None:
        self._client = client

    def is_enabled(self, key: str) -> bool:
        """An unknown key resolves to `False` rather than raising."""
        result = self._client.send(f"/api/plugins/feature-flags/{quote(key, safe='')}", method="GET")
        return bool(result["enabled"])
