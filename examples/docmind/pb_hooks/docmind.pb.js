/// <reference path="./types.d.ts" />
// DocMind — the AI-flagship worked example from
// docs/superpowers/specs/2026-09-04-value-add-strategy.md §8: an internal
// knowledge-base copilot built entirely out of collections + one hooks
// file, no separate vector DB / RAG service.
//
// Three things live here:
//
//   1. `onRecordAfterCreateSuccess` on `docs` — reads a freshly uploaded
//      file straight off local disk and writes one `chunks` record per
//      chunk, text only (no vector yet — see "Why embedding is a
//      one-minute-later cron backfill" below).
//   2. A `docmind_index` cron job — finds any `chunks` row still missing
//      its vector and PATCHes it through the real record-update route,
//      which is what actually runs the vector field's `embedding` auto-
//      embed config. This is the only thing in this file that computes an
//      embedding; the hook above never touches a float array.
//   3. `routerAdd("POST", "/docmind/ask", ...)` — the retrieval-augmented
//      chat endpoint the UI calls. It exists as a custom endpoint (the
//      W8 "BFF" pattern — see the spec doc's §6 "already covered by W8"
//      note) rather than the browser calling `?nearestTo=` directly,
//      because turning a raw question into a *query vector* requires
//      writing a throwaway record through the real auto-embedding
//      pipeline (see "Why a throwaway record" below), which needs
//      privileges an ordinary employee account doesn't have.
//
// # Why embedding is a one-minute-later cron backfill, not inline
//
// The obvious design — chunk the file and `POST
// /api/collections/chunks/records` for each piece, right there in the
// `docs` create hook, letting the vector field's `embedding` config do
// the rest — does not work in this server, for a reason worth recording
// so nobody "fixes" this file back into it:
//
// `write_record`'s after-success hook (`crates/server/src/routes/
// records.rs::finish`) still runs *inside* the same database transaction
// the `docs` row is being created in — `run_request` passes the entire
// validate → execute → finish chain, after-success included, as the one
// closure `App::run_scoped` wraps in a transaction. A `$http.send` call
// made from inside that hook is a genuinely separate HTTP request on a
// separate DB connection/read snapshot, which — under WAL, same as every
// other reader — cannot see the still-uncommitted `docs` row yet. Live-
// verified twice while building this: the self-`GET /api/files/docs/...`
// call 404'd with "Missing collection context", and switching to
// `onRecordCreateRequest` + `e.next()` (expecting "outer wrapper, so
// `next()` blocks until the transaction commits") did not help either —
// `bind_js_hook` (`crates/server/src/hooks.rs`) runs a JS handler to
// completion *before* deciding whether to continue the chain
// (`if outcome.next_called { e.next().await }`), so `e.next()` called
// from JS only sets a flag; the real downstream write happens after the
// handler function has already returned, never interleaved with it.
// There is no hook stage exposed to JS that fires *after* a record's own
// transaction commits.
//
// So the write side splits in two:
//   - Chunking happens immediately, inside the same transaction, using
//     `e.app.save()` (transaction-safe — same executor as the `docs`
//     row's own still-open write, so the `docId` relation resolves fine)
//     — but `$app.save()` (`HostApi::save_record`) does not run
//     `crate::embeddings::apply_embeddings`, so these rows are created
//     with an empty `embedding`.
//   - The `docmind_index` cron job (below) runs once a minute — croner's
//     finest resolution; see its own comment — long after any triggering
//     transaction has committed, so its `$http.send` calls are genuinely
//     unblocked. It finds every `chunks` row with an empty `embedding`
//     and `PATCH`es it with `{text: <same text>}`: re-supplying the
//     source field is what makes `apply_embeddings` recompute an
//     existing record's vector (see its own "leaves a previously
//     computed vector alone" doc comment) through the one code path that
//     actually runs it.
//
// Net effect: a `chunks` row (text, `docId`) appears the instant a `docs`
// upload completes; its `embedding` fills in within the next cron tick
// (≤60s). README.md's verification section timed this live.

const CHUNK_SIZE = 800; // characters; "simple fixed-size" per the spec, no overlap.
const NEAREST_LIMIT = 5;
const INDEX_CRON_ID = "docmind_index";
const INDEX_CRON_EXPR = "* * * * *"; // every minute — croner normalises the seconds field away (crates/server/src/cron.rs), so this is as fine-grained as cronAdd gets.

function baseUrl() {
  return $app.settings().meta.appURL;
}

// Mints a fresh superuser bearer token in-process (no login round trip,
// no stored credential) so the cron job below can drive the *real*
// record-update route — the only thing that runs `apply_embeddings` —
// for a collection (`chunks`) an ordinary employee account has no write
// access to.
function mintSuperuserToken() {
  const su = $app.findFirstRecordByFilter("_superusers", 'id != ""');
  return $tokens.recordAuthToken($app, su);
}

// A thin `fetch`-alike over `$http.send`: JSON in, JSON out, throws on
// any non-2xx so callers don't have to check `statusCode` everywhere.
function apiCall(method, path, token, body) {
  const headers = { "Content-Type": "application/json" };
  if (token) headers["Authorization"] = "Bearer " + token;
  const res = $http.send({ url: baseUrl() + path, method: method, headers: headers, body: body });
  if (res.statusCode >= 300) {
    throw new Error("cratebase " + method + " " + path + " -> " + res.statusCode + ": " + res.raw);
  }
  return res.json;
}

function chunkText(text, size) {
  const chunks = [];
  for (let i = 0; i < text.length; i += size) {
    const piece = text.slice(i, i + size).trim();
    if (piece.length > 0) chunks.push(piece);
  }
  return chunks;
}

// Reads an uploaded file's bytes straight off local disk
// (`<dataDir>/storage/<collectionId>/<recordId>/<filename>`, the layout
// `crates/server/src/routes/common.rs::file_key` uses for the default,
// non-S3 `Storage` backend — see `crates/storage/src/lib.rs`), instead of
// `GET /api/files/...`: that route does its own DB lookup to authorize
// the download, which is exactly the query this hook cannot make yet
// (see the module doc above). The bytes themselves are already durably
// on disk by this point — `common::store_uploads` runs *before* the
// transaction `write_record` (and this hook) executes inside — so a
// direct file read has no such visibility problem.
//
// `DOCMIND_DATA_DIR` must match whatever `--dir`/`CRATEBASE_DATA_DIR`
// the server was actually started with (default `./pb_data`) — see
// README.md. Only covers local-disk storage, not an S3-backed bucket;
// noted as a known limitation, not fixed here.
function readUploadedFile(collectionId, recordId, filename) {
  const dataDir = ($os.getenv("DOCMIND_DATA_DIR") || "./pb_data").replace(/\/+$/, "");
  const path = dataDir + "/storage/" + collectionId + "/" + recordId + "/" + filename;
  if (!$os.exists(path)) return null;
  return $os.readFile(path);
}

// ---------------------------------------------------------------------
// 1. Upload -> chunk (immediate, text only)
// ---------------------------------------------------------------------

onRecordAfterCreateSuccess((e) => {
  try {
    const filename = e.record.getString("file");
    if (!filename) {
      // A `docs` row created without a file (e.g. a manual dashboard
      // entry) has nothing to chunk.
      return;
    }

    const text = readUploadedFile(e.record.collectionId, e.record.id, filename);
    if (text === null) {
      $app.logger().error("docmind: uploaded file not found on local disk", "doc", e.record.id, "filename", filename);
      return;
    }

    // No PDF/DOCX text extraction here on purpose (out of scope per the
    // assignment's "no need for anything fancy") — this only makes sense
    // for a plain-text upload.
    const pieces = chunkText(text, CHUNK_SIZE);
    const chunksCollection = $app.findCollectionByNameOrId("chunks");
    for (let i = 0; i < pieces.length; i++) {
      const chunk = new Record(chunksCollection, { docId: e.record.id, text: pieces[i], chunkIndex: i });
      // Same transaction as the `docs` row's own still-open create — see
      // the module doc for why this is `e.app.save()`, not an HTTP call,
      // and why its `embedding` is filled in later by `docmind_index`.
      e.app.save(chunk);
    }
    $app.logger().info(
      "docmind: chunked a document; embeddings backfill on the next docmind_index tick",
      "doc",
      e.record.id,
      "chunks",
      pieces.length,
    );
  } catch (err) {
    $app.logger().error("docmind: chunking hook failed", "doc", e.record.id, "error", String(err));
  }
}, "docs");

// ---------------------------------------------------------------------
// 1b. Embedding backfill: PATCH every un-embedded chunk through the
//     real record-update route once a minute.
// ---------------------------------------------------------------------

cronAdd(INDEX_CRON_ID, INDEX_CRON_EXPR, () => {
  try {
    const candidates = $app.findRecordsByFilter("chunks", 'id != ""', "-created", 500);
    const pending = candidates.filter((r) => {
      const v = r.get("embedding");
      return !v || v.length === 0;
    });
    if (pending.length === 0) return;

    const token = mintSuperuserToken();
    for (const chunk of pending) {
      // Re-supplying `text` (even unchanged) is what makes
      // `apply_embeddings` recompute this record's vector — see the
      // module doc.
      apiCall("PATCH", "/api/collections/chunks/records/" + chunk.id, token, { text: chunk.getString("text") });
    }
    $app.logger().info("docmind: backfilled embeddings", "count", pending.length);
  } catch (err) {
    $app.logger().error("docmind: embedding backfill cron failed", "error", String(err));
  }
});

// ---------------------------------------------------------------------
// 2. Retrieval-augmented chat: POST /docmind/ask { question, clientId? }
// ---------------------------------------------------------------------

routerAdd(
  "POST",
  "/docmind/ask",
  (e) => {
    try {
      const info = e.requestInfo();
      const question = info.body && info.body.question ? String(info.body.question).trim() : "";
      const clientId = info.body && info.body.clientId ? String(info.body.clientId) : undefined;
      const docId = info.body && info.body.docId ? String(info.body.docId) : undefined;
      if (!question) {
        throw new BadRequestError("question is required.");
      }

      const suToken = mintSuperuserToken();

      // Why a throwaway record: the only code path that computes an
      // "echo"-provider embedding is `apply_embeddings`, which only runs
      // on a real `POST /api/collections/{c}/records` call (see this
      // file's module doc). To rank `chunks` against the caller's raw
      // question we need *a* vector computed the same way every chunk's
      // vector was — so we round-trip the question through `chunks`
      // itself, read back the vector the server just computed, use it
      // for one `?nearestTo=` ranked read, then delete the row. This
      // never leaves a `__query__`-style placeholder chunk lying around,
      // and needs no bespoke embedding-only endpoint.
      const queryRecord = apiCall("POST", "/api/collections/chunks/records", suToken, {
        text: question,
        chunkIndex: -1,
      });

      let filter = 'id != "' + queryRecord.id + '"';
      if (docId) filter += ' && docId = "' + docId.replace(/"/g, '\\"') + '"';
      const qs =
        "?nearestTo=" +
        encodeURIComponent("embedding:" + queryRecord.id) +
        "&nearestLimit=" +
        NEAREST_LIMIT +
        "&filter=" +
        encodeURIComponent(filter);
      const nearest = apiCall("GET", "/api/collections/chunks/records" + qs, suToken);

      apiCall("DELETE", "/api/collections/chunks/records/" + queryRecord.id, suToken);

      const sources = (nearest.items || []).map((r) => ({ id: r.id, docId: r.docId, text: r.text }));
      const context = sources.length
        ? sources.map((s, i) => "[" + (i + 1) + "] " + s.text).join("\n\n")
        : "(no matching documents found — the knowledge base may be empty.)";

      const messages = [
        {
          role: "system",
          content:
            "You are DocMind, an internal knowledge-base copilot. Answer the " +
            "question using only the numbered context below; cite sources by " +
            "number. If the context doesn't answer the question, say so.\n\n" +
            context,
        },
        { role: "user", content: question },
      ];

      // Everything from here runs *as the calling employee*, not the
      // superuser above — `e.auth` is guaranteed non-null by the
      // `requireAuth("users")` middleware this route is registered
      // with. Minting a fresh token from their own record (rather than
      // reusing whatever bearer token they sent) sidesteps having to
      // parse/forward the `Authorization` header.
      const callerToken = $tokens.recordAuthToken($app, e.auth);

      // The gateway's own `collection` option (`POST /api/llm/chat`)
      // would persist only `{prompt, response, model}` with no
      // request-scoping field (see
      // `crates/server/src/routes/llm.rs::persist_exchange`) — it can't
      // satisfy `messages`' per-employee `authRule` on its own. So this
      // hook calls the gateway *without* `collection` and persists the
      // exchange itself, one line down, with an explicit `author` field
      // the gateway has no way to fill in.
      const chatResult = apiCall("POST", "/api/llm/chat", callerToken, {
        messages: messages,
        clientId: clientId,
      });

      const saved = apiCall("POST", "/api/collections/messages/records", callerToken, {
        author: e.auth.id,
        prompt: question,
        response: chatResult.reply,
        model: $app.settings().llm.model,
      });

      e.json(200, {
        reply: chatResult.reply,
        promptTokens: chatResult.promptTokens,
        completionTokens: chatResult.completionTokens,
        sources: sources,
        message: saved,
      });
    } catch (err) {
      if (err instanceof ApiError) throw err;
      throw new InternalServerError(String((err && err.message) || err));
    }
  },
  $apis.requireAuth("users"),
);
