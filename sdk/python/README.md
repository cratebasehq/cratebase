# cratebase

Official Python client SDK for [Cratebase](../../README.md).

```bash
pip install cratebase
```

```python
from cratebase import Cratebase, ClientResponseError

cb = Cratebase("http://127.0.0.1:8090")

# auth
cb.collection("users").auth_with_password("alice@example.com", "secret123")

# records
page = cb.collection("posts").get_list(1, 20, filter="published = true", sort="-created")
post = cb.collection("posts").create({"title": "Hello", "published": True})

# files: pass `files=` for collections with file fields (same shape httpx accepts for `files=`)
with open("report.pdf", "rb") as fh:
    doc = cb.collection("docs").create(
        {"title": "Report"},
        files={"attachment": ("report.pdf", fh, "application/pdf")},
    )
url = cb.get_file_url(doc, doc["attachment"])

# errors
try:
    cb.collection("posts").get_one("missing-id")
except ClientResponseError as e:
    print(e.status, e.data)

# realtime — runs a background thread; callback fires off the calling thread
unsubscribe = cb.realtime.subscribe("posts", lambda e: print(e["action"], e["record"]))
# later
unsubscribe()
```

Synchronous throughout (built on `httpx.Client`) — every method blocks until the response arrives, no `async`/`await` required. Fully typed: `RecordModel`/`AuthRecord` are `Dict[str, Any]` (record shape is defined per-collection by your schema, not known ahead of time), everything else — `ListResult`, `CollectionModel`, `AuthResponse`, etc. — is a `TypedDict`/`dataclass` you get real type checking on.

Requires Python 3.9+.

See the [API reference](../../openapi.yaml) for the full request/response shapes this client wraps.
