"""Superuser (admin panel) session management."""

from __future__ import annotations

from typing import TYPE_CHECKING

from .types import AdminAuthResponse

if TYPE_CHECKING:
    from .client import Cratebase


class AdminService:
    def __init__(self, client: "Cratebase") -> None:
        self._client = client

    def auth_with_password(self, email: str, password: str) -> AdminAuthResponse:
        result: AdminAuthResponse = self._client.send(
            "/api/admins/auth-with-password",
            method="POST",
            body={"email": email, "password": password},
        )
        self._client.auth_store.save(result["token"], result["admin"])
        return result

    def auth_refresh(self) -> AdminAuthResponse:
        result: AdminAuthResponse = self._client.send("/api/admins/auth-refresh", method="POST")
        self._client.auth_store.save(result["token"], result["admin"])
        return result
