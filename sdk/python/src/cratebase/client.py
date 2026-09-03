"""The `Cratebase` client -- entry point for the whole SDK."""

from __future__ import annotations

import json as _json
from typing import Any, Dict, Optional, TYPE_CHECKING

import httpx

from .auth_store import AuthStore
from .errors import ClientResponseError

if TYPE_CHECKING:
    from .admin_service import AdminService
    from .feature_flags_service import FeatureFlagsService
    from .queue_service import QueueService
    from .realtime_service import RealtimeService
    from .record_service import RecordService
    from .schema_service import SchemaService


class Cratebase:
    """Cratebase API client. One instance per backend URL; safe to share
    across your app -- records/collections/auth all read `auth_store`
    lazily on every request, so logging in updates every subsequent call.

    ```python
    cb = Cratebase("https://api.example.com")
    cb.collection("users").auth_with_password("a@b.com", "secret")
    posts = cb.collection("posts").get_list(1, 20, filter="published = true")
    ```

    Uses a synchronous `httpx.Client` under the hood. Every method blocks
    until the response arrives; there is no `async`/`await` in this SDK.
    """

    def __init__(self, base_url: str = "/", auth_store: Optional[AuthStore] = None) -> None:
        self.base_url: str = base_url.rstrip("/")
        self.auth_store: AuthStore = auth_store if auth_store is not None else AuthStore()
        self._http = httpx.Client(base_url=self.base_url)

        # Imported lazily (not at module scope) to avoid a circular import:
        # every service module imports `Cratebase` only for type hints.
        from .admin_service import AdminService
        from .feature_flags_service import FeatureFlagsService
        from .queue_service import QueueService
        from .realtime_service import RealtimeService
        from .schema_service import SchemaService

        self.admins: "AdminService" = AdminService(self)
        self.collections: "SchemaService" = SchemaService(self)
        self.realtime: "RealtimeService" = RealtimeService(self)
        self.feature_flags: "FeatureFlagsService" = FeatureFlagsService(self)
        self.queue: "QueueService" = QueueService(self)

    def collection(self, id_or_name: str) -> "RecordService":
        """A `RecordService` bound to one collection (by id or name)."""
        from .record_service import RecordService

        return RecordService(self, id_or_name)

    def get_file_url(self, record: Dict[str, Any], filename: str) -> str:
        """The download URL for a file field's stored filename."""
        collection = record.get("collectionName") or record.get("collectionId")
        return f"{self.base_url}/api/files/{collection}/{record['id']}/{filename}"

    def close(self) -> None:
        """Closes the underlying HTTP connection pool and any open
        realtime SSE connection. Call when you're done with the client
        (or use it as a context manager)."""
        self.realtime.disconnect()
        self._http.close()

    def __enter__(self) -> "Cratebase":
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def send(
        self,
        path: str,
        *,
        method: str = "GET",
        body: Optional[Dict[str, Any]] = None,
        files: Optional[Dict[str, Any]] = None,
        query: Optional[Dict[str, Any]] = None,
        headers: Optional[Dict[str, str]] = None,
    ) -> Any:
        """Low-level request helper every service is built on. Raises
        `ClientResponseError` for non-2xx responses.

        `body` is sent as JSON unless `files` is given, in which case
        `body` becomes the multipart form fields and `files` the file
        parts (same shapes `httpx` accepts for `data=`/`files=`) -- the
        Python equivalent of passing a JS SDK a `FormData` body.
        """
        params: Dict[str, Any] = {k: v for k, v in (query or {}).items() if v is not None}
        request_headers: Dict[str, str] = dict(headers or {})
        if self.auth_store.token:
            request_headers["authorization"] = f"Bearer {self.auth_store.token}"

        kwargs: Dict[str, Any] = {"params": params, "headers": request_headers}
        if files is not None:
            kwargs["data"] = body or {}
            kwargs["files"] = files
        elif body is not None:
            kwargs["json"] = body

        response = self._http.request(method, path, **kwargs)

        if response.status_code == 204:
            return None

        text = response.text
        data = _json.loads(text) if text else None

        if response.is_error:
            raise ClientResponseError(str(response.url), response.status_code, data)
        return data
