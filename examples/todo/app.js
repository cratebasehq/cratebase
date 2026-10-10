// Cratebase Todo example — the comprehensive flow: create an account, sign
// in, then manage a shared realtime todo list. Everything after "sign in"
// requires an authenticated session (`todos`' rules are `@request.auth.id
// != ""`), so this one page exercises both the auth API and record CRUD +
// realtime end to end.
//
// Imported via the bare specifier "@cratebase/client", resolved by the
// import map in index.html to the browser bundle `examples/serve.sh`
// builds into examples/lib/ — no npm install in the example itself.
// See README.md.
import { createClient } from "@cratebase/client";

const BASE_URL = "http://localhost:8090";
const COLLECTION = "todos";
const REMOVE_ANIMATION_MS = 220;

// The ticker below wraps `window.fetch` *after* the client exists, so hand
// the client a thunk that resolves `window.fetch` per request instead of
// the construction-time binding.
const cb = createClient(BASE_URL, { fetch: (input, init) => window.fetch(input, init) });
const users = cb.auth;
const todos = cb.collection(COLLECTION);

// ---------------------------------------------------------------------
// Request ticker — the page's signature element. Every API call this demo
// makes flashes here (method, path, status), so signing in and adding a
// todo are never a black box: you see the literal request each action
// sends. Purely observational — wraps fetch, never touches request/
// response bodies.
// ---------------------------------------------------------------------

const tickerEl = document.getElementById("tickerText");
let tickerTimer = null;

const nativeFetch = window.fetch.bind(window);
window.fetch = async (input, init) => {
  const url = typeof input === "string" ? input : input.url;
  const isApiCall = url.startsWith(BASE_URL);
  const method = (init && init.method) || "GET";
  const start = performance.now();
  const response = await nativeFetch(input, init);
  if (isApiCall) {
    const path = url.slice(BASE_URL.length).split("?")[0];
    const ms = Math.round(performance.now() - start);
    flashTicker(method, path, response.status, ms);
  }
  return response;
};

function flashTicker(method, path, status, ms) {
  tickerEl.innerHTML = `<span class="m m-${method}">${method}</span> ${path} · ${status} · ${ms}ms`;
  tickerEl.classList.remove("show");
  // restart the CSS transition
  void tickerEl.offsetWidth;
  tickerEl.classList.add("show");
  window.clearTimeout(tickerTimer);
  tickerTimer = window.setTimeout(() => tickerEl.classList.remove("show"), 2600);
}

// ---------------------------------------------------------------------
// DOM refs
// ---------------------------------------------------------------------

const statusEl = document.getElementById("status");
const statusTextEl = document.getElementById("statusText");
const whoamiEl = document.getElementById("whoami");
const whoamiEmailEl = document.getElementById("whoamiEmail");
const bannerEl = document.getElementById("banner");

const viewGuest = document.getElementById("view-guest");
const viewTodos = document.getElementById("view-todos");

const listEl = document.getElementById("list");
const emptyStateEl = document.getElementById("emptyState");
const composerForm = document.getElementById("composer");
const titleInput = document.getElementById("title");
const addButton = document.getElementById("add");

// --- Status banner ------------------------------------------------------

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

function setAuthStatus(text, kind) {
  const el = document.getElementById("status-auth");
  el.textContent = text;
  el.className = "form-status" + (kind ? ` ${kind}` : "") + (text ? "" : " hidden");
}

// --- Empty/loading state --------------------------------------------------

function refreshEmptyState() {
  const hasTodos = !!listEl.querySelector(".todo");
  emptyStateEl.style.display = hasTodos ? "none" : "";
}

function setEmptyState(text) {
  emptyStateEl.textContent = text;
}

// ---------------------------------------------------------------------
// Auth: tab switching + register/login forms
// ---------------------------------------------------------------------

const tabButtons = document.querySelectorAll(".tab");
const formLogin = document.getElementById("form-login");
const formRegister = document.getElementById("form-register");

for (const tab of tabButtons) {
  tab.addEventListener("click", () => {
    for (const t of tabButtons) t.classList.toggle("active", t === tab);
    const isLogin = tab.dataset.tab === "login";
    formLogin.classList.toggle("hidden", !isLogin);
    formRegister.classList.toggle("hidden", isLogin);
    setAuthStatus("");
  });
}

async function handleAuthForm(form, action) {
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    setAuthStatus("");
    const data = Object.fromEntries(new FormData(form).entries());
    const submitButton = form.querySelector('button[type="submit"]');
    submitButton.disabled = true;
    try {
      await action(data);
      form.reset();
    } catch (err) {
      setAuthStatus(describeError(err), "err");
    } finally {
      submitButton.disabled = false;
    }
  });
}

function describeError(err) {
  if (err && err.response && err.response.data) {
    const fieldErrors = Object.entries(err.response.data)
      .map(([field, info]) => `${field}: ${(info && info.message) || "invalid"}`)
      .join("; ");
    if (fieldErrors) return fieldErrors;
  }
  if (err && err.response && err.response.message) return err.response.message;
  return err && err.message ? err.message : "Something went wrong.";
}

handleAuthForm(formLogin, async (data) => {
  await users.signIn.password({ identity: data.identity, password: data.password });
});

handleAuthForm(formRegister, async (data) => {
  await users.signUp({ email: data.email, password: data.password, passwordConfirm: data.passwordConfirm });
});

document.getElementById("btn-logout").addEventListener("click", () => {
  cb.realtime.stop();
  cb.auth.signOut();
});

// ---------------------------------------------------------------------
// Todo rendering
// ---------------------------------------------------------------------

function applyDone(el, done) {
  el.classList.toggle("done", done);
  el.querySelector(".checkbox").checked = done;
}

function renderTodo(record, { prepend = false } = {}) {
  const existing = listEl.querySelector(`[data-id="${record.id}"]`);
  if (existing) {
    applyDone(existing, !!record.done);
    existing.querySelector(".title").textContent = record.title;
    return existing;
  }

  const item = document.createElement("li");
  item.className = "todo";
  item.dataset.id = record.id;

  const label = document.createElement("label");
  label.className = "checkbox-wrap";

  const checkbox = document.createElement("input");
  checkbox.type = "checkbox";
  checkbox.className = "checkbox";
  checkbox.checked = !!record.done;
  checkbox.addEventListener("change", () => toggleDone(record.id, checkbox.checked, item));

  const mark = document.createElement("span");
  mark.className = "mark";

  label.append(checkbox, mark);

  const title = document.createElement("span");
  title.className = "title";
  title.textContent = record.title;

  const deleteButton = document.createElement("button");
  deleteButton.type = "button";
  deleteButton.className = "delete";
  deleteButton.setAttribute("aria-label", "Delete todo");
  deleteButton.textContent = "\u00d7";
  deleteButton.addEventListener("click", () => removeTodo(record.id, item));

  item.append(label, title, deleteButton);
  applyDone(item, !!record.done);

  if (prepend && listEl.firstChild) {
    listEl.insertBefore(item, listEl.firstChild);
  } else {
    listEl.appendChild(item);
  }

  refreshEmptyState();
  return item;
}

function removeTodoElement(id) {
  const el = listEl.querySelector(`[data-id="${id}"]`);
  if (!el || el.classList.contains("removing")) return;
  el.classList.add("removing");
  window.setTimeout(() => {
    el.remove();
    refreshEmptyState();
  }, REMOVE_ANIMATION_MS);
}

// --- Actions --------------------------------------------------------------

async function toggleDone(id, done, item) {
  applyDone(item, done); // optimistic — instant feedback in this tab
  try {
    await todos.update(id, { done });
    hideBanner();
  } catch (err) {
    console.error("failed to update todo", err);
    applyDone(item, !done); // revert
    showBanner(`Failed to update todo: ${describeError(err)}.`);
  }
}

async function removeTodo(id, item) {
  item.classList.add("removing");
  try {
    await todos.delete(id);
    hideBanner();
    window.setTimeout(() => {
      item.remove();
      refreshEmptyState();
    }, REMOVE_ANIMATION_MS);
  } catch (err) {
    console.error("failed to delete todo", err);
    item.classList.remove("removing");
    showBanner(`Failed to delete todo: ${describeError(err)}.`);
  }
}

composerForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const title = titleInput.value.trim();
  if (!title) return;

  addButton.disabled = true;
  try {
    const record = await todos.create({ title, done: false });
    renderTodo(record, { prepend: true });
    titleInput.value = "";
    titleInput.focus();
    hideBanner();
  } catch (err) {
    console.error("failed to create todo", err);
    showBanner(`Failed to add todo: ${describeError(err)}.`);
  } finally {
    addButton.disabled = false;
  }
});

// --- Load + realtime --------------------------------------------------------

async function loadTodos() {
  setEmptyState("Loading…");
  const result = await todos.list({ page: 1, perPage: 200, sort: "-created" });
  listEl.innerHTML = "";
  for (const record of result.items) renderTodo(record);
  setEmptyState("No todos yet — add one above.");
  refreshEmptyState();
}

async function connectRealtime() {
  setStatus("connecting", "connecting…");
  try {
    await cb.collection(COLLECTION).subscribe("*", (event) => {
      if (event.action === "create") {
        renderTodo(event.record, { prepend: true });
      } else if (event.action === "update") {
        renderTodo(event.record);
      } else if (event.action === "delete") {
        removeTodoElement(event.record.id);
      }
    });
    setStatus("connected", "live");
    hideBanner();
  } catch (err) {
    console.error("realtime subscribe failed", err);
    setStatus("error", "disconnected");
    showBanner("Realtime connection failed — is `cratebase serve` running on localhost:8090? See README.md.");
  }
}

async function bootTodos() {
  try {
    await loadTodos();
    hideBanner();
  } catch (err) {
    console.error("failed to load todos", err);
    setEmptyState("");
    showBanner(`Failed to load todos: ${describeError(err)}.`);
  }
  await connectRealtime();
}

// ---------------------------------------------------------------------
// Auth state -> view switching
// ---------------------------------------------------------------------

function renderAuthState() {
  const authed = cb.auth.isValid;
  viewGuest.classList.toggle("hidden", authed);
  viewTodos.classList.toggle("hidden", !authed);
  whoamiEl.classList.toggle("hidden", !authed);

  if (authed) {
    whoamiEmailEl.textContent = cb.auth.record?.email || "";
  } else {
    setStatus("disconnected", "signed out");
    listEl.innerHTML = "";
    setEmptyState("Loading…");
  }
}

let wasAuthed = false;
cb.auth.onChange(() => {
  renderAuthState();
  const authed = cb.auth.isValid;
  if (authed && !wasAuthed) {
    bootTodos();
  } else if (!authed && wasAuthed) {
    cb.realtime.stop();
  }
  wasAuthed = authed;
});

// --- Boot -------------------------------------------------------------------

renderAuthState();
wasAuthed = cb.auth.isValid;
if (wasAuthed) bootTodos();
