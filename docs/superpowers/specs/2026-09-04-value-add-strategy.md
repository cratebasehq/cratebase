# Cratebase — value-add strategy, 2026-09-04

Working notes captured live while W1–W8 (see
`2026-09-04-next-work-handoff.md`) were in flight, so the reasoning isn't
lost before it gets folded into `ROADMAP.md` / `README.md` in W7. Nothing
here is implemented yet — this is the decision record for what to build
next and *why*, plus one explicit rule about how far we're allowed to
diverge from the official SDK while doing it.

## 1a. Flagship positioning: "the backend you pick to build an AI app"

User steer, stated explicitly: this is now the primary lens, not just
one item among many. Re-reads §6's ranked list through "does this make
Cratebase the default choice for an AI app" rather than pure
defensibility × cost. Postgres multi-node realtime stays technically
first (see §6) but the items below are the ones that actually change
what Cratebase *is* for this audience, and jump ahead of everything in
§6 except that one item — because pgvector (below) needs Postgres
anyway, so the two reinforce rather than compete.

1. **Native vector field + ANN search.** The concrete, buildable
   primitive: a `vector` field type backed by `sqlite-vec` on SQLite and
   `pgvector` on Postgres (both are embeddable extensions, not separate
   services — keeps the "one binary" story intact). The pitch writes
   itself: "app DB, auth, and vector search in one binary, no separate
   Pinecone/Weaviate/Qdrant to run." Honest ceiling to state up front:
   this targets app-scale (thousands to low millions of vectors), not
   billion-vector dedicated-vector-DB scale — don't oversell it.
- **Auto-embedding on write.** A vector field with an `embedding: {
  provider, model, sourceField }` config computes the embedding
  server-side on record save (call an external embedding provider —
  OpenAI/Voyage/Cohere/local Ollama — abstracted behind a provider trait
  the same way `crates/mailer` already abstracts SMTP/Resend). Without
  this, a vector field is just a column the app has to fill in by hand;
  with it, "write text, get search" actually holds. This is what makes
  item 1 practically usable, not a separate feature.
2. **MCP server over collections.** Already in §6 item 3; the case for
   ranking it near the top of the *AI* list specifically: the API rules
   already enforce `@request.auth.*`-based access control per record —
   an MCP tool call from an agent authenticates as an ordinary auth
   record (an API key is just another identity) and gets the *same*
   rule enforcement a human REST caller gets, for free. No parallel
   permission system to design or trust. That's the differentiator over
   hand-rolling an MCP server on top of a generic Postgres instance,
   where you'd build authz from scratch.
3. **Scoped LLM gateway primitive.** Not general compute (see the
   Appwrite-Functions rejection in §4) — a narrow, provider-abstracted
   endpoint: `POST /api/llm/chat` calls a configured provider
   (OpenAI/Anthropic/local), streams tokens back over the *existing* SSE
   infrastructure (`crate::realtime` already solves "stream server events
   to a client"), and optionally persists the exchange to an ordinary
   collection (`messages`) the app defines. Turns Cratebase into a
   credible "chat/agent app backend in one binary" — conversation
   history, auth, and streaming completions without standing up a
   separate Node/Python service. Pairs naturally with a per-key usage
   ledger (token counts) for cost tracking, reusing the existing
   rate-limit settings shape.
4. **Schema-derived tool schemas.** A collection's fields already fully
   describe a JSON shape; expose that same shape as a JSON-Schema/
   function-calling tool definition (`GET
   /api/collections/{c}/tool-schema`) so an LLM can be handed
   "create-a-`posts`-record" as a structured-output target without the
   app hand-writing a duplicate schema. Ties directly into the
   schema-as-code/type-generation item in §6 — one schema, three
   consumers (REST validation, generated TS types, LLM tool schema).

**Explicit non-goals, same reasoning as the Appwrite-Functions
rejection in §4:** no hosting/running of arbitrary agent code, no
training/serving our own embedding models (always call out to an
external provider, like the mailer pattern), no pitching this as a
replacement for a dedicated vector DB at massive scale. Each of these
would either break the single-binary story or overextend a narrow,
well-scoped primitive into a maintenance burden.

## 1. The core tension

Parity work (W1–W8) deliberately rides the official `pocketbase` npm SDK
unchanged — that's the whole point of the conformance suite, and it's why
the in-house SDK was deleted (see PR #3). But every interesting value-add
below either needs a new endpoint the official SDK doesn't know about, or
a client-side capability (typed vector search, presence, generated types)
that isn't in PocketBase's surface at all.

**Resolution: don't re-fork the SDK. Layer a thin, optional extension
package on top of it instead.**

- Anything that's transparent at the wire level (Postgres
  `LISTEN`/`NOTIFY` multi-node realtime, presence built on existing
  collections + subscribe) needs **no SDK change** — ship it server-side
  only.
- Anything that needs a new endpoint but no new client ergonomics can go
  through the official SDK's existing escape hatch: `pb.send(path,
  options)`. Zero fork risk, just less discoverable/typed.
- Anything that genuinely needs typed client ergonomics (vector search
  helpers, a nicer presence API, push-subscription registration) ships as
  a **separate, optional** package — working name `@cratebase/extras` —
  that takes an existing `PocketBase` client instance and bolts on
  methods, rather than replacing or wrapping the whole client. A project
  that only wants parity never installs it and pays zero cost; a project
  that wants the extra surface adds one dependency on top of the SDK it
  already has.
- The one case where a full generated client is legitimate: **schema-as-
  code + type generation**. That's a build-time codegen artifact specific
  to one project's schema, not a hand-maintained SDK fork subject to
  upstream drift — different risk profile entirely.

Rule of thumb for every future feature proposal: *does this need a new
package, or does it fit in `pb.send()`?* Default to the latter until
proven otherwise.

## 2. SSR / meta-framework story (TanStack Start, Next.js RSC, SvelteKit)

No code gap today — the official SDK already supports the standard SSR
pattern (per-request client, cookie-hydrated auth store):

```ts
// inside a TanStack Start server function / loader
const pb = new PocketBase(url);
pb.autoCancellation(false); // see gotcha below
pb.authStore.loadFromCookie(getCookie("pb_auth") ?? "");
const posts = await pb.collection("posts").getList(1, 20);
setCookie("pb_auth", pb.authStore.exportToCookie());
```

**Gotcha worth documenting explicitly:** the SDK's default auto-
cancellation (dedupes in-flight duplicate requests, meant for client-side
UI) silently cancels concurrent loader calls on the server, where each
request is independent. `autoCancellation(false)` is required for any
server context. This is the single most common first-time-SSR footgun
with PocketBase-family clients and costs nothing to document.

**Action for W7:** add an `examples/tanstack-start` (or at minimum a
README section) showing this exact pattern. This is a documentation gap,
not a code gap — "works with SSR meta-frameworks out of the box" is a
real selling point once it has a worked example backing it, and it's the
cheapest item in this entire doc.

## 3. Messaging pillar: push notifications, webhooks, SMS

Prompted by "push notif?" — yes, and it's worth widening to a full
**Messaging** pillar rather than shipping push in isolation, because the
three pieces share almost all their infrastructure and the grouping
itself is a legible product story (this is explicitly how Appwrite frames
it, and it's a real gap in PocketBase — no first-party push/SMS/webhooks
of any kind).

- **Outgoing webhooks — build this first.** POST a JSON payload to a
  configured URL on a record/collection event. This is *not new
  infrastructure* — `crates/server/src/hooks.rs`'s event chain and the
  realtime fan-out's per-record rule evaluation already do 90% of the
  work; a webhook is "subscribe like realtime, but the delivery target is
  an HTTP URL instead of an SSE connection." Cheapest of the three,
  highest immediate utility (every "send this to Zapier/Slack/my other
  service" use case), zero third-party account/credential needed to
  demo it. No SDK impact at all — configured server-side (dashboard or a
  `_webhooks` collection), fires regardless of client.
- **Push notifications — real gap, real effort.** PocketBase has none;
  Appwrite does (unified Messaging API), so this is a differentiator
  against PocketBase specifically and parity against Appwrite, not a
  clean moat. Needs:
  - A `_push_subscriptions` (or per-user field) collection storing device
    tokens, keyed by platform (web/iOS/Android).
  - Web Push (VAPID, pure Rust crate available, no external account
    needed) — the easy third.
  - FCM HTTP v1 (Firebase service-account JWT) for Android + as the
    common path for web push on Chrome/Android too — needs a Firebase
    project credential from the operator.
  - APNs (JWT provider token, `.p8` key) for iOS — the fiddliest of the
    three, but only needed for native iOS apps specifically (Android/web
    covered by FCM/VAPID alone).
  - Trigger surface: reuse the same hook points as webhooks (fire on a
    record event) plus a direct `POST /api/push/send` for
    application-triggered notifications (not just record-driven ones —
    "notify this user" is at least as common a use case as "notify on
    this record change").
  - Client-side device registration needs a small SDK surface
    (register/unregister a token) — this is the `@cratebase/extras`
    case from §1, not a core-SDK change: it's optional and most
    collections never touch it.
  - **Honest effort call:** this is the most infrastructure-heavy item in
    this whole doc — three delivery backends, credential management per
    operator, and retry/dead-token cleanup (a push token goes stale
    silently; PocketBase-style "just works" means detecting and pruning
    dead tokens on delivery failure, not leaving that to the operator).
    Sequence it *after* webhooks land and prove the event-subscription
    plumbing, not as the first Messaging item.
- **SMS — lowest priority of the three.** Same shape as email (one
  provider call, e.g. Twilio), reuses `crates/mailer`'s existing
  provider-abstraction pattern almost exactly. Cheap once webhooks/push
  establish the "Messaging" collection conventions, not worth doing
  first since demand is lower than the other two for a typical web/app
  backend.

Recommended sequencing within Messaging: **webhooks → push → SMS**, each
justified independently (webhooks: near-zero-cost immediate value; push:
real gap worth the effort once the plumbing exists; SMS: cheap top-up,
not a reason to start here).

## 4. Appwrite feature-parity scan

Prompted by "ambil feature-feature Appwrite juga" — Appwrite is the
closer competitor for several of these ideas (it already ships
Messaging, Functions, Teams), so it's worth scanning its surface
explicitly rather than only comparing against PocketBase. Not everything
it does fits our positioning; adopt selectively, and write down *why* the
rejected ones are rejected so nobody re-proposes them without the
context.

**Worth adopting:**

- **Teams / memberships.** Application-level multi-user workspaces —
  invite a member, assign a role, scope data per team. This is distinct
  from the "multi-tenant superuser" idea already ruled out in
  `ROADMAP.md`'s "Explicitly out of scope" section (that's about who can
  access the *admin dashboard*; this is an ordinary data-model feature
  for end users, like a Slack workspace or a shared project). Genuinely
  high value for B2B SaaS use cases and one of Appwrite's real draws.
  Cheapest once W8's record-lifecycle hooks exist to enforce
  membership/role checks without touching `crates/auth`.
- **Avatars / QR utility endpoints, with a concrete style now** —
  generated deterministically per identity (collection id / record id):
  a small fixed shape set (circle, hexagon, rounded-square) + one
  deterministic color from a fixed palette (hash of the id) + two simple
  dot eyes — pure SVG, no external asset, no ML. Concrete reference:
  grokbots.ai's card grid (checked live, screenshot on file) — colorful
  gradient blobs with dot eyes, one distinct shape/color per bot,
  instantly recognizable at small size, zero illustration effort per
  entity. Deliberately *not* the Grok Companions style (fully animated,
  voice-responsive 3D characters) — that's a different product
  (consumer chat companion), out of scope for an admin tool. Application
  here: sidebar collection icons and the Users table's record avatar,
  replacing today's plain-initials fallback. Favicon fetch + QR
  generation stay as originally scoped (small utility endpoints, cheap
  given `image` is already a dependency).
- **Messaging pillar** (§3) — the grouping itself (push + SMS + webhooks
  as one API family) is explicitly modeled on how Appwrite frames it.

**Deliberately NOT adopting, with reasons (so it doesn't get
re-proposed without this context):**

- **Multi-language Functions (arbitrary user code in isolated
  containers, Node/Python/Dart/etc.).** This needs container
  orchestration and per-language runtime images — it directly
  contradicts the "one binary, no runtime, no CGO" pitch that's one of
  our two shipping edges (§ README "Same deal on deployment"). The
  moment self-hosting Cratebase requires Docker-in-Docker to run a user
  function, we've stopped being the thing people switch to *for*
  simplicity. W8's embedded QuickJS hooks already cover "run logic near
  your data" without that ops burden — that's the intentional
  differentiator, not a gap.
- **Sites (static hosting).** Outside the core value prop (backend as a
  service, not a hosting platform). Skip entirely.
- **A GraphQL endpoint.** Adds a whole second API surface to maintain
  with no demonstrated demand, and doesn't interact with the "official
  SDK works unchanged" moat at all (that SDK is REST + SSE only). Revisit
  only if a specific integration actually needs it.

## 5. Dashboard: raw SQL console for search/ops

Prompted by "di dashboard kita harus bisa query raw juga untuk search" —
yes, and PocketBase's dashboard has no equivalent (Supabase Studio's SQL
Editor is the closest reference point). The filter language covers
per-collection rule-shaped queries well but can't express joins/
aggregates/ad-hoc exploratory search across tables — a raw SQL console
is the honest answer for that gap, not an extension of the filter
language.

Cheap to build: `crates/db`'s `Executor` trait already runs compiled
arbitrary SQL (`rules.rs`'s `check_via_sql` does exactly this against a
synthetic table). The work is one new endpoint + one dashboard page, not
new database plumbing.

**Non-negotiable safety constraints, since this bypasses the entire rule
engine:**

- **Superuser-only**, same tier as collection schema edits — this is an
  operator tool, never exposed to a `type: "auth"` record.
- **Read-only by default.** Only `SELECT` (and dialect-equivalent
  read-only statements) allowed unless an explicit "unlock write mode"
  toggle is set per-session — mirrors how a production DB console
  usually gates `SELECT` vs `INSERT/UPDATE/DELETE/DDL` separately, and
  keeps the common "let me search for something" case safe by
  construction rather than by operator discipline.
- **Row/time limits enforced server-side** — an unbounded `SELECT *`
  against a large table shouldn't be able to OOM the server or block the
  single-writer SQLite connection for other requests.
- **Explicit non-portability warning in the UI itself**, not just docs:
  unlike every other Cratebase surface (filter language, rules, the REST
  API), raw SQL is dialect-specific — a query written against SQLite
  syntax will not necessarily run against Postgres and vice versa. This
  is the one place where "identical filter syntax either way" (README's
  pitch) doesn't hold, and pretending otherwise would be misleading.

## 5a. Dashboard: bucket/file manager

Prompted by "bikinin file manager juga kali kalo kita pakai r2 which is
s3 compatible" — yes, same "why am I tabbing to another dashboard"
problem as the SQL console above, applied to storage instead of the DB.

Cheap for the same reason: `crates/storage`'s `Storage` trait (local
disk or any S3-compatible bucket via `object_store`) already has
list/get/put/delete — this is one dashboard page over an existing
abstraction, not new storage plumbing. Concretely: prefix-based
"folder" navigation (S3 keys are flat; a `/` in a key renders as a
folder the same way every S3 console does it), per-object
download/delete/rename, drag-and-drop upload, and an image preview for
the file field's own naming convention
(`<collectionId>/<recordId>/<filename>`) so an operator can tell which
record a given object belongs to at a glance.

Two things worth calling out rather than assuming:

- **Same superuser-only gate as the SQL console** — this bypasses
  per-record file-field rules entirely (it's bucket-level, not
  record-level), so it needs the same trust tier as schema edits, not
  the record API's rule engine.
- **Deleting a file this way orphans the record's field reference** —
  unlike deleting through a record update (which clears the field), a
  bucket-level delete leaves the record pointing at a now-404 file. The
  UI needs to say this explicitly (a confirmation dialog naming the
  owning record if one is inferable from the key, not a generic "are you
  sure"), rather than silently create a papercut for whoever hits it.

Ships alongside the SQL console in §6's roadmap — same audience
(self-hosted operators), same effort shape (one page over an existing
trait), same safety tier.

## 6. Value-add roadmap, re-prioritized

Supersedes the ordering in `2026-09-04-next-work-handoff.md` §3 with the
SDK-strategy lens applied, and folds in §1a and §3–5a above. Ranked by
defensibility × cost, with the §1a AI-flagship items called out
separately since that's the stated primary lens, not folded into the
same defensibility × cost ranking as everything else:

**AI-flagship tier (§1a), ranked internally by how foundational each is
to the others:**

1. Native vector field + ANN search + auto-embedding on write.
2. MCP server over collections (reuses existing rule engine for authz).
3. Scoped LLM gateway primitive (chat/completions over existing SSE).
4. Schema-derived tool schemas.

**Everything else, ranked by defensibility × cost:**

1. **Postgres multi-node realtime (`LISTEN`/`NOTIFY`).** The structural
   moat — PocketBase is SQLite-only *by design, permanently*, so this is
   the one item they cannot follow us over. Zero SDK impact (same SSE
   wire format). Also the prerequisite for pgvector above, so it's
   effectively tied with the AI-flagship tier rather than competing with
   it. Ship first.
2. **SSR example/docs (§2) + outgoing webhooks (§3) + avatars/QR (§4) +
   dashboard SQL console (§5) + dashboard file manager (§5a).** All
   trivial-to-moderate effort relative to payoff, all reuse
   infrastructure that already exists (SDK docs; the hook/realtime event
   chain; the `image` crate; `Executor`'s existing raw-SQL path; the
   `Storage` trait's existing list/get/put/delete). Ship alongside W7.
3. **Schema-as-code + generated TS types.** Retention feature for teams
   past the toy-project stage — collections as a checked-in file,
   generated client is a legitimate exception to the "no SDK fork" rule
   (see §1). Independent of the SDK-strategy question otherwise.
4. **Teams / memberships (§4).** Real B2B draw, needs W8's
   record-lifecycle hooks as a prerequisite.
5. **Push notifications (§3).** Real gap vs PocketBase, real effort
   (three delivery backends + credential management + dead-token
   hygiene) — sequence after webhooks prove the plumbing.
6. **Presence.** Buildable entirely on existing collections + realtime
   subscribe — no new server capability, optionally a nicer client
   wrapper in `@cratebase/extras`.
7. **SMS.** Cheap top-up to the Messaging pillar, not a starting point.
8. **WASM plugin registry.** Packaging problem, not a capability gap —
   the in-process Rust `Plugin` trait already works. Defer until there's
   an actual ecosystem demand signal.
9. **Observability (Prometheus/OTel).** Expected table stakes
   eventually, not a differentiator. Cheap, do opportunistically
   alongside other work rather than as a headline feature.

**Not on this list, on purpose:** multi-language Functions and Sites
(§4) — rejected as contrary to the single-binary positioning, not
merely deprioritized.

**Risk flagged separately, not part of the "new value" pitch:** W8 (JS
hooks / `pb_hooks`) is parity, not value-add, but it's the single biggest
*defection* risk today — it's PocketBase's primary extension point, and
"you can't do that yet" is currently the strongest reason a real
PocketBase user would refuse to switch. It's already in the W1–W8 batch;
flagging here so it doesn't get bumped by the fancier ideas above it.

**Also confirmed as already covered by W8, not a new item:** the
"custom aggregating endpoint for a mobile app" (BFF) pattern — `routerAdd`
plus `find_records_by_filter`/`find_record_by_id` in `HostApi` already
support one JS-defined endpoint composing several collection reads into
one response, no recompile needed. Raw-SQL BFF endpoints (joins/
aggregates a filter can't express) go through the Rust `Plugin` trait
instead, which already has full `Executor` access today. Verifying
`routerAdd` specifically (not just record hooks) is now an explicit
acceptance criterion on the in-flight W8 task.

## 7. Who this is actually for (ICP)

Prompted by "siapa sih yang cocok pakai cratebase" — five fits, ranked,
plus explicit non-fits so positioning stays sharp rather than "for
everyone":

1. **Indie dev / small team shipping an MVP fast** — the classic
   PocketBase audience. We're a strictly better fit than PocketBase for
   them specifically because of the one fear that audience has about
   PocketBase: "what happens when I outgrow SQLite." Cheapest to
   validate — this is what W1–W8 parity work already targets.
2. **Teams building an AI/agent product** (chatbot, RAG, internal
   copilot) — §1a's flagship bet. No competitor (PocketBase) has this
   story at all; assembling it themselves means gluing together
   Postgres + pgvector + a hand-rolled authz layer + a hand-rolled MCP
   server.
3. **Small/mid startup wanting instant admin tooling** — §5/§5a's SQL
   console + file manager + Teams/RBAC, without buying into a heavier
   platform (Appwrite Cloud, Firebase) full of unused surface area.
4. **Coding agents provisioning a backend on the fly** — underexplored
   and worth watching: README already pitches "or a coding agent that
   just needs a backend." Tools like v0/bolt.new/Replit-agent/agentic
   coding assistants scaffolding an app for a user need something that
   stands up trivially — one binary, an SDK already known to every LLM's
   training data, zero bespoke config. Nobody has explicitly claimed
   this positioning yet.
5. **Self-hoster / privacy-conscious team** — explicit no to vendor
   lock-in (Firebase/Supabase Cloud), wants full data ownership on their
   own infra.

**Explicit non-fits, so the pitch doesn't get diluted into "for
everyone":**

- Consumer apps at billion-vector / petabyte scale needing a dedicated
  infra team — not our ceiling (see §1a's honesty note on vector search
  scale).
- Teams wanting a no-code visual app builder — we're a backend/API, not
  a builder.
- Teams needing arbitrary multi-language serverless functions — rejected
  in §4, contradicts the single-binary positioning.
- Enterprises needing a support contract/compliance certification today
  — pre-1.0, unproven in production (see `2026-09-04-next-work-handoff.md`
  §3's own "honest weaknesses" list).

## 8. Worked example: "DocMind" (plausible, not hypothetical-vague)

A concrete scenario tying together most of this doc into one coherent
story, requested as "kasih contoh real case yang plausible" — useful
later as a README/`examples/` walkthrough, not just an internal note.

**Setup:** a 20-person startup builds an internal AI knowledge-base
copilot over their own docs.

1. Employees upload PDFs/notes through a normal app UI → Cratebase file
   storage (R2) holds the raw file. A `pb_hooks` JS hook
   (`onRecordCreate` on a `docs` collection, §"BFF"/W8) chunks the text
   and calls an embedding provider, writing `{text, vector}` rows to a
   `chunks` collection (§1a's vector field + auto-embedding). One
   binary now holds app data, files, and the vector index — no separate
   Pinecone/Weaviate to run.
2. An employee asks the copilot a question → `POST /api/llm/chat`
   (§1a's LLM gateway) does an ANN similarity search over `chunks`,
   calls the LLM provider, streams the answer back over the *existing*
   SSE realtime infrastructure, and persists the exchange to a
   `messages` collection. `messages`' `authRule` scopes each employee to
   their own conversations — the same rule engine every REST call
   already goes through, no second authz system.
3. A manager's `manageRule` grants visibility into their team's
   conversations; if departments need hard isolation, that's
   Teams/memberships (§4).
4. Ops debugs a bad retrieval from the dashboard's SQL console (§5) and
   inspects the underlying file straight from the file manager (§5a) —
   no separate Cloudflare R2 console tab.
5. A question tagged "urgent" (a filter rule) fires an outgoing webhook
   (§3) to a Slack channel.
6. Three months later the company wants an external agent (e.g. a
   coding assistant) to search the same knowledge base as a tool — the
   same `docs`/`chunks` collections get exposed through the MCP server
   (§1a item 2). The agent authenticates as an ordinary auth record (an
   API key is just another identity) and gets *exactly* the same
   rule-gated access a human user does — nothing new to design.
7. Deployment stays "one binary + managed Postgres (e.g. Neon) + R2" —
   ops never stands up a separate vector DB, auth service, or realtime
   fan-out layer.

This is the argument for why §1a's items are a *bundle*, not four
independent checkboxes: none of them alone makes this story work, and
together they're a story no PocketBase-shaped competitor can currently
tell.

## 9. Bonus, cheap: self-hosted web analytics (Plausible-style)

Prompted by a Plausible.io dashboard screenshot — privacy-friendly,
self-hosted web analytics (visitors/pageviews/bounce-rate/duration stat
cards, a daily traffic chart, sources and entry-page tables). Relevant
to the self-hoster ICP (§7 item 5): one less SaaS subscription to run
alongside the app.

Cheap specifically because most of the plumbing already exists:
`crate::middleware::request_log` already captures request events, and
the dashboard's request-logs page already has the
table/`SettingsSection` components this would reuse. The net-new pieces
are narrow: a tiny public beacon script (`<script>`, no cookies, no PII
— matches the privacy-friendly framing) that POSTs a pageview event to
an `_analytics` collection from the *visitor's* browser, and one new
dashboard page rendering the stat cards/chart/tables shown in the
reference screenshot.

**Scope caution, stated explicitly so this doesn't creep into
competing for roadmap attention:** this is a different product surface
— website analytics, not app-data backend — and a "nice, we already
have 80% of the plumbing" bonus, not a pillar. It belongs in the same
cheap tier as §5/§5a (SQL console, file manager) in §6's roadmap, not
competing with the §1a AI-flagship bet for priority.

## 10. Open question, deliberately not decided here

Whether `@cratebase/extras` ships from this repo (`sdk/js` currently
exists as an empty directory — leftover from the deleted in-house SDK,
not yet repurposed) or as a separate repo entirely. Doesn't need
resolving until item 3, 6, or 7 above actually gets built.

## 11. Status as of this session — parity work landed

Everything in §1a–§10 above is forward-looking; none of it is implemented.
What *did* land in this same session, closing out the W1–W8 parity batch
this doc's predecessor scoped:

- Batch API (`POST /api/batch`), full auth surface (verification,
  password reset, email change, OTP, MFA, impersonate, `_authOrigins`
  origin tracking + new-location alerts), JS hooks (`pb_hooks/` +
  `routerAdd`, including a live-verified `routerAdd` BFF-style endpoint —
  see §"BFF" above), and first-run superuser setup (the dashboard now
  shows an inline setup form on an empty database instead of a bare login
  screen pointing at a CLI command).
- Conformance: **180 pass, 1 skip, 0 fail of 181** (`tests/conformance`
  against the official `pocketbase` npm SDK v0.28, driven against a real
  built `cratebase serve` binary) — up from the 155/181 baseline this
  doc's predecessor recorded. `cargo test --workspace`, `cargo clippy
  --workspace --all-targets -- -D warnings`, and `cargo fmt --all --
  --check` all pass clean.
- Dashboard gains: backup upload/restore, collection export/import, a
  Network settings page (rate limits/trusted proxy/superuser IPs), a
  geoPoint field editor, per-collection auth-provider UI.
- `examples/kanban/` shipped — a live collaborative Kanban board
  (Vite+React) demonstrating auth+rules+realtime together, live-verified
  across two independent browser contexts.
- W6 (benchmark re-run) is **blocked**, not skipped: the host was under
  memory pressure during this session (swap exhausted, <2.5GB RAM free)
  — the same invalid-measurement condition the predecessor doc already
  flagged. Needs an actually idle host before the numbers in any doc can
  be trusted; do not treat any benchmark figure in README/ARCHITECTURE as
  current until that re-run happens.
