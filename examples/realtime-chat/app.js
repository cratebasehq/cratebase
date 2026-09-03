// Cratebase Realtime Chat example.
//
// Imported via the bare specifier "cratebase", resolved by the import map
// in index.html to the built local package (sdk/js/dist) — no npm publish
// or build step needed. See README.md.
import { Cratebase } from "cratebase";

const BASE_URL = "http://localhost:8090";
const COLLECTION = "messages";
const NAME_STORAGE_KEY = "cratebase-chat-display-name";

const cb = new Cratebase(BASE_URL);

const statusEl = document.getElementById("status");
const statusTextEl = document.getElementById("statusText");
const bannerEl = document.getElementById("banner");
const messagesEl = document.getElementById("messages");
const emptyStateEl = document.getElementById("emptyState");
const nameInput = document.getElementById("displayName");
const composerForm = document.getElementById("composer");
const contentInput = document.getElementById("content");
const sendButton = document.getElementById("send");

// --- Display name, persisted locally (no auth — just a label) -------------

function loadDisplayName() {
  const stored = localStorage.getItem(NAME_STORAGE_KEY);
  if (stored && stored.trim()) return stored;
  const generated = `Guest-${Math.floor(1000 + Math.random() * 9000)}`;
  localStorage.setItem(NAME_STORAGE_KEY, generated);
  return generated;
}

nameInput.value = loadDisplayName();
nameInput.addEventListener("change", () => {
  const trimmed = nameInput.value.trim() || loadDisplayName();
  nameInput.value = trimmed;
  localStorage.setItem(NAME_STORAGE_KEY, trimmed);
});

// --- Status banner ----------------------------------------------------------

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

// --- Message rendering -------------------------------------------------------

function formatTime(isoString) {
  const date = isoString ? new Date(isoString) : new Date();
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function appendMessage(record) {
  emptyStateEl.style.display = "none";

  const isMine = record.author === nameInput.value;

  const wrapper = document.createElement("div");
  wrapper.className = `msg${isMine ? " mine" : ""}`;
  wrapper.dataset.id = record.id;

  const meta = document.createElement("div");
  meta.className = "meta";
  const authorEl = document.createElement("span");
  authorEl.textContent = record.author || "Anonymous";
  const timeEl = document.createElement("span");
  timeEl.textContent = formatTime(record.created);
  meta.append(authorEl, timeEl);

  const bubble = document.createElement("div");
  bubble.className = "bubble";
  bubble.textContent = record.content;

  wrapper.append(meta, bubble);
  messagesEl.appendChild(wrapper);

  const nearBottom = messagesEl.scrollHeight - messagesEl.scrollTop - messagesEl.clientHeight < 200;
  if (nearBottom) {
    messagesEl.scrollTop = messagesEl.scrollHeight;
  }
}

function removeMessage(id) {
  const el = messagesEl.querySelector(`[data-id="${id}"]`);
  if (el) el.remove();
  if (!messagesEl.querySelector(".msg")) {
    emptyStateEl.style.display = "";
  }
}

// --- Initial history load ----------------------------------------------------

async function loadHistory() {
  const list = await cb.collection(COLLECTION).getList(1, 50, { sort: "created" });
  for (const record of list.items) {
    appendMessage(record);
  }
  messagesEl.scrollTop = messagesEl.scrollHeight;
}

// --- Realtime subscription ----------------------------------------------------

async function connectRealtime() {
  setStatus("connecting", "connecting…");
  try {
    await cb.realtime.subscribe(COLLECTION, (event) => {
      if (event.action === "create") {
        appendMessage(event.record);
      } else if (event.action === "delete") {
        removeMessage(event.record.id);
      }
      // "update" is ignored — this example never edits messages.
    });
    setStatus("connected", "live");
    hideBanner();
  } catch (err) {
    console.error("realtime subscribe failed", err);
    setStatus("error", "disconnected");
    showBanner("Realtime connection failed — is `cratebase serve` running on localhost:8090? See README.md.");
  }
}

// --- Sending messages -----------------------------------------------------

composerForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const content = contentInput.value.trim();
  if (!content) return;

  const author = nameInput.value.trim() || "Anonymous";

  sendButton.disabled = true;
  try {
    await cb.collection(COLLECTION).create({ author, content });
    contentInput.value = "";
    contentInput.focus();
    hideBanner();
  } catch (err) {
    console.error("failed to send message", err);
    showBanner(
      `Failed to send: ${err && err.message ? err.message : "unknown error"}. ` +
        "Make sure the 'messages' collection exists (see README.md) and cratebase is running.",
    );
  } finally {
    sendButton.disabled = false;
  }
});

// --- Boot -------------------------------------------------------------------

async function main() {
  try {
    await loadHistory();
  } catch (err) {
    console.error("failed to load history", err);
    showBanner(
      `Failed to load messages: ${err && err.message ? err.message : "unknown error"}. ` +
        "Make sure `cratebase serve` is running on localhost:8090 and the 'messages' collection exists (see README.md).",
    );
  }
  await connectRealtime();
}

main();
