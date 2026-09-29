// Chapter 4 — Auth (22–30 s). docs/shotlist.md §4.
// REBUILT: the sign-in card from examples/team-board/src/components/
// AuthScreen.tsx, same copy, tokens (tailwind.config.js) and fonts (Inter,
// Archivo), rebuilt as DOM so it can morph between states and scale
// crisply under the camera. The TOTP step is a RECREATE from the real API
// (`cb.auth.signIn.totp({ mfaId, code })`, RFC 6238, 6 digits, 30 s) — the
// example ships no TOTP screen. The 12 provider labels are the real
// `KnownProvider` presets, labelled exactly as the example renders them.

import { el, S, place, show, px, heavy, snappy, soft, chrome, prog, statement, drawStatement, cursor, drawCursor, html, text } from "../lib/kit.js";
import { clamp, track, indicator } from "../lib/motion.js";
import { addCue } from "../lib/timeline.js";

const PROVIDERS = ["Google", "GitHub", "Apple", "Microsoft", "Discord", "GitLab", "Facebook", "X", "LinkedIn", "Slack", "Twitch", "Spotify"];
const OTP = "481927";
const TOTP = "305118";
// state machine on the grid: [time, form state]
// form states: pw (password, the real default tab), ml (magic link), mlSent,
// oc (email code), ocCode (code entry), totp, done
const STATES = [
  [0, "pw"], [22.5, "ml"], [23.25, "mlSent"], [24.0, "oc"], [24.5, "ocCode"], [26.0, "totp"], [27.5, "done"],
];
const TAB_OF = { pw: 0, ml: 2, mlSent: 2, oc: 1, ocCode: 1, totp: 1, done: 1 };
const CLICKS = [22.5, 23.25, 24.0, 24.5, 25.875, 27.5];

CLICKS.forEach((t) => addCue(t, "click", {}));
addCue(22.0, "whoosh", { dur: 0.45, gain: 0.6 });
addCue(23.3, "send", {});
[25.0, 25.125, 25.25, 25.375, 25.5, 25.625].forEach((t, i) => addCue(t, "tick", { note: 76 + i }));
addCue(26.0, "whoosh", { dur: 0.3, gain: 0.4 });
[26.5, 26.625, 26.75, 26.875, 27.0, 27.125].forEach((t, i) => addCue(t, "tick", { note: 79 + i }));
addCue(27.5, "confirm", {});
addCue(28.0, "whoosh", { dur: 0.5, gain: 0.7 });
PROVIDERS.forEach((_, i) => addCue(28.125 + i * 0.1, "pop", { note: 62 + [0, 2, 3, 5, 7, 9, 10, 12, 14, 15, 17, 19][i] }));

const cardCSS = `
  .tb { font-family: Inter, system-ui, sans-serif; color: #16181d; width: 360px; }
  .tb h1 { font-family: Archivo, sans-serif; font-weight: 600; font-size: 24px; line-height: 32px; margin: 0; letter-spacing: -0.01em; }
  .tb .lede { margin-top: 8px; font-size: 14px; line-height: 20px; color: rgba(22,24,29,.6); }
  .tb .tabs { position: relative; margin-top: 32px; display: flex; gap: 4px; border-radius: 6px; background: #efebe2; padding: 4px; }
  .tb .tab { position: relative; flex: 1; z-index: 1; border-radius: 4px; padding: 6px 12px; font-size: 14px; line-height: 20px; font-weight: 500; text-align: center; color: rgba(22,24,29,.5); }
  .tb .tab.on { color: #16181d; }
  .tb .ind { position: absolute; top: 4px; bottom: 4px; border-radius: 4px; background: #fff; box-shadow: 0 1px 2px rgba(0,0,0,.08); }
  .tb .formwrap { position: relative; margin-top: 24px; overflow: hidden; }
  .tb .form { position: absolute; left: 0; top: 0; width: 360px; display: flex; flex-direction: column; gap: 12px; }
  .tb label { display: flex; flex-direction: column; gap: 4px; font-size: 14px; }
  .tb label > span { font-weight: 500; color: rgba(22,24,29,.8); }
  .tb .input { border-radius: 6px; border: 1px solid rgba(201,194,180,.7); background: #fff; padding: 8px 12px; font-size: 14px; line-height: 20px; min-height: 38px; }
  .tb .input.focus { border-color: #e8622c; }
  .tb .ph { color: rgba(22,24,29,.35); }
  .tb .btn { border-radius: 6px; background: #e8622c; color: #fff; padding: 8px 16px; font-size: 14px; line-height: 20px; font-weight: 500; text-align: center; margin-top: 4px; }
  .tb .hint { font-size: 12px; line-height: 16px; color: rgba(22,24,29,.5); }
  .tb .link { font-size: 14px; color: #3a5b8c; }
  .tb .copy { font-size: 14px; line-height: 20px; color: rgba(22,24,29,.7); }
  .tb .mono { font-family: JBMono, ui-monospace, monospace; letter-spacing: 0.1em; }
  .tb .boxes { display: flex; gap: 8px; }
  .tb .box { flex: 1; height: 52px; border-radius: 6px; border: 1px solid rgba(201,194,180,.7); background: #fff; display: flex; align-items: center; justify-content: center; font-family: JBMono, monospace; font-size: 24px; font-weight: 600; }
  .tb .box.focus { border-color: #e8622c; box-shadow: 0 0 0 3px rgba(232,98,44,.18); }
  .tb .ok { display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 600; color: #3e8b63; }
  .tb .div { margin-top: 32px; display: flex; align-items: center; gap: 12px; font-size: 12px; color: rgba(22,24,29,.4); }
  .tb .div span { height: 1px; flex: 1; background: rgba(201,194,180,.6); }
  .tb .prov { margin-top: 16px; display: flex; flex-direction: column; gap: 8px; }
  .tb .pbtn { border-radius: 6px; border: 1px solid rgba(201,194,180,.6); padding: 8px 16px; font-size: 14px; line-height: 20px; font-weight: 500; text-align: center; }
`;

function forms() {
  return {
    pw: `<label><span>Email</span><div class="input">alice@example.com</div></label>
         <label><span>Password</span><div class="input ph">At least 8 characters</div></label>
         <div class="btn">Sign in</div>
         <div class="link">New here? Create an account</div>`,
    ml: `<label><span>Email</span><div class="input">alice@example.com</div></label>
         <div class="btn" data-k="send-ml">Send magic link</div>`,
    mlSent: `<div class="copy">If <b style="font-weight:600">alice@example.com</b> has an account, a sign-in link is on its way — in dev, open the dashboard's Mail inbox (Settings → Mail inbox) to read it.</div>
         <div class="link">Use a different email</div>`,
    oc: `<label><span>Email</span><div class="input">alice@example.com</div></label>
         <div class="hint">We'll email a one-time code — in dev, open the dashboard's Mail inbox (Settings → Mail inbox) to read it.</div>
         <div class="btn" data-k="send-oc">Send code</div>`,
    ocCode: `<label><span>Code</span><div class="input mono focus" data-k="code"><span class="ph">123456</span></div></label>
         <div class="btn" data-k="verify">Verify and sign in</div>
         <div class="link">Use a different email</div>`,
    totp: `<label><span>Authenticator code</span></label>
         <div class="boxes">${[0, 1, 2, 3, 4, 5].map((i) => `<div class="box" data-b="${i}"></div>`).join("")}</div>
         <div class="hint" style="display:flex;align-items:center;gap:8px"><svg width="16" height="16" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.5" fill="none" stroke="rgba(201,194,180,.9)" stroke-width="2"/><circle data-k="ring" cx="8" cy="8" r="6.5" fill="none" stroke="#e8622c" stroke-width="2" stroke-dasharray="40.84" stroke-dashoffset="0" transform="rotate(-90 8 8)"/></svg>Enter the 6-digit code from your authenticator app.</div>
         <div class="btn" data-k="verify2">Verify</div>`,
    done: `<label><span>Authenticator code</span></label>
         <div class="boxes">${TOTP.split("").map((d) => `<div class="box">${d}</div>`).join("")}</div>
         <div class="ok"><svg width="18" height="18" viewBox="0 0 18 18"><circle cx="9" cy="9" r="9" fill="#3e8b63"/><path d="M5 9.2l2.6 2.6L13 6.4" stroke="#fff" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"/></svg>Signed in as Alice Chen</div>`,
  };
}

export default [
  {
    id: "auth",
    from: 22,
    to: 30,
    z: 3,
    build(root, L) {
      S(root, { background: "#f6f4ef" });
      el("style", "", root, cardCSS);
      const card = el("div", "abs tb", root);
      S(card, { transformOrigin: "0 0" });
      el("h1", "", card, "Sign in to team-board");
      el("div", "lede", card, "A shared board for your team. Use the demo account, or create your own.");
      const tabs = el("div", "tabs", card);
      const ind = el("div", "ind", tabs);
      const tabEls = ["Password", "Email code", "Magic link"].map((n) => el("div", "tab", tabs, n));
      const wrap = el("div", "formwrap", card);
      const F = forms();
      const layers = {};
      for (const k of Object.keys(F)) {
        layers[k] = el("div", "form", wrap, F[k]);
      }
      const div = el("div", "div", card, "<span></span>or continue with<span></span>");
      const prov = el("div", "prov", card);
      PROVIDERS.slice(0, 5).forEach((p) => el("div", "pbtn", prov, `Continue with ${p}`));
      // measure every form's natural height and the tab geometry (card-local px)
      const heights = {};
      for (const k of Object.keys(layers)) heights[k] = layers[k].scrollHeight;
      const tabGeo = tabEls.map((e) => ({ x: e.offsetLeft, w: e.offsetWidth, y: e.offsetTop }));
      const wrapTop = wrap.offsetTop;
      const tabsTop = tabs.offsetTop;
      // provider wall (28–30): big button-style tiles, same border/weight as the real buttons
      const wall = el("div", "abs", root);
      const tiles = PROVIDERS.map((p) => {
        const t = el("div", "abs nowrap", wall, p);
        S(t, { fontFamily: "Inter, sans-serif", fontWeight: 600, color: "#16181d", background: "#fff", border: "3px solid rgba(201,194,180,.8)", borderRadius: "22px", display: "flex", alignItems: "center", justifyContent: "center", letterSpacing: "-0.02em" });
        return t;
      });
      const stMethod = {
        ml: statement(root, L.portrait ? ["Magic link."] : ["Magic", "link."], "disp"),
        oc: statement(root, L.portrait ? ["Email code."] : ["Email", "code."], "disp"),
        totp: statement(root, L.portrait ? ["TOTP 2FA."] : ["TOTP", "2FA."], "disp"),
      };
      const stWall = statement(root, ["12 OAuth2 presets."], "disp");
      const stWall2 = statement(root, ["plus any OpenID Connect provider."]);
      const cur = cursor(root);
      // local positions of click targets (inside formwrap → card-local)
      const loc = (layer, sel) => {
        const e = layers[layer].querySelector(sel);
        return { x: e.offsetLeft + e.offsetWidth / 2, y: wrapTop + e.offsetTop + e.offsetHeight / 2 };
      };
      const targets = {
        tabMl: { x: tabGeo[2].x + tabGeo[2].w / 2, y: tabsTop + 18 },
        tabOc: { x: tabGeo[1].x + tabGeo[1].w / 2, y: tabsTop + 18 },
        sendMl: loc("ml", '[data-k="send-ml"]'),
        sendOc: loc("oc", '[data-k="send-oc"]'),
        code: loc("ocCode", '[data-k="code"]'),
        verify: loc("ocCode", '[data-k="verify"]'),
        verify2: loc("totp", '[data-k="verify2"]'),
      };
      return { card, ind, tabEls, tabGeo, layers, heights, wrap, div, prov, wall, tiles, stMethod, stWall, stWall2, cur, targets, wrapTop };
    },
    draw(t, s, L) {
      const P = L.portrait;
      // ---- which form is current, and the one before it (for the swap)
      let cur = 0;
      STATES.forEach(([tt], i) => { if (t >= tt) cur = i; });
      const [tNow, kNow] = STATES[cur];
      const kPrev = cur > 0 ? STATES[cur - 1][1] : null;
      const sw = snappy(t - tNow);
      for (const [k, e] of Object.entries(s.layers)) {
        const isNow = k === kNow, isPrev = k === kPrev && sw < 0.999;
        show(e, isNow || isPrev);
        if (isNow) S(e, { transform: `translate3d(0,${((1 - sw) * 40).toFixed(2)}px,0)`, opacity: String(clamp(sw * 1.6)) });
        if (isPrev) S(e, { transform: `translate3d(0,${(-sw * 40).toFixed(2)}px,0)`, opacity: String(clamp(1 - sw * 2)) });
      }
      // form container height morphs on track()
      const h = track(t, STATES.map(([tt, k]) => [tt, s.heights[k]]), 260, 30);
      S(s.wrap, { height: px(h) });
      // tab indicator stretches (lead/trail on different springs)
      const tabKeys = STATES.map(([tt, k]) => [tt, s.tabGeo[TAB_OF[k]].x]);
      const ind = indicator(t, tabKeys);
      const w0 = s.tabGeo[0].w;
      S(s.ind, { left: px(ind.left), width: px(ind.right - ind.left + w0) });
      s.tabEls.forEach((e, i) => e.classList.toggle("on", TAB_OF[kNow] === i));
      // typed OTP digits (25.0–25.75, 8ths) and TOTP boxes (26.5–27.25)
      if (kNow === "ocCode") {
        const n = clamp(Math.floor((t - 25.0) / 0.125) + 1, 0, 6);
        const box = s.layers.ocCode.querySelector('[data-k="code"]');
        html(box, n > 0 ? OTP.slice(0, n) : '<span class="ph">123456</span>');
      }
      if (kNow === "totp") {
        const n = clamp(Math.floor((t - 26.5) / 0.125) + 1, 0, 6);
        s.layers.totp.querySelectorAll(".box").forEach((b, i) => {
          text(b, i < n ? TOTP[i] : "");
          b.classList.toggle("focus", i === n);
        });
        const ring = s.layers.totp.querySelector('[data-k="ring"]');
        ring.setAttribute("stroke-dashoffset", (40.84 * clamp((t - 26.0) / 30 + 0.62)).toFixed(2));
      }
      // ---- camera on the card: scale k, focus on the tabs+form block
      const k = P ? (L.W * 0.9) / 360 : Math.min((L.W * 0.66) / 360, 3.6);
      const exitP = soft(t - 28.0);
      const enter = heavy(t - 22.0);
      const focusY = s.wrapTop - 70 + h / 2; // card-local y to keep centred
      const cx = P ? (L.W - 360 * k) / 2 : L.W * 0.315;
      const cyScreen = P ? L.H * 0.6 : L.H * 0.54;
      const kk = k * (0.82 + 0.18 * enter) * (1 - 0.3 * exitP);
      const x = cx + (1 - enter) * L.W * 0.5 - exitP * (P ? 0 : L.W * 0.25);
      const y = cyScreen - focusY * kk + exitP * L.H * 1.1;
      S(s.card, { transform: `translate3d(${px(x)},${px(y)},0) scale(${kk.toFixed(4)}) rotate(${((1 - enter) * 8).toFixed(2)}deg)` });
      show(s.card, t < 28.7);
      // ---- cursor, in card-local coordinates mapped through the card transform
      const T = s.targets;
      const toS = (p) => [x + p.x * kk, y + p.y * kk];
      const path = [
        [22.0, ...toS({ x: 420, y: 520 })],
        [22.3, ...toS(T.tabMl)],
        [23.0, ...toS(T.sendMl)],
        [23.8, ...toS(T.tabOc)],
        [24.3, ...toS(T.sendOc)],
        [24.9, ...toS({ x: T.code.x + 120, y: T.code.y + 20 })],
        [25.7, ...toS(T.verify)],
        [26.3, ...toS({ x: T.verify2.x + 150, y: T.verify2.y + 40 })],
        [27.3, ...toS(T.verify2)],
      ];
      drawCursor(s.cur, t, path, CLICKS, L.u * 1.25, t < 28.0);
      // ---- method statements (left band / top band)
      const ms = (P ? 150 : 210) * L.u;
      const mo = P ? { x: L.m, y: L.H * 0.08, size: ms, color: "#16181d", lh: 0.86 } : { x: L.m, y: L.H * 0.2, size: ms, color: "#16181d", lh: 0.86 };
      drawStatement(s.stMethod.ml, t, 22.25, 24.0, mo);
      drawStatement(s.stMethod.oc, t, 24.0, 26.0, mo);
      drawStatement(s.stMethod.totp, t, 26.0, 28.0, mo);
      // ---- provider wall 28–30
      const wallOn = t >= 28.0;
      show(s.wall, wallOn);
      if (wallOn) {
        const cols = P ? 3 : 4, rows = P ? 4 : 3;
        const top = P ? L.H * 0.3 : L.H * 0.3;
        const gw = L.W - 2 * L.m, gh = (P ? L.H * 0.62 : L.H * 0.62);
        const gap = 22 * L.u;
        const tw = (gw - gap * (cols - 1)) / cols, th = (gh - gap * (rows - 1)) / rows;
        s.tiles.forEach((e, i) => {
          const c = i % cols, r = Math.floor(i / cols);
          const p = heavy(t - 28.125 - i * 0.1);
          const tx = L.m + c * (tw + gap), ty = top + r * (th + gap);
          // each tile flies out of the card's provider list (lower centre)
          const ox = L.W * 0.5 - tw / 2, oy = L.H * 1.05;
          S(e, { width: px(tw), height: px(th), fontSize: px(Math.min(th * 0.34, tw * 0.16)), display: t >= 28.1 + i * 0.1 ? "flex" : "none" });
          place(e, ox + (tx - ox) * p, oy + (ty - oy) * p, ` scale(${(0.6 + 0.4 * p).toFixed(3)})`);
        });
      }
      drawStatement(s.stWall, t, 28.0, 30.0, { x: L.m, y: P ? L.H * 0.08 : L.H * 0.07, size: (P ? 120 : 150) * L.u, color: "#16181d" });
      drawStatement(s.stWall2, t, 28.5, 30.0, { x: L.m, y: P ? L.H * 0.08 + 130 * L.u : L.H * 0.07 + 150 * L.u, size: (P ? 52 : 60) * L.u, color: "#5b6b7c" });
    },
  },
];
