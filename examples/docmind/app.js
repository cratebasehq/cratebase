// DocMind — Cratebase's AI-flagship worked example (vector search +
// auto-embedding + LLM chat gateway + the pb_hooks BFF pattern), tied
// together per docs/superpowers/specs/2026-09-04-value-add-strategy.md §8.
//
// Imported via the bare specifier "pocketbase", resolved by the import map
// in index.html to the official PocketBase JS SDK on esm.sh — no npm
// install or build step needed, same convention as examples/todo and
// examples/realtime-chat. See README.md for why this example doesn't need
// @cratebase/extras: the retrieval step (turning a raw question into a
// query vector) needs privileges an ordinary employee account doesn't
// have, so it happens server-side in pb_hooks/docmind.pb.js's
// `POST /docmind/ask` — the client only ever calls that one custom
// endpoint and the stock `pocketbase` SDK's realtime subscribe, both of
// which the official SDK already covers with `pb.send`/`pb.realtime`.
import PocketBase from "pocketbase";

const BASE_URL = "http://localhost:8090";

const cb = new PocketBase(BASE_URL);
const users = cb.collection("users");
const docs = cb.collection("docs");
const messages = cb.collection("messages");

// --- DOM refs -------------------------------------------------------------

const statusEl = document.getElementById("status");
const statusTextEl = document.getElementById("statusText");
const bannerEl = document.getElementById("banner");
const whoamiEl = document.getElementById("whoami");
const whoamiEmailEl = document.getElementById("whoamiEmail");

const viewGuest = document.getElementById("view-guest");
const viewApp = document.getElementById("view-app");

const formLogin = document.getElementById("formLogin");
const formRegister = document.getElementById("formRegister");
const authTitle = document.getElementById("authTitle");
const toggleToRegister = document.getElementById("toggleToRegister");
const toggleToLogin = document.getElementById("toggleToLogin");

const uploadForm = document.getElementById("uploadForm");
const uploadButton = document.getElementById("uploadButton");
const docTitleInput = document.getElementById("docTitle");
const docFileInput = document.getElementById("docFile");
const docListEl = document.getElementById("docList");
const docsEmptyEl = document.getElementById("docsEmpty");

const answerEl = document.getElementById("answer");
const sourcesEl = document.getElementById("sources");
const askForm = document.getElementById("askForm");
const askButton = document.getElementById("askButton");
const questionInput = document.getElementById("question");
const historyEl = document.getElementById("history");

// --- Status / banner --------------------------------------------------

function setStatus(state, text) {
  statusEl.dataset.state = state;
  statusTextEl.textContent = text;
}

function showBanner(message) {
  bannerEl.textContent = message;
  bannerEl.classList.add("visible");
}

function hideBanner() {
  bannerEl.classList.remove("visible");
}

// --- Auth: register / sign in, mirrors examples/todo's exact calls -----

toggleToRegister.addEventListener("click", (e) => {
  if (e.target.tagName !== "A") return;
  formLogin.style.display = "none";
  formRegister.style.display = "flex";
  formRegister.style.flexDirection = "column";
  authTitle.textContent = "Create an account";
  toggleToRegister.style.display = "none";
  toggleToLogin.style.display = "inline";
});

toggleToLogin.addEventListener("click", (e) => {
  if (e.target.tagName !== "A") return;
  formRegister.style.display = "none";
  formLogin.style.display = "flex";
  formLogin.style.flexDirection = "column";
  authTitle.textContent = "Sign in";
  toggleToLogin.style.display = "none";
  toggleToRegister.style.display = "inline";
});

formLogin.addEventListener("submit", async (e) => {
  e.preventDefault();
  hideBanner();
  try {
    await users.authWithPassword(document.getElementById("li-email").value, document.getElementById("li-password").value);
  } catch (err) {
    showBanner(err && err.message ? err.message : "Sign in failed.");
  }
});

formRegister.addEventListener("submit", async (e) => {
  e.preventDefault();
  hideBanner();
  const email = document.getElementById("re-email").value;
  const password = document.getElementById("re-password").value;
  const passwordConfirm = document.getElementById("re-password2").value;
  try {
    await users.create({ email, password, passwordConfirm });
    await users.authWithPassword(email, password);
  } catch (err) {
    showBanner(err && err.message ? err.message : "Registration failed.");
  }
});

document.getElementById("btn-logout").addEventListener("click", () => {
  cb.authStore.clear();
});

cb.authStore.onChange(() => {
  if (cb.authStore.isValid) {
    enterApp();
  } else {
    viewGuest.style.display = "block";
    viewApp.classList.remove("active");
    whoamiEl.style.display = "none";
    setStatus("idle", "not connected");
  }
}, true);

// --- Realtime: subscribe once to the LLM gateway's SSE frames ----------
//
// `llm_chunk`/`llm_done`/`llm_error` are not collection topics — they are
// the three realtime event names `crates/server/src/routes/llm.rs` sends
// over the caller's *existing* `GET /api/realtime` connection (see that
// file's module doc). `pb.realtime.subscribe` accepts any topic string,
// not just `<collection>/*`, which is what makes this possible with the
// stock SDK and no `@cratebase/extras` import.

let realtimeReady = null;
let pendingChunks = "";

function ensureRealtime() {
  if (realtimeReady) return realtimeReady;
  realtimeReady = Promise.all([
    cb.realtime.subscribe("llm_chunk", (data) => {
      pendingChunks += data.delta;
      renderStreamingAnswer();
    }),
    cb.realtime.subscribe("llm_done", () => {
      /* finalization happens once the POST /docmind/ask promise settles */
    }),
    cb.realtime.subscribe("llm_error", (data) => {
      showBanner("LLM gateway error: " + data.message);
    }),
  ]).then(() => {
    setStatus("connected", "realtime connected");
    return cb.realtime.clientId;
  });
  return realtimeReady;
}

function renderStreamingAnswer() {
  answerEl.textContent = pendingChunks;
  const cursor = document.createElement("span");
  cursor.className = "cursor";
  answerEl.appendChild(cursor);
  answerEl.scrollTop = answerEl.scrollHeight;
}

// --- Docs: upload + list -------------------------------------------------

function renderDoc(record) {
  const li = document.createElement("li");
  const created = new Date(record.created).toLocaleString();
  li.innerHTML = `<div>${escapeHtml(record.title)}</div><div class="meta">${created}${record.file ? " · " + escapeHtml(record.file) : ""}</div>`;
  return li;
}

function escapeHtml(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

async function loadDocs() {
  const page = await docs.getList(1, 50, { sort: "-created" });
  docListEl.innerHTML = "";
  docsEmptyEl.style.display = page.items.length ? "none" : "block";
  for (const record of page.items) docListEl.appendChild(renderDoc(record));
}

uploadForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  hideBanner();
  const file = docFileInput.files[0];
  if (!file) return;
  const formData = new FormData();
  formData.append("title", docTitleInput.value);
  formData.append("file", file);
  uploadButton.disabled = true;
  uploadButton.textContent = "Uploading…";
  try {
    await docs.create(formData);
    docTitleInput.value = "";
    docFileInput.value = "";
    await loadDocs();
  } catch (err) {
    showBanner(err && err.message ? err.message : "Upload failed.");
  } finally {
    uploadButton.disabled = false;
    uploadButton.textContent = "Upload & index";
  }
});

// --- Chat: ask -> retrieval-augmented answer, streamed -------------------

function renderSources(sources) {
  sourcesEl.innerHTML = "";
  for (let i = 0; i < sources.length; i++) {
    const li = document.createElement("li");
    const preview = sources[i].text.length > 160 ? sources[i].text.slice(0, 160) + "…" : sources[i].text;
    li.innerHTML = `<b>[${i + 1}]</b> ${escapeHtml(preview)}`;
    sourcesEl.appendChild(li);
  }
}

function renderHistoryTurn(prompt, response) {
  const div = document.createElement("div");
  div.className = "turn";
  div.innerHTML = `<div class="q">${escapeHtml(prompt)}</div><div class="a">${escapeHtml(response)}</div>`;
  historyEl.prepend(div);
}

async function loadHistory() {
  try {
    const page = await messages.getList(1, 20, { sort: "-created" });
    historyEl.innerHTML = "";
    for (const record of page.items.slice().reverse()) renderHistoryTurn(record.prompt, record.response);
  } catch {
    // messages' listRule requires @request.auth.id = author; a brand new
    // account with zero conversations still gets an empty (not denied)
    // list, so this only fires on a genuine network/auth problem.
  }
}

askForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  hideBanner();
  const question = questionInput.value.trim();
  if (!question) return;

  askButton.disabled = true;
  askButton.textContent = "Thinking…";
  pendingChunks = "";
  answerEl.textContent = "";
  sourcesEl.innerHTML = "";

  try {
    const clientId = await ensureRealtime();
    const result = await cb.send("/docmind/ask", { method: "POST", body: { question, clientId } });
    answerEl.textContent = result.reply;
    renderSources(result.sources || []);
    renderHistoryTurn(question, result.reply);
    questionInput.value = "";
  } catch (err) {
    answerEl.innerHTML = '<span class="placeholder">Nothing yet.</span>';
    showBanner(err && err.message ? err.message : "Ask failed.");
  } finally {
    askButton.disabled = false;
    askButton.textContent = "Ask";
  }
});

// --- Boot -----------------------------------------------------------------

async function enterApp() {
  viewGuest.style.display = "none";
  viewApp.classList.add("active");
  whoamiEl.style.display = "flex";
  whoamiEmailEl.textContent = cb.authStore.record?.email ?? "";
  setStatus("idle", "connecting…");
  try {
    await Promise.all([loadDocs(), loadHistory(), ensureRealtime()]);
  } catch (err) {
    setStatus("error", "connection failed");
    showBanner(err && err.message ? err.message : "Failed to load DocMind.");
  }
}
