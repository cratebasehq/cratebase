"""Wire-format types for the Cratebase API.

These mirror `sdk/js/src/types.ts` field-for-field, including the JSON
casing the server actually sends (`perPage`, `totalItems`, `collectionId`,
...) rather than translating to `snake_case`, since these are dicts decoded
directly from JSON responses, not Python-native structures.

`RecordModel` and `AuthRecord` are plain `Dict[str, Any]` aliases (not
`TypedDict`s) because record shape is defined per-collection at runtime by
each Cratebase project's schema -- there is no fixed set of keys to
declare ahead of time, only the handful (`id`, `created`, ...) every record
is guaranteed to carry.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, Generic, List, Literal, Optional, TypeVar, TypedDict

# A record from any collection. Always carries at least `id`, `created`,
# `updated`, `collectionId`, `collectionName`; every other key is whatever
# that collection's schema defines.
RecordModel = Dict[str, Any]

# A record from an `auth`-typed collection. Additionally carries `email`
# and `verified` at runtime (not statically declared, same reasoning as
# `RecordModel` above).
AuthRecord = Dict[str, Any]


class AdminModel(TypedDict):
    id: str
    email: str
    created: str
    updated: str


T = TypeVar("T")


@dataclass
class ListResult(Generic[T]):
    page: int
    perPage: int
    totalItems: int
    totalPages: int
    items: List[T]


FieldType = Literal[
    "text",
    "editor",
    "number",
    "bool",
    "email",
    "url",
    "date",
    "autodate",
    "select",
    "json",
    "relation",
    "file",
    "password",
]


class FieldSchema(TypedDict, total=False):
    id: str
    name: str
    type: FieldType
    required: bool
    unique: bool
    options: Dict[str, Any]


CollectionType = Literal["base", "auth", "view"]


class AuthOptions(TypedDict, total=False):
    minPasswordLength: int
    # Which schema field identifies an auth record for login. Defaults to
    # "email"; set to "username" (or any other field name) to log in with
    # something other than an email address.
    identityField: str
    requireEmailVerification: bool
    tokenTtlSeconds: int


class CollectionModel(TypedDict, total=False):
    id: str
    name: str
    type: CollectionType
    schema: List[FieldSchema]
    listRule: Optional[str]
    viewRule: Optional[str]
    createRule: Optional[str]
    updateRule: Optional[str]
    deleteRule: Optional[str]
    authOptions: AuthOptions
    created: str
    updated: str


@dataclass
class ListOptions:
    filter: Optional[str] = None
    sort: Optional[str] = None


@dataclass
class AuthResponse(Generic[T]):
    token: str
    record: T


class AdminAuthResponse(TypedDict):
    token: str
    admin: AdminModel


class OAuth2ProviderInfo(TypedDict):
    name: str
    authUrl: str


class OAuth2Info(TypedDict):
    enabled: bool
    providers: List[OAuth2ProviderInfo]


class AuthMethodsResponse(TypedDict):
    password: bool
    oauth2: OAuth2Info


class RealtimeEvent(TypedDict):
    action: Literal["create", "update", "delete"]
    record: RecordModel


class QueueJob(TypedDict, total=False):
    id: str
    created: str
    updated: str
    collectionId: str
    collectionName: str
    queue: str
    payload: Any
    status: Literal["pending", "processing", "completed", "failed"]
    attempts: int
    maxAttempts: int
    availableAt: str
    lastError: Optional[str]
