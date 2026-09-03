"""CRUD + auth operations scoped to one collection."""

from __future__ import annotations

from typing import Any, Dict, List, Optional, TYPE_CHECKING

from .types import AuthMethodsResponse, AuthResponse, ListResult, RecordModel

if TYPE_CHECKING:
    from .client import Cratebase


class RecordService:
    """CRUD + auth operations scoped to one collection. Auth-only methods
    (`auth_with_password`, `auth_refresh`, ...) only make sense for
    `auth`-typed collections but are exposed here unconditionally since the
    client has no way to know a collection's type ahead of a request.
    """

    def __init__(self, client: "Cratebase", collection_id_or_name: str) -> None:
        self._client = client
        self._collection_id_or_name = collection_id_or_name

    @property
    def _base_path(self) -> str:
        return f"/api/collections/{self._collection_id_or_name}/records"

    def get_list(
        self,
        page: int = 1,
        per_page: int = 30,
        *,
        filter: Optional[str] = None,
        sort: Optional[str] = None,
    ) -> ListResult[RecordModel]:
        result = self._client.send(
            self._base_path,
            method="GET",
            query={"page": page, "perPage": per_page, "filter": filter, "sort": sort},
        )
        return ListResult(**result)

    def get_full_list(
        self,
        *,
        filter: Optional[str] = None,
        sort: Optional[str] = None,
        batch_size: int = 200,
    ) -> List[RecordModel]:
        """Fetches every page and concatenates the results. Convenient for
        small collections; prefer `get_list` with pagination for large
        ones."""
        items: List[RecordModel] = []
        page = 1
        while True:
            result = self.get_list(page, batch_size, filter=filter, sort=sort)
            items.extend(result.items)
            if page >= result.totalPages:
                break
            page += 1
        return items

    def get_one(self, id: str) -> RecordModel:
        return self._client.send(f"{self._base_path}/{id}", method="GET")

    def create(self, data: Dict[str, Any], *, files: Optional[Dict[str, Any]] = None) -> RecordModel:
        return self._client.send(self._base_path, method="POST", body=data, files=files)

    def update(self, id: str, data: Dict[str, Any], *, files: Optional[Dict[str, Any]] = None) -> RecordModel:
        return self._client.send(f"{self._base_path}/{id}", method="PATCH", body=data, files=files)

    def delete(self, id: str) -> None:
        self._client.send(f"{self._base_path}/{id}", method="DELETE")

    def auth_with_password(self, identity: str, password: str) -> AuthResponse[RecordModel]:
        """`identity` is whatever the collection's configured identity
        field is (email by default, but could be a username)."""
        result = self._client.send(
            f"/api/collections/{self._collection_id_or_name}/auth-with-password",
            method="POST",
            body={"identity": identity, "password": password},
        )
        self._client.auth_store.save(result["token"], result["record"])
        return AuthResponse(token=result["token"], record=result["record"])

    def auth_refresh(self) -> AuthResponse[RecordModel]:
        result = self._client.send(
            f"/api/collections/{self._collection_id_or_name}/auth-refresh",
            method="POST",
        )
        self._client.auth_store.save(result["token"], result["record"])
        return AuthResponse(token=result["token"], record=result["record"])

    def list_auth_methods(self, redirect_uri: Optional[str] = None) -> AuthMethodsResponse:
        """Lists available auth methods. Pass your app's OAuth2 redirect
        URI/deep link and each provider's `authUrl` comes back ready to
        open directly."""
        return self._client.send(
            f"/api/collections/{self._collection_id_or_name}/auth-methods",
            method="GET",
            query={"redirectUri": redirect_uri},
        )

    def auth_with_oauth2(self, provider: str, code: str, redirect_uri: str) -> AuthResponse[RecordModel]:
        """Completes an OAuth2 login: exchange the `code` your app
        received at `redirect_uri` (the same one used to build the
        `authUrl` from `list_auth_methods`) for a session."""
        result = self._client.send(
            f"/api/collections/{self._collection_id_or_name}/auth-with-oauth2",
            method="POST",
            body={"provider": provider, "code": code, "redirectUri": redirect_uri},
        )
        self._client.auth_store.save(result["token"], result["record"])
        return AuthResponse(token=result["token"], record=result["record"])

    def request_verification(self, email: str) -> None:
        """Sends a verification email if `email` matches an account --
        always resolves regardless, so it can't be used to enumerate
        accounts."""
        self._client.send(
            f"/api/collections/{self._collection_id_or_name}/request-verification",
            method="POST",
            body={"email": email},
        )

    def confirm_verification(self, token: str) -> None:
        """Confirms a verification token from the emailed link."""
        self._client.send(
            f"/api/collections/{self._collection_id_or_name}/confirm-verification",
            method="POST",
            body={"token": token},
        )

    def request_password_reset(self, email: str) -> None:
        """Sends a password reset email if `email` matches an account --
        always resolves regardless, so it can't be used to enumerate
        accounts."""
        self._client.send(
            f"/api/collections/{self._collection_id_or_name}/request-password-reset",
            method="POST",
            body={"email": email},
        )

    def confirm_password_reset(self, token: str, password: str, password_confirm: str) -> None:
        """Confirms a password reset token and sets a new password."""
        self._client.send(
            f"/api/collections/{self._collection_id_or_name}/confirm-password-reset",
            method="POST",
            body={"token": token, "password": password, "passwordConfirm": password_confirm},
        )

    def request_email_change(self, new_email: str) -> None:
        """Requires the current session's record to be authenticated.
        Sends a confirmation link to `new_email` -- the identity only
        changes once that link is confirmed, proving ownership of the new
        address."""
        self._client.send(
            f"/api/collections/{self._collection_id_or_name}/request-email-change",
            method="POST",
            body={"newEmail": new_email},
        )

    def confirm_email_change(self, token: str) -> None:
        """Confirms an email-change token from the emailed link."""
        self._client.send(
            f"/api/collections/{self._collection_id_or_name}/confirm-email-change",
            method="POST",
            body={"token": token},
        )
