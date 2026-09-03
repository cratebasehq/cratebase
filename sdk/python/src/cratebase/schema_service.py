"""Manage collection schemas. Requires a superuser token in `client.auth_store`."""

from __future__ import annotations

from typing import Any, Dict, List, TYPE_CHECKING

from .types import CollectionModel

if TYPE_CHECKING:
    from .client import Cratebase


class SchemaService:
    def __init__(self, client: "Cratebase") -> None:
        self._client = client

    def get_list(self) -> List[CollectionModel]:
        return self._client.send("/api/collections", method="GET")

    def get_one(self, id_or_name: str) -> CollectionModel:
        return self._client.send(f"/api/collections/{id_or_name}", method="GET")

    def create(self, data: Dict[str, Any]) -> CollectionModel:
        return self._client.send("/api/collections", method="POST", body=data)

    def update(self, id_or_name: str, data: Dict[str, Any]) -> CollectionModel:
        return self._client.send(f"/api/collections/{id_or_name}", method="PATCH", body=data)

    def delete(self, id_or_name: str) -> None:
        self._client.send(f"/api/collections/{id_or_name}", method="DELETE")
