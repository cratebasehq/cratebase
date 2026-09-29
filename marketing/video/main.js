// Cratebase launch film — render contract: window.seek(t) paints frame t.
// Pure function of time: no CSS transitions, no timers, no state carried
// between frames, no Math.random (see docs/style_guide.md, lib/motion.js).
//
// This is the "stills"/"animatic" pass (course step 07-08): the full
// 66s timeline is wired up end to end with real geometry and real
// captured UI, but per-shot visual polish (exact crop framing, the
// team-board auth-card wipe timing, brace geometry on the crate, the
// Superpowers split-panel) is deliberately left rougher here — that's
// the "polish" gate's job, on a later pass. See docs/critique.md.

import { spring, track, Springs, clamp, wipeWindow, mulberry32, isoProject, ISO_EDGE_ANGLE } from "./lib/motion.js";

const DUR = 66;
const STAGE_W = 1920;
const STAGE_H = 1080;

const $ = (sel) => document.querySelector(sel);
const panel = $("#panel");
const headline = $("#headline");
const sectionTitle = $("#section-title");
const hud = $("#hud");
const bgParallax = $("#bg-parallax");

// ---------------------------------------------------------------------
// Background parallax: a few faint crate silhouettes drifting at a
// constant rate (never spring-eased — a camera drift should read as
// constant velocity, per docs/style_guide.md "Camera language").
// ---------------------------------------------------------------------
const rngBg = mulberry32(20260929);
const BG_CRATES = Array.from({ length: 6 }, () => ({
  x: rngBg() * STAGE_W,
  y: 120 + rngBg() * (STAGE_H - 240),
  s: 40 + rngBg() * 70,
  speed: 4 + rngBg() * 6,
}));
bgParallax.innerHTML = BG_CRATES.map(
  (c, i) => `<svg id="bgc${i}" viewBox="-60 -60 120 120" width="${c.s}" height="${c.s}"><path d="${cubeFacePath("top")}" fill="#eef1f4"/></svg>`,
).join("");

function cubeFacePath(which, U = 60) {
  const H = (U * 5.8) / 11;
  const V = (U * 10.4) / 11;
  const proj = ([x, y, z]) => [(x - y) * U, (x + y) * H - z * V];
  const faces = {
    top: [
      [-0.5, -0.5, 0.5],
      [0.5, -0.5, 0.5],
      [0.5, 0.5, 0.5],
      [-0.5, 0.5, 0.5],
    ],
    left: [
      [-0.5, 0.5, -0.5],
      [0.5, 0.5, -0.5],
      [0.5, 0.5, 0.5],
      [-0.5, 0.5, 0.5],
    ],
    right: [
      [0.5, -0.5, -0.5],
      [0.5, -0.5, 0.5],
      [0.5, 0.5, 0.5],
      [0.5, 0.5, -0.5],
    ],
  };
  const pts = faces[which].map(proj);
  return "M" + pts.map(([x, y]) => `${x.toFixed(2)} ${y.toFixed(2)}`).join(" L") + " Z";
}

function drawBgParallax(t) {
  BG_CRATES.forEach((c, i) => {
    const x = c.x - t * c.speed; // constant velocity, never a spring
    const wrapped = ((x % (STAGE_W + 200)) + STAGE_W + 200) % (STAGE_W + 200) - 100;
    const el = document.getElementById(`bgc${i}`);
    el.style.left = `${wrapped}px`;
    el.style.top = `${c.y}px`;
    el.style.position = "absolute";
  });
}

// ---------------------------------------------------------------------
// The one morphing container's geometry — a single track() call per
// dimension across the whole timeline. See docs/style_guide.md "The one
// morphing container."
// ---------------------------------------------------------------------
const CRATE_SMALL = { x: 1240, y: 520, w: 420, h: 420, r: 28 };
const TERMINAL = { x: 260, y: 170, w: 1400, h: 740, r: 16 };
const DATA_PANEL = { x: 210, y: 130, w: 1500, h: 820, r: 16 };
const AUTH_CONFIG = { x: 210, y: 130, w: 1500, h: 820, r: 16 };
const AUTH_CARD = { x: 660, y: 90, w: 600, h: 880, r: 20 };
const EMAIL_PANEL = { x: 210, y: 130, w: 1500, h: 820, r: 16 };
const SUPER_PANEL = { x: 160, y: 110, w: 1600, h: 860, r: 16 };
const PROOF_PANEL = { x: 260, y: 180, w: 1400, h: 680, r: 16 };
const COMPAT_PANEL = { x: 560, y: 340, w: 800, h: 380, r: 16 };
const CRATE_END = { x: 300, y: 480, w: 380, h: 380, r: 28 };

const GEOM_KEYS = [
  [1.9, CRATE_SMALL],
  [4.0, TERMINAL],
  [10.0, DATA_PANEL],
  [20.0, AUTH_CONFIG],
  [22.5, AUTH_CARD],
  [28.0, EMAIL_PANEL],
  [36.0, SUPER_PANEL],
  [50.0, PROOF_PANEL],
  [56.0, COMPAT_PANEL],
  [58.0, CRATE_END],
];

function panelGeom(t) {
  const dims = ["x", "y", "w", "h", "r"];
  const out = {};
  for (const d of dims) {
    out[d] = track(
      t,
      GEOM_KEYS.map(([time, v]) => [time, v[d]]),
      Springs.default.k,
      Springs.default.d,
    );
  }
  return out;
}

// ---------------------------------------------------------------------
// Crate build-in: a 0->1 progress per face, staggered (Heavy spring, no
// overshoot by construction). Used at the hook (build) and reused in
// reverse visually by just holding progress=1 for the rest of the film
// (the crate never disassembles on screen again).
// ---------------------------------------------------------------------
function crateBuildProgress(tLocal, faceIndex) {
  return spring(tLocal - faceIndex * 0.09, Springs.heavy.k, Springs.heavy.d);
}

function renderCrate(t, tSlotStart) {
  const svg = panel.querySelector('[data-layer="crate"] svg');
  const tLocal = t - tSlotStart;
  const faces = ["top", "left", "right"];
  svg.innerHTML = faces
    .map((f, i) => {
      const s = clamp(crateBuildProgress(tLocal, i));
      const fill = f === "top" ? "#ef7d3e" : f === "left" ? "#0e2238" : "#16324d";
      return `<g transform="scale(${s.toFixed(3)})"><path d="${cubeFacePath(f, 180)}" fill="${fill}" /></g>`;
    })
    .join("");
}

// ---------------------------------------------------------------------
// Terminal typing — deterministic reveal: characters shown = floor(rate * t).
// ---------------------------------------------------------------------
const TERM_LINES = [
  { at: 0.0, rate: 42, text: "$ curl -fsSL https://cratebase.dev/install.sh | sh" },
  { at: 1.3, rate: 70, text: "cratebase 0.4.0 installed to ~/.local/bin/cratebase" },
  { at: 2.6, rate: 42, text: "$ cratebase dev --dir demo" },
  { at: 3.6, rate: 90, text: "cratebase dev" },
  { at: 3.9, rate: 90, text: "API:        http://127.0.0.1:8090/api/" },
  { at: 4.1, rate: 90, text: "Dashboard:  http://127.0.0.1:8090/_/" },
  { at: 4.3, rate: 90, text: "Types:      ./cratebase-types.d.ts" },
  { at: 4.5, rate: 90, text: "Mail inbox: http://127.0.0.1:8090/api/dev/mails" },
];
function renderTerminal(t, tSlotStart) {
  const tLocal = t - tSlotStart;
  const out = TERM_LINES.map((line) => {
    const n = Math.max(0, Math.floor((tLocal - line.at) * line.rate));
    return line.text.slice(0, n);
  }).join("\n");
  $("#term-text").textContent = out;
}

// ---------------------------------------------------------------------
// Real captured screenshots. Paths are relative to this file, resolved
// against capture/shots/ — regenerate with capture/seed.sh + capture.mjs
// (README.md) before rendering; the directory is gitignored.
// ---------------------------------------------------------------------
const SHOT = (name) => `capture/shots/${name}`;

// ---------------------------------------------------------------------
// Per-frame content layer selection.
// ---------------------------------------------------------------------
function setActiveLayer(name) {
  panel.querySelectorAll(".layer").forEach((el) => el.classList.toggle("active", el.dataset.layer === name));
}

function showShot(slot, src) {
  const img = slot === "a" ? $("#shot-a-img") : $("#shot-b-img");
  if (img.dataset.src !== src) {
    img.src = src;
    img.dataset.src = src;
  }
}

function renderProof(t, tLocal) {
  const el = $("#layer-proof");
  const barW = 140;
  const gap = 90;
  const baseY = 560;
  const cbTarget = 320;
  const pbTarget = 30;
  const cbH = track(tLocal, [[0, 0], [0.8, cbTarget]], Springs.default.k, Springs.default.d);
  const pbH = track(tLocal, [[0, 0], [0.8, pbTarget]], Springs.default.k, Springs.default.d);
  const ratio = track(tLocal, [[0, 1], [1.2, 10.47]], Springs.default.k, Springs.default.d);
  el.innerHTML = `
    <div class="bench-bar crate-color" style="left:520px;width:${barW}px;height:${Math.max(0, cbH)}px;bottom:120px"></div>
    <div class="bench-bar" style="left:${520 + barW + gap}px;width:${barW}px;height:${Math.max(0, pbH)}px;bottom:120px;opacity:.5"></div>
    <div class="bench-label" style="left:520px;bottom:80px">Cratebase</div>
    <div class="bench-label" style="left:${520 + barW + gap}px;bottom:80px">PocketBase</div>
    <div class="bench-number" style="left:900px;top:80px">${ratio.toFixed(2)}x</div>
    <div class="bench-label" style="left:900px;top:210px">faster reads at concurrency 50</div>
    <div class="bench-label" style="left:60px;bottom:40px;font-size:18px">benchmarks/README.md, 2026-09-04 idle-host run</div>
  `;
}

function renderCompat(tLocal) {
  const el = $("#layer-compat");
  const x = track(tLocal, [[0, 0], [0.6, 1]], Springs.default.k, Springs.default.d);
  el.innerHTML = `
    <div class="toggle-pill ${x < 0.5 ? "on" : ""}" style="left:120px;top:150px;width:220px;height:64px">SQLite</div>
    <div class="toggle-pill ${x >= 0.5 ? "on" : ""}" style="left:380px;top:150px;width:220px;height:64px">Postgres</div>
    <div class="bench-label" style="left:120px;top:250px">Same API, same filter syntax, same rules either way.</div>
    <div class="bench-label" style="left:120px;top:290px">PocketBase-wire-compatible.</div>
  `;
}

function renderTotp(tLocal) {
  const el = $("#layer-totp");
  const s = spring(tLocal, Springs.default.k, Springs.default.d);
  el.innerHTML = `
    <div class="qr" style="left:60px;top:60px;width:${180 * s}px;height:${180 * s}px;opacity:${s}"></div>
    ${[0, 1, 2, 3, 4, 5]
      .map((i) => `<div class="totp-box" style="left:${300 + i * 76}px;top:${120 - (1 - s) * 20}px;opacity:${s}">${i < 3 ? "•" : ""}</div>`)
      .join("")}
    <div class="bench-label" style="left:60px;top:270px;opacity:${s}">Scan, then enter the 6-digit code.</div>
  `;
}

// Real seeded `places` names (capture/seed.sh) — reused for the Search
// and Nearby recreations so they show real data, not placeholder text.
const PLACES = [
  { name: "Kopi Kenangan Senopati", km: 0.4 },
  { name: "Blok M Co-working Loft", km: 0.9 },
  { name: "Pasar Santa Vinyl Corner", km: 1.6 },
  { name: "Warung Tekko", km: 2.3 },
];

function groundGrid(w, h, step = 80) {
  let d = "";
  for (let x = 0; x <= w; x += step) d += `M${x} 0 L${x} ${h} `;
  for (let y = 0; y <= h; y += step) d += `M0 ${y} L${w} ${y} `;
  return `<svg width="${w}" height="${h}" style="position:absolute;inset:0;opacity:.08"><path d="${d}" stroke="var(--harbor-ink)" stroke-width="1" fill="none"/></svg>`;
}

function renderMap(tLocal) {
  const el = $("#layer-map");
  const positions = [
    [220, 220],
    [460, 360],
    [700, 200],
    [340, 520],
  ];
  const pins = positions
    .map(([x, y], i) => {
      const s = spring(tLocal - i * 0.14, Springs.chromeSnap.k, Springs.chromeSnap.d);
      const p = PLACES[i];
      return `
        <div class="pin" style="left:${x}px;top:${y}px;transform:scale(${clamp(s, 0, 1.3).toFixed(2)})"></div>
        <div class="cursor-label" style="left:${x + 22}px;top:${y - 8}px;opacity:${clamp(s)};font-size:22px">${p.name} · ${p.km.toFixed(1)}km</div>
      `;
    })
    .join("");
  el.innerHTML = `
    ${groundGrid(1600, 860)}
    <div class="bench-label" style="left:60px;top:40px;font-size:26px;color:var(--harbor-ink)">cb.rpc("nearest_places", { lat, lng })</div>
    <div class="bench-label" style="left:60px;top:76px">PostGIS-accelerated · nearest first</div>
    ${pins}
  `;
}

// Search re-rank: a screenshot can't show reordering, so this is a
// grounded recreation (real seeded place names) instead of a static
// capture — see docs/critique.md round 1, problem #1.
function renderSearch(tLocal) {
  const el = document.getElementById("layer-search");
  const before = [0, 1, 2, 3];
  const after = [2, 0, 3, 1]; // "coffee" ranks Kopi Kenangan + Blok M highest
  const switchAt = 1.1;
  const rowH = 96;
  const rows = PLACES.map((p, i) => {
    const orderKey = (rank) => 40 + rank * rowH;
    const y = track(
      tLocal,
      [
        [0, orderKey(before.indexOf(i))],
        [switchAt, orderKey(after.indexOf(i))],
      ],
      Springs.default.k,
      Springs.default.d,
    );
    const rank = after.indexOf(i);
    return `
      <div style="position:absolute;left:40px;top:${y}px;width:1520px;height:76px;display:flex;align-items:center;gap:20px;border-bottom:1px solid var(--harbor-hairline)">
        <span style="font-family:var(--font-mono);color:var(--harbor-ink-secondary);width:32px">${rank + 1}</span>
        <span style="font-size:30px">${p.name}</span>
        <span class="bench-label" style="position:static;margin-left:auto">match: description</span>
      </div>
    `;
  }).join("");
  el.innerHTML = `
    <div class="bench-label" style="left:40px;top:0;font-size:26px;color:var(--harbor-ink)">?search=coffee</div>
    <div style="position:absolute;inset:0;top:40px">${rows}</div>
  `;
}

function renderPresence(t, tLocal) {
  const el = $("#layer-presence");
  const r1 = mulberry32(11);
  const r2 = mulberry32(29);
  const path = (rng) => Array.from({ length: 6 }, () => [rng() * 1500 + 60, rng() * 700 + 60]);
  const p1 = path(mulberry32(11));
  const p2 = path(mulberry32(29));
  const pos = (pts, tl) => {
    const keys = pts.map((p, i) => [i * 1.0, p]);
    const x = track(tl % (pts.length - 1 + 1e-6), keys.map(([tm, p]) => [tm, p[0]]), Springs.default.k, Springs.default.d);
    const y = track(tl % (pts.length - 1 + 1e-6), keys.map(([tm, p]) => [tm, p[1]]), Springs.default.k, Springs.default.d);
    return [x, y];
  };
  const [x1, y1] = pos(p1, tLocal);
  const [x2, y2] = pos(p2, tLocal);
  el.innerHTML = `
    ${groundGrid(1600, 860)}
    <div class="bench-label" style="left:60px;top:40px;font-size:26px;color:var(--harbor-ink)">usePresence("places")</div>
    <div class="bench-label" style="left:60px;top:76px">Every visitor's cursor, live, over realtime.</div>
    <div class="cursor-dot" style="left:${x1}px;top:${y1}px"></div>
    <div class="cursor-label" style="left:${x1 + 18}px;top:${y1 - 22}px;font-size:22px">Nico</div>
    <div class="cursor-dot" style="left:${x2}px;top:${y2}px;background:var(--harbor-ink)"></div>
    <div class="cursor-label" style="left:${x2 + 18}px;top:${y2 - 22}px;font-size:22px">Amara</div>
  `;
}

function renderNotif(tLocal) {
  const el = $("#layer-notif");
  const items = [
    { at: 0.1, text: "Amara commented on \"Draft: nearest-place notes\"" },
    { at: 0.7, text: "New sign-in from a new location" },
  ];
  const unread = items.filter((it) => tLocal >= it.at && tLocal < 1.6).length;
  const s = spring(tLocal, Springs.chromeSnap.k, Springs.chromeSnap.d);
  const rows = items
    .map((it, i) => {
      const rowS = clamp(spring(tLocal - it.at, Springs.default.k, Springs.default.d));
      return `<div class="chat-bubble" style="left:${560 + (1 - rowS) * 30}px;top:${200 + i * 90}px;opacity:${rowS};width:820px">${it.text}</div>`;
    })
    .join("");
  el.innerHTML = `
    <div class="bench-label" style="left:60px;top:40px;font-size:26px;color:var(--harbor-ink)">cb.notifications.list()</div>
    <div class="bell-badge" style="left:${560 + 30 * s}px;top:120px;width:40px;height:40px;opacity:${unread ? 1 : 0}">${unread}</div>
    ${rows}
    <div class="bench-label" style="left:560px;top:${200 + items.length * 90 + 10}px">${unread ? "" : "All caught up."}</div>
  `;
}

function renderChannel(tLocal) {
  const el = $("#layer-channel");
  const lines = [
    { at: 0.0, who: "Amara", text: "on my way" },
    { at: 0.6, who: "Nico", text: "see you in 5" },
  ];
  const rows = lines
    .map((l, i) => {
      const s = clamp(spring(tLocal - l.at, Springs.default.k, Springs.default.d));
      return `<div class="chat-bubble" style="left:${560 + (1 - s) * 40}px;top:${240 + i * 100}px;opacity:${s};width:600px"><strong>${l.who}:</strong> ${l.text}</div>`;
    })
    .join("");
  el.innerHTML = `
    <div class="bench-label" style="left:60px;top:40px;font-size:26px;color:var(--harbor-ink)">cb.channel("room:demo")</div>
    <div class="bench-label" style="left:60px;top:76px">Chat, cursors, or a live counter — one channel API.</div>
    ${rows}
  `;
}

// ---------------------------------------------------------------------
// Headline / section-title / HUD text — clip-path wipe reveal only,
// never opacity fades (docs/style_guide.md "Text enter"/"Text exit").
// ---------------------------------------------------------------------
function wipeClip(progress) {
  return `inset(0 ${(1 - clamp(progress)) * 100}% 0 0)`;
}

function renderHeadline(t) {
  if (t < 4.2) {
    const p1 = wipeWindow(t, 0.0, 66, 0.5, 0.1);
    const p2 = wipeWindow(t, 0.5, 66, 0.5, 0.1);
    headline.style.display = "block";
    headline.style.left = "120px";
    headline.style.top = "660px";
    headline.style.fontSize = "108px";
    headline.innerHTML = `
      <div class="line" style="top:0;clip-path:${wipeClip(p1)}">Stop rebuilding</div>
      <div class="line" style="top:116px;clip-path:${wipeClip(p2)}">your backend.</div>
    `;
  } else if (t >= 61.0) {
    const p = wipeWindow(t, 61.0, 68, 0.5, 0.1);
    headline.style.display = "block";
    headline.style.left = "760px";
    headline.style.top = "300px";
    headline.style.fontSize = "68px";
    const linkP = wipeWindow(t, 64.0, 68, 0.4, 0.1);
    headline.innerHTML = `
      <div class="line" style="top:0;width:1100px;clip-path:${wipeClip(p)}">The backend you actually ship with.</div>
      <div class="line" style="top:140px;font-family:var(--font-mono);font-size:32px;font-weight:400;clip-path:${wipeClip(linkP)}">cratebase.dev</div>
      <div class="line" style="top:190px;font-family:var(--font-mono);font-size:32px;font-weight:400;clip-path:${wipeClip(linkP)}">GitHub</div>
      <div class="line" style="top:240px;font-family:var(--font-mono);font-size:32px;font-weight:400;clip-path:${wipeClip(linkP)}">Open in Codespaces</div>
    `;
  } else {
    headline.style.display = "none";
  }
}

const SECTION_TITLES = [
  { text: "Data", tIn: 10.0, tOut: 13.0 },
  { text: "Auth", tIn: 20.0, tOut: 22.8 },
  { text: "Email", tIn: 28.0, tOut: 31.0 },
];
function renderSectionTitle(t) {
  const active = SECTION_TITLES.find((s) => t >= s.tIn && t < s.tOut + 0.6);
  if (!active) {
    sectionTitle.style.display = "none";
    return;
  }
  const p = wipeWindow(t, active.tIn, active.tOut + 0.6, 0.3, 0.2);
  sectionTitle.style.display = "block";
  sectionTitle.style.left = "80px";
  sectionTitle.style.top = "60px";
  sectionTitle.style.clipPath = wipeClip(p);
  sectionTitle.textContent = active.text;
}

const HUD_LABELS = [
  { text: "Search.", tIn: 36.0, tOut: 38.5 },
  { text: "Nearby.", tIn: 38.5, tOut: 41.0 },
  { text: "Presence.", tIn: 41.0, tOut: 44.5 },
  { text: "Notifications.", tIn: 44.5, tOut: 47.5 },
  { text: "Channels.", tIn: 47.5, tOut: 50.0 },
  { text: "One binary.", tIn: 8.0, tOut: 10.0 },
  { text: "Send it from your frontend.", tIn: 31.0, tOut: 33.5 },
];
function renderHud(t) {
  const active = HUD_LABELS.find((s) => t >= s.tIn && t < s.tOut);
  if (!active) {
    hud.style.display = "none";
    return;
  }
  const p = wipeWindow(t, active.tIn, active.tOut, 0.2, 0.15);
  hud.style.display = "block";
  hud.style.clipPath = wipeClip(p);
  hud.textContent = active.text;
}

// ---------------------------------------------------------------------
// The one hard cut in the film: an isometric-angle wipe at 50.0s.
// ---------------------------------------------------------------------
function hardCutClip(t) {
  const cutStart = 49.9;
  const cutDur = 0.18;
  if (t < cutStart || t > cutStart + cutDur) return "none";
  const p = clamp((t - cutStart) / cutDur);
  const angleDeg = (ISO_EDGE_ANGLE * 180) / Math.PI;
  return `polygon(0 0, ${100 * p}% 0, ${100 * p - Math.tan((angleDeg * Math.PI) / 180) * 20}% 100%, 0 100%)`;
}

// ---------------------------------------------------------------------
// Main seek(t)
// ---------------------------------------------------------------------
function seek(t) {
  t = clamp(t, 0, DUR);
  drawBgParallax(t);

  const g = panelGeom(t);
  panel.style.left = `${g.x}px`;
  panel.style.top = `${g.y}px`;
  panel.style.width = `${g.w}px`;
  panel.style.height = `${g.h}px`;
  panel.style.borderRadius = `${g.r}px`;
  // Only the split second the crate build actually starts — not the
  // ~0.9s empty-box gap an earlier cut of this scene left visible before
  // the first face landed (docs/critique.md round 1, problem #1).
  panel.style.opacity = t < 1.9 ? 0 : 1;

  if (t < 4.0) {
    setActiveLayer("crate");
    renderCrate(t, 1.9);
  } else if (t < 10.0) {
    setActiveLayer("terminal");
    renderTerminal(t, 4.0);
  } else if (t < 20.0) {
    setActiveLayer("shot-a");
    showShot("a", SHOT("02-collections-places-records.png"));
    if (t >= 13.0 && t < 16.5) showShot("a", SHOT("04-collections-places-api-docs.png"));
    if (t >= 16.5) showShot("a", SHOT("03-collections-places-schema-rules.png"));
  } else if (t < 22.5) {
    setActiveLayer("shot-a");
    showShot("a", SHOT("06-users-auth-options.png"));
  } else if (t < 25.5) {
    setActiveLayer("shot-a");
    if (t < 23.5) showShot("a", SHOT("12-team-board-auth-password.png"));
    else if (t < 24.5) showShot("a", SHOT("13-team-board-auth-otp.png"));
    else showShot("a", SHOT("14-team-board-auth-magic-link.png"));
  } else if (t < 28.0) {
    setActiveLayer("totp");
    renderTotp(t - 25.5);
  } else if (t < 36.0) {
    setActiveLayer("shot-a");
    if (t < 31.0) showShot("a", SHOT("07-settings-email-templates.png"));
    else if (t < 33.5) showShot("a", SHOT("08-settings-email-template-editor.png"));
    else showShot("a", SHOT("10-settings-mail-inbox.png"));
  } else if (t < 38.5) {
    setActiveLayer("search");
    renderSearch(t - 36.0);
  } else if (t < 41.0) {
    setActiveLayer("map");
    renderMap(t - 38.5);
  } else if (t < 44.5) {
    setActiveLayer("presence");
    renderPresence(t, t - 41.0);
  } else if (t < 47.5) {
    setActiveLayer("notif");
    renderNotif(t - 44.5);
  } else if (t < 50.0) {
    setActiveLayer("channel");
    renderChannel(t - 47.5);
  } else if (t < 56.0) {
    setActiveLayer("proof");
    renderProof(t, t - 50.0);
  } else if (t < 58.0) {
    setActiveLayer("compat");
    renderCompat(t - 56.0);
  } else {
    setActiveLayer("crate");
    renderCrate(t, 1.9); // already fully built; holds at progress 1
  }

  renderHeadline(t);
  renderSectionTitle(t);
  renderHud(t);

  document.body.style.clipPath = "none";
  document.getElementById("stage").style.clipPath = hardCutClip(t);
}

window.seek = seek;

// Live preview in a normal browser; off during headless render
// (navigator.webdriver is true under Playwright).
if (!navigator.webdriver) {
  const t0 = performance.now();
  (function loop() {
    seek(((performance.now() - t0) / 1000) % DUR);
    requestAnimationFrame(loop);
  })();
} else {
  seek(0);
}
