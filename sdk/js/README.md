# cratebase

Official JS/TS client SDK for [Cratebase](../../README.md).

```bash
npm install cratebase
```

```ts
import { Cratebase } from "cratebase";

const cb = new Cratebase("http://127.0.0.1:8090");

// auth
await cb.collection("users").authWithPassword("alice@example.com", "secret123");

// records
const page = await cb.collection("posts").getList(1, 20, { filter: 'published = true', sort: '-created' });
const post = await cb.collection("posts").create({ title: "Hello", published: true });

// files: pass a FormData for collections with file fields
const form = new FormData();
form.append("title", "Report");
form.append("attachment", fileInput.files[0]);
const doc = await cb.collection("docs").create(form);
const url = cb.getFileUrl(doc, doc.attachment);

// realtime
const unsubscribe = await cb.realtime.subscribe("posts", (e) => {
  console.log(e.action, e.record);
});
```

Works in browsers, Node.js (18+), and React Native/Expo. Realtime subscriptions need a global `EventSource` — present in every browser and in Expo; on plain Node.js install the `eventsource` package and assign it to `globalThis.EventSource` before using `cb.realtime`.

See the [API reference](../../docs/API.md) for the full request/response shapes this client wraps.
