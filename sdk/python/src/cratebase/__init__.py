"""Cratebase Python SDK.

```python
from cratebase import Cratebase

cb = Cratebase("http://127.0.0.1:8090")
cb.collection("users").auth_with_password("a@b.com", "secret")
posts = cb.collection("posts").get_list(1, 20, filter="published = true")
```
"""

from .admin_service import AdminService
from .auth_store import AuthModel, AuthStore
from .client import Cratebase
from .errors import ApiErrorBody, ClientResponseError, FieldError
from .feature_flags_service import FeatureFlagsService
from .queue_service import QueueService
from .realtime_service import RealtimeCallback, RealtimeService
from .record_service import RecordService
from .schema_service import SchemaService
from .types import (
    AdminAuthResponse,
    AdminModel,
    AuthMethodsResponse,
    AuthOptions,
    AuthRecord,
    AuthResponse,
    CollectionModel,
    CollectionType,
    FieldSchema,
    FieldType,
    ListOptions,
    ListResult,
    OAuth2ProviderInfo,
    QueueJob,
    RealtimeEvent,
    RecordModel,
)

__all__ = [
    "Cratebase",
    "AuthStore",
    "AuthModel",
    "ClientResponseError",
    "ApiErrorBody",
    "FieldError",
    "RecordService",
    "AdminService",
    "SchemaService",
    "RealtimeService",
    "RealtimeCallback",
    "RealtimeEvent",
    "FeatureFlagsService",
    "QueueService",
    "QueueJob",
    "RecordModel",
    "AuthRecord",
    "AdminModel",
    "ListResult",
    "ListOptions",
    "FieldType",
    "FieldSchema",
    "CollectionType",
    "CollectionModel",
    "AuthOptions",
    "AuthResponse",
    "AdminAuthResponse",
    "OAuth2ProviderInfo",
    "AuthMethodsResponse",
]

__version__ = "0.1.0"
