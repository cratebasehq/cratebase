// Cratebase Todo example.
//
// Imported via the bare specifier "cratebase", resolved by the import map
// in index.html to the built local package (sdk/js/dist) — no npm publish
// or build step needed. See README.md.
import { Cratebase } from "cratebase";

const BASE_URL = "http://localhost:8090";
const COLLECTION = "todos";
const REMOVE_ANIMATION_MS = 220;

const cb = new Cratebase(BASE_URL);

const statusEl = document.getElementById("status");
const statusTextEl = document.getElementById("statusText");
const bannerEl = document.getElementById("banner");
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

// --- Empty/loading state --------------------------------------------------

function refreshEmptyState() {
  const hasTodos = !!listEl.querySelector(".todo");
  emptyStateEl.style.display = hasTodos ? "none" : "";
}

function setEmptyState(text) {
  emptyStateEl.textContent = text;
}

// --- Todo rendering ---------------------------------------------------------

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
    await cb.collection(COLLECTION).update(id, { done });
    hideBanner();
  } catch (err) {
    console.error("failed to update todo", err);
    applyDone(item, !done); // revert
    showBanner(`Failed to update todo: ${err && err.message ? err.message : "unknown error"}.`);
  }
}

async function removeTodo(id, item) {
  item.classList.add("removing");
  try {
    await cb.collection(COLLECTION).delete(id);
    hideBanner();
    window.setTimeout(() => {
      item.remove();
      refreshEmptyState();
    }, REMOVE_ANIMATION_MS);
  } catch (err) {
    console.error("failed to delete todo", err);
    item.classList.remove("removing");
    showBanner(`Failed to delete todo: ${err && err.message ? err.message : "unknown error"}.`);
  }
}

composerForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const title = titleInput.value.trim();
  if (!title) return;

  addButton.disabled = true;
  try {
    const record = await cb.collection(COLLECTION).create({ title, done: false });
    renderTodo(record, { prepend: true });
    titleInput.value = "";
    titleInput.focus();
    hideBanner();
  } catch (err) {
    console.error("failed to create todo", err);
    showBanner(
      `Failed to add todo: ${err && err.message ? err.message : "unknown error"}. ` +
        "Make sure the 'todos' collection exists (see README.md) and cratebase is running.",
    );
  } finally {
    addButton.disabled = false;
  }
});

// --- Initial load -----------------------------------------------------------

async function loadTodos() {
  setEmptyState("Loading…");
  const list = await cb.collection(COLLECTION).getList(1, 200, { sort: "-created" });
  listEl.innerHTML = "";
  for (const record of list.items) {
    renderTodo(record);
  }
  setEmptyState("No todos yet — add one above.");
  refreshEmptyState();
}

// --- Realtime subscription ----------------------------------------------------

async function connectRealtime() {
  setStatus("connecting", "connecting…");
  try {
    await cb.realtime.subscribe(COLLECTION, (event) => {
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

// --- Boot -------------------------------------------------------------------

async function main() {
  try {
    await loadTodos();
  } catch (err) {
    console.error("failed to load todos", err);
    setEmptyState("");
    showBanner(
      `Failed to load todos: ${err && err.message ? err.message : "unknown error"}. ` +
        "Make sure `cratebase serve` is running on localhost:8090 and the 'todos' collection exists (see README.md).",
    );
  }
  await connectRealtime();
}

main();
