"""Holds the current auth token + record/admin.

Mirrors `sdk/js/src/auth-store.ts`, but Python has no browser
`localStorage` to persist into automatically, so `AuthStore` simply holds
state in memory for the process lifetime -- matching the JS SDK's own
non-browser fallback (SSR, React Native without a storage polyfill, CLI
scripts). Attach an `on_change` listener, or subclass and override
`save`/`clear`, if you need custom persistence (a file, a keyring entry, an
httpOnly cookie, a session table).
"""

from __future__ import annotations

import base64
import binascii
import json
import time
from typing import Any, Callable, Dict, List, Optional

AuthModel = Optional[Dict[str, Any]]
Listener = Callable[[str, AuthModel], None]


class AuthStore:
    def __init__(self) -> None:
        self._token: str = ""
        self._model: AuthModel = None
        self._listeners: List[Listener] = []

    @property
    def token(self) -> str:
        return self._token

    @property
    def model(self) -> AuthModel:
        return self._model

    @property
    def is_valid(self) -> bool:
        """`True` if a token is present and (when it's a JWT with an `exp`
        claim) not yet expired. Tokens without a parseable `exp` are
        assumed valid -- the server is still the source of truth."""
        if not self._token:
            return False
        payload = _decode_jwt_payload(self._token)
        if payload is None or not isinstance(payload.get("exp"), (int, float)):
            return True
        return payload["exp"] * 1000 > time.time() * 1000

    def save(self, token: str, model: AuthModel) -> None:
        self._token = token
        self._model = model
        self._emit()

    def clear(self) -> None:
        self._token = ""
        self._model = None
        self._emit()

    def on_change(self, listener: Listener) -> Callable[[], None]:
        """Registers `listener` to be called with `(token, model)` on every
        `save`/`clear`. Returns an unsubscribe function."""
        self._listeners.append(listener)

        def unsubscribe() -> None:
            if listener in self._listeners:
                self._listeners.remove(listener)

        return unsubscribe

    def _emit(self) -> None:
        for listener in list(self._listeners):
            listener(self._token, self._model)


def _decode_jwt_payload(token: str) -> Optional[Dict[str, Any]]:
    parts = token.split(".")
    if len(parts) != 3:
        return None
    try:
        padded = parts[1] + "=" * (-len(parts[1]) % 4)
        decoded = base64.urlsafe_b64decode(padded)
        return json.loads(decoded)
    except (ValueError, binascii.Error, json.JSONDecodeError):
        return None
