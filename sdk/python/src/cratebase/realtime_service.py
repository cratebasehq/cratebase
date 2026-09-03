"""Live record change subscriptions over Server-Sent Events.

Subscribe to a bare collection name for every change, or
`"collection/recordId"` for just one record.

```python
def on_change(event):
    print(event["action"], event["record"])

unsubscribe = cb.realtime.subscribe("posts", on_change)
# later
unsubscribe()
```

**Threading model**: `httpx`'s synchronous client has no event loop to
interleave a long-lived streaming GET with the rest of your program, so
`subscribe()` lazily starts a single background daemon thread the first
time it's called. That thread owns the `/api/realtime` SSE connection,
parses `event:`/`data:` lines itself (this SDK deliberately takes no SSE
dependency), and invokes your callbacks -- **on that background thread,
not the thread that called `subscribe()`**. If your callback touches
thread-unsafe state (most GUI toolkits, a non-thread-safe ORM session,
...) you must marshal back to your own thread/queue yourself. The thread
exits automatically once the last subscription is removed (`disconnect()`
is also called for you at that point).
"""

from __future__ import annotations

import json
import threading
from typing import Any, Callable, Dict, List, Optional, Set, TYPE_CHECKING

import httpx

from .types import RealtimeEvent

if TYPE_CHECKING:
    from .client import Cratebase

RealtimeCallback = Callable[[RealtimeEvent], None]

_CONNECT_TIMEOUT_SECONDS = 10.0


def _is_connect_payload(value: Any) -> bool:
    return isinstance(value, dict) and isinstance(value.get("clientId"), str)


def _is_realtime_event(value: Any) -> bool:
    if not isinstance(value, dict) or "action" not in value or "record" not in value:
        return False
    record = value["record"]
    return isinstance(record, dict) and "collectionName" in record and "id" in record


class RealtimeService:
    def __init__(self, client: "Cratebase") -> None:
        self._client = client
        self._client_id: str = ""
        self._subscriptions: Dict[str, Set[RealtimeCallback]] = {}
        self._lock = threading.Lock()
        self._connected = threading.Event()
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._connect_error: Optional[Exception] = None
        self._response: Optional[httpx.Response] = None

    def subscribe(self, topic: str, callback: RealtimeCallback) -> Callable[[], None]:
        with self._lock:
            self._subscriptions.setdefault(topic, set()).add(callback)
        self._ensure_connected()
        self._sync_subscriptions()

        def unsubscribe() -> None:
            self.unsubscribe(topic, callback)

        return unsubscribe

    def unsubscribe(self, topic: str, callback: Optional[RealtimeCallback] = None) -> None:
        with self._lock:
            subs = self._subscriptions.get(topic)
            if subs is None:
                return
            if callback is not None:
                subs.discard(callback)
            else:
                subs.clear()
            if not subs:
                self._subscriptions.pop(topic, None)
            everything_empty = not self._subscriptions

        if everything_empty:
            self.disconnect()
        else:
            self._sync_subscriptions()

    def disconnect(self) -> None:
        """Closes the SSE connection and stops the background thread, if
        any. Safe to call even when not connected."""
        self._stop.set()
        response = self._response
        if response is not None:
            try:
                response.close()
            except Exception:
                pass
        thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=5)
        with self._lock:
            self._subscriptions.clear()
        self._thread = None
        self._response = None
        self._client_id = ""
        self._connected.clear()
        self._stop.clear()

    def _ensure_connected(self) -> None:
        if self._thread is not None and self._thread.is_alive():
            if not self._connected.wait(timeout=_CONNECT_TIMEOUT_SECONDS):
                raise TimeoutError("failed to connect to /api/realtime")
            return

        self._connected.clear()
        self._connect_error = None
        self._stop.clear()
        self._thread = threading.Thread(target=self._run, name="cratebase-realtime", daemon=True)
        self._thread.start()
        if not self._connected.wait(timeout=_CONNECT_TIMEOUT_SECONDS):
            raise TimeoutError("failed to connect to /api/realtime")
        if self._connect_error is not None:
            error, self._connect_error = self._connect_error, None
            raise error

    def _run(self) -> None:
        url = f"{self._client.base_url}/api/realtime"
        try:
            with httpx.stream("GET", url, timeout=None) as response:
                self._response = response
                event_name = "message"
                data_lines: List[str] = []
                for line in response.iter_lines():
                    if self._stop.is_set():
                        break
                    if line == "":
                        if data_lines:
                            self._dispatch(event_name, "\n".join(data_lines))
                        event_name = "message"
                        data_lines = []
                        continue
                    if line.startswith(":"):
                        continue  # SSE comment/keepalive
                    if line.startswith("event:"):
                        event_name = line[len("event:") :].strip()
                    elif line.startswith("data:"):
                        data_lines.append(line[len("data:") :].strip())
        except Exception as exc:  # noqa: BLE001 - surfaced to the waiting subscribe() caller
            self._connect_error = exc
        finally:
            # Unblocks any subscribe() waiting on the first connection even
            # if we never got a PB_CONNECT (matches the JS SDK's
            # EventSource.onerror behavior: reject only if never connected).
            self._connected.set()

    def _dispatch(self, event_name: str, data_raw: str) -> None:
        try:
            parsed = json.loads(data_raw)
        except json.JSONDecodeError:
            return

        if event_name == "PB_CONNECT":
            if _is_connect_payload(parsed):
                self._client_id = parsed["clientId"]
                self._connected.set()
            return

        if not _is_realtime_event(parsed):
            return
        record = parsed["record"]
        topics = [record["collectionName"], f"{record['collectionName']}/{record['id']}"]
        with self._lock:
            callbacks: List[RealtimeCallback] = []
            for topic in topics:
                callbacks.extend(self._subscriptions.get(topic, ()))
        for callback in callbacks:
            callback(parsed)

    def _sync_subscriptions(self) -> None:
        if not self._client_id:
            return
        with self._lock:
            topics = list(self._subscriptions.keys())
        self._client.send(
            "/api/realtime",
            method="POST",
            body={"clientId": self._client_id, "subscriptions": topics},
        )
