// Cratebase — realtime multiplayer cursors example.
//
// Uses the official PocketBase JS SDK, imported via the bare specifier
// "pocketbase" (resolved by the import map in index.html to the published
// package on esm.sh — no npm install or build step needed). Cratebase's
// API is byte-compatible with PocketBase v0.23+, so the official client
// works unchanged.
import PocketBase, { ClientResponseError } from "pocketbase";

// Defaults to the Cratebase server's usual dev port. Override with
// `?api=http://host:port` if you're serving this example from somewhere
// else than localhost:8090 (CORS is wide open by default — see README).
const API_BASE = new URL(window.location.href).searchParams.get("api") || "http://localhost:8090";
const COLLECTION = "cursors";

const cb = new PocketBase(API_BASE);
const cursors = cb.collection(COLLECTION);

// ---------------------------------------------------------------------
// Identity: a random clientId + color persisted in localStorage so a
// reload keeps "being" the same cursor instead of spawning a new dot.
// ---------------------------------------------------------------------

const ADJECTIVES = ["Swift", "Neon", "Quiet", "Bold", "Lucky", "Cosmic", "Sunny", "Rapid", "Mellow", "Vivid", "Frosty", "Golden"];
const ANIMALS = ["Otter", "Falcon", "Lynx", "Comet", "Panda", "Heron", "Wolf", "Tiger", "Sparrow", "Fox", "Whale", "Raven"];

function randomId(len = 12) {
  const bytes = new Uint8Array(len);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(36).padStart(2, "0")).join("").slice(0, len);
}

function randomColor() {
  const hue = Math.floor(Math.random() * 360);
  return `hsl(${hue} 85% 65%)`;
}

function randomLabel() {
  const a = ADJECTIVES[Math.floor(Math.random() * ADJECTIVES.length)];
  const b = ANIMALS[Math.floor(Math.random() * ANIMALS.length)];
  return `${a} ${b}`;
}

function loadOrCreateIdentity() {
  let clientId = localStorage.getItem("cursors:clientId");
  let color = localStorage.getItem("cursors:color");
  let label = localStorage.getItem("cursors:label");
  if (!clientId || !color || !label) {
    clientId = randomId();
    color = randomColor();
    label = randomLabel();
    localStorage.setItem("cursors:clientId", clientId);
    localStorage.setItem("cursors:color", color);
    localStorage.setItem("cursors:label", label);
  }
  return { clientId, color, label };
}

const identity = loadOrCreateIdentity();
// The id of *our* cursor record, once we know it (create response, or a
// prior session's id remembered across reloads).
let myRecordId = localStorage.getItem("cursors:recordId") || null;

// ---------------------------------------------------------------------
// Upsert our own cursor position via the SDK's RecordService.
// ---------------------------------------------------------------------

/** Upsert our own cursor position: PATCH the remembered record id first,
 * and fall back to creating a fresh record if that id is gone (first
 * visit, or the record expired/was deleted server-side). */
async function upsertCursor(x, y) {
  const payload = { clientId: identity.clientId, x, y, color: identity.color, label: identity.label };

  if (myRecordId) {
    try {
      await cursors.update(myRecordId, payload);
      return;
    } catch (err) {
      if (!(err instanceof ClientResponseError) || err.status !== 404) throw err;
      // Our remembered id is stale — fall through to create.
      myRecordId = null;
      localStorage.removeItem("cursors:recordId");
    }
  }

  const record = await cursors.create(payload);
  myRecordId = record.id;
  localStorage.setItem("cursors:recordId", myRecordId);
}

function deleteOwnCursor() {
  if (!myRecordId) return;
  // Best-effort cleanup on tab close, using raw `fetch` (not the SDK)
  // specifically for its `keepalive` flag, which lets the request survive
  // page unload — `RecordService.delete` doesn't expose that fetch
  // option, and there's no response to handle at this point anyway.
  fetch(`${API_BASE}/api/collections/${COLLECTION}/records/${myRecordId}`, { method: "DELETE", keepalive: true }).catch(() => {});
}

// ---------------------------------------------------------------------
// Realtime subscription via the SDK's RealtimeService.
// ---------------------------------------------------------------------

async function subscribeCursors(onEvent, onStatus) {
  const unsubscribe = await cursors.subscribe("*", onEvent);
  // RecordService.subscribe() resolving means the subscription is live;
  // it doesn't expose a connection-status stream (drops/reconnects happen
  // transparently under the hood), so this pill is "did we ever connect",
  // not a continuously accurate live/offline indicator.
  onStatus(true);
  return unsubscribe;
}

// ---------------------------------------------------------------------
// Throttled mousemove -> upsert. requestAnimationFrame drives rendering
// smoothness; a simple timestamp gate caps outbound writes to ~1 per
// 50ms regardless of how fast the mouse actually moves.
// ---------------------------------------------------------------------

const SEND_INTERVAL_MS = 50;
let pendingPos = null;
let lastSentAt = 0;
let sendInFlight = false;

function scheduleSend(x, y) {
  pendingPos = { x, y };
}

function sendLoop(now) {
  if (pendingPos && !sendInFlight && now - lastSentAt >= SEND_INTERVAL_MS) {
    const { x, y } = pendingPos;
    pendingPos = null;
    lastSentAt = now;
    sendInFlight = true;
    upsertCursor(Math.round(x), Math.round(y))
      .catch((err) => console.error("cursor update failed:", err))
      .finally(() => {
        sendInFlight = false;
      });
  }
  requestAnimationFrame(sendLoop);
}
requestAnimationFrame(sendLoop);

window.addEventListener("mousemove", (event) => {
  scheduleSend(event.clientX, event.clientY);
});

window.addEventListener("beforeunload", deleteOwnCursor);

// ---------------------------------------------------------------------
// Rendering: one absolutely-positioned dot per *other* connected client.
// ---------------------------------------------------------------------

const layer = document.getElementById("cursors-layer");
const statusEl = document.getElementById("status");
const statusText = document.getElementById("status-text");
const countEl = document.getElementById("count");
const hintEl = document.getElementById("hint");

/** clientId -> { el, x, y, color, label, lastSeen } */
const peers = new Map();

function cursorSvg(color) {
  return `<svg width="22" height="26" viewBox="0 0 22 26" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M2 1.5L20 12.5L11.5 14.2L7.2 22.5L2 1.5Z" fill="${color}" stroke="rgba(0,0,0,0.35)" stroke-width="1.2" stroke-linejoin="round"/>
  </svg>`;
}

function ensurePeerEl(clientId, color, label) {
  let peer = peers.get(clientId);
  if (peer) return peer;

  const el = document.createElement("div");
  el.className = "cursor";
  el.innerHTML = `${cursorSvg(color)}<span class="label" style="background:${color}">${escapeHtml(label || "Guest")}</span>`;
  layer.appendChild(el);

  peer = { el, x: -100, y: -100, color, label, lastSeen: Date.now() };
  peers.set(clientId, peer);
  updateCount();
  return peer;
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

function movePeer(clientId, x, y, color, label) {
  const peer = ensurePeerEl(clientId, color, label);
  peer.x = x;
  peer.y = y;
  peer.lastSeen = Date.now();
  peer.el.classList.remove("stale");
  peer.el.style.transform = `translate(${x}px, ${y}px)`;
  if (color && peer.color !== color) {
    peer.color = color;
    peer.el.innerHTML = `${cursorSvg(color)}<span class="label" style="background:${color}">${escapeHtml(label || "Guest")}</span>`;
  }
}

function removePeer(clientId) {
  const peer = peers.get(clientId);
  if (!peer) return;
  peer.el.remove();
  peers.delete(clientId);
  updateCount();
}

function updateCount() {
  countEl.textContent = String(peers.size + 1);
}

// Fade out (then drop) cursors whose owner stopped sending updates —
// covers tabs that close without a clean DELETE (network drop, crash).
const STALE_AFTER_MS = 6000;
const DROP_AFTER_MS = 20000;
setInterval(() => {
  const now = Date.now();
  for (const [clientId, peer] of peers) {
    const age = now - peer.lastSeen;
    if (age > DROP_AFTER_MS) {
      removePeer(clientId);
    } else if (age > STALE_AFTER_MS) {
      peer.el.classList.add("stale");
    }
  }
}, 1000);

function handleRealtimeEvent({ action, record }) {
  if (record.clientId === identity.clientId) return; // never render ourselves

  if (action === "delete") {
    removePeer(record.clientId);
    return;
  }
  movePeer(record.clientId, record.x, record.y, record.color, record.label);
}

function setConnected(connected) {
  statusEl.classList.toggle("live", connected);
  statusText.textContent = connected ? "live" : "connecting…";
}

// ---------------------------------------------------------------------
// Boot.
// ---------------------------------------------------------------------

subscribeCursors(handleRealtimeEvent, setConnected);

window.addEventListener(
  "mousemove",
  () => {
    hintEl.classList.add("fade");
  },
  { once: true },
);

// Seed our own record immediately (off-canvas position) so other clients
// see us the instant we connect, reusing the same create-or-update path
// throttled mousemove uses.
upsertCursor(-100, -100).catch((err) => {
  // Only remaining failure mode once upsertCursor's own 404 fallback runs:
  // a stale-but-still-existing record for this clientId from a previous
  // session where localStorage lost the record id (private browsing,
  // cleared storage) — the unique constraint on `clientId` rejects our
  // create. Recover by looking the record up and reusing it.
  const status = err instanceof ClientResponseError ? err.status : undefined;
  if (status === 400 && !myRecordId) {
    cursors
      .getList(1, 1, { filter: `clientId = "${identity.clientId}"` })
      .then((list) => {
        const existing = list?.items?.[0];
        if (existing) {
          myRecordId = existing.id;
          localStorage.setItem("cursors:recordId", myRecordId);
        }
      })
      .catch((lookupErr) => console.error("failed to recover existing cursor record:", lookupErr));
  } else {
    console.error("failed to seed cursor record:", err);
  }
});
