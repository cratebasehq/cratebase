#!/usr/bin/env node
// audio/score.mjs — synthesizes the Cratebase launch-film score + SFX
// entirely in code from ../lib/timeline.js's beat grid and cue sheet, and
// writes a WAV. Zero dependencies, deterministic, Node 22 ES module.
//
//   node audio/score.mjs [--out out/score.wav] [--dur 64] [--tail 2.5] [--stems]

import path from "node:path";
import { BEAT, BAR, DUR, SECTIONS, CHORDS, CUES } from "../lib/cues.mjs";
import {
  TWO_PI,
  clamp,
  lerp,
  dbToGain,
  midiToFreq,
  mulberry32,
  saw,
  adsrAt,
  Biquad,
  StereoDelay,
  Reverb,
  softClip,
  Limiter,
  buildDuckEnvelope,
} from "./dsp.mjs";
import { chordTones, chordRootMidi } from "./theory.mjs";
import { writeWav } from "./wav.mjs";

// ------------------------------------------------------------------- CLI --

function parseArgs(argv) {
  const args = { out: "out/score.wav", dur: DUR, tail: 2.5, stems: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--out") args.out = argv[++i];
    else if (a === "--dur") args.dur = parseFloat(argv[++i]);
    else if (a === "--tail") args.tail = parseFloat(argv[++i]);
    else if (a === "--stems") args.stems = true;
    else if (a === "--bits") args.bits = parseInt(argv[++i], 10);
  }
  return args;
}
const args = parseArgs(process.argv.slice(2));

const SR = 48000;
const N = Math.ceil((args.dur + args.tail) * SR);

// ------------------------------------------------------------------ setup --

const rand = mulberry32(0xc0ffee);
const noise = () => rand() * 2 - 1;

function toSample(t) {
  return Math.round(t * SR);
}
function sectionAt(t) {
  for (const s of SECTIONS) if (t >= s.from && t < s.to) return s;
  return SECTIONS[SECTIONS.length - 1];
}
function chordAt(t) {
  const b = clamp(Math.floor(t / BAR), 0, CHORDS.length - 1);
  return CHORDS[b];
}
function panGains(p) {
  const angle = ((p + 1) * 0.25) * Math.PI; // p in [-1,1] -> [0, pi/2]
  return [Math.cos(angle), Math.sin(angle)];
}

function makeBus(n) {
  return { L: new Float32Array(n), R: new Float32Array(n) };
}

// Music arrangement (SECTIONS/CHORDS-driven): split into a "direct" bus
// (kick/hats/claps/arp — never ducked) and a "duckable" bus (bass/pad —
// pumps under the kick). SFX (CUES-driven) is a separate bus/stem.
const musicDirect = makeBus(N);
const musicDuckable = makeBus(N);
const arpDry = makeBus(N); // arp printed dry here first, then run through a delay
const sfx = makeBus(N);

const musicRevSend = new Float32Array(N); // mono sends -> per-bus reverb
const sfxRevSend = new Float32Array(N);

const kickTimes = []; // drives the pad/bass sidechain pump
const bigHitTimes = []; // drives the "music ducks 3dB under big SFX hits" duck

// ------------------------------------------------------- generic mono writer --

/**
 * Writes `len` samples of `fn(localIndex)` into `bus` starting at sample
 * `i0`, panned to `panPos` (-1..1), optionally sending a copy (pre-pan) to
 * a mono `send` bus scaled by `sendAmt`. Bounds-checked against bus length.
 */
function addMono(bus, i0, len, panPos, fn, send = null, sendAmt = 0) {
  const [gl, gr] = panGains(panPos);
  const total = bus.L.length;
  const start = Math.max(0, i0);
  const end = Math.min(total, i0 + len);
  for (let i = start; i < end; i++) {
    const s = fn(i - i0);
    bus.L[i] += s * gl;
    bus.R[i] += s * gr;
    if (send) send[i] += s * sendAmt;
  }
}

// ------------------------------------------------------------- instruments --

function kick(bus, i0, { gain = 0.5, pan = 0, pitchStart = 150, pitchEnd = 48, send = null, sendAmt = 0.03 } = {}) {
  const len = Math.round(0.35 * SR);
  const clickLen = Math.round(0.006 * SR);
  let phase = 0;
  addMono(
    bus,
    i0,
    len,
    pan,
    (i) => {
      const t = i / SR;
      const f = pitchEnd + (pitchStart - pitchEnd) * Math.exp(-t / 0.045);
      phase += (TWO_PI * f) / SR;
      const body = Math.sin(phase) * Math.exp(-t / 0.28);
      const click = i < clickLen ? noise() * Math.exp(-t / 0.003) * 0.6 : 0;
      return (body + click) * gain;
    },
    send,
    sendAmt
  );
}

function snare(bus, i0, { gain = 0.5, pan = 0, send = null, sendAmt = 0.05 } = {}) {
  const len = Math.round(0.22 * SR);
  const bp = new Biquad();
  bp.bandpass(SR, 1900, 1.2);
  let bodyPhase = 0;
  addMono(
    bus,
    i0,
    len,
    pan,
    (i) => {
      const t = i / SR;
      const n = bp.process(noise());
      const nEnv = Math.exp(-t / 0.1);
      bodyPhase += (TWO_PI * 190) / SR;
      const body = Math.sin(bodyPhase) * Math.exp(-t / 0.06);
      return (n * 0.9 * nEnv + body * 0.5) * gain;
    },
    send,
    sendAmt
  );
}

function clap(bus, i0, { gain = 0.5, pan = 0, send = null, sendAmt = 0.06 } = {}) {
  const len = Math.round(0.3 * SR);
  const bp = new Biquad();
  bp.bandpass(SR, 1500, 1.5);
  const bursts = [0, 0.011, 0.022, 0.033];
  addMono(
    bus,
    i0,
    len,
    pan,
    (i) => {
      const t = i / SR;
      const n = bp.process(noise());
      let env = 0;
      for (const b of bursts) if (t >= b) env = Math.max(env, Math.exp(-(t - b) / 0.02));
      const tailEnv = t > 0.033 ? Math.exp(-(t - 0.033) / 0.13) : 0;
      return n * gain * Math.max(env * 0.8, tailEnv * 0.5);
    },
    send,
    sendAmt
  );
}

function hat(bus, i0, { open = false, gain = 0.3, pan = 0, send = null, sendAmt = 0.015 } = {}) {
  const decay = open ? 0.22 : 0.045;
  const len = Math.round((decay * 2.2) * SR);
  const hp = new Biquad();
  hp.highpass(SR, open ? 6500 : 8500, 0.9);
  addMono(
    bus,
    i0,
    len,
    pan,
    (i) => {
      const t = i / SR;
      const n = hp.process(noise());
      return n * Math.exp(-t / decay) * gain;
    },
    send,
    sendAmt
  );
}

function bassNote(bus, i0, lenSamples, { midi, gain = 0.32, pan = 0, cutoff = 550, send = null, sendAmt = 0.02 } = {}) {
  const f = midiToFreq(midi);
  const dt = f / SR;
  let phase = 0;
  const lp = new Biquad();
  lp.lowpass(SR, cutoff, 0.8);
  addMono(
    bus,
    i0,
    lenSamples,
    pan,
    (i) => {
      const p01 = phase - Math.floor(phase);
      const sawV = saw(p01, dt);
      const sineV = Math.sin(TWO_PI * p01);
      phase += dt;
      const raw = sawV * 0.55 + sineV * 0.55;
      const filtered = lp.process(raw);
      const env = adsrAt(i, lenSamples, SR, { a: 0.004, d: 0.06, s: 0.8, r: 0.05 });
      return filtered * env * gain;
    },
    send,
    sendAmt
  );
}

function padTone(bus, i0, lenSamples, { midi, gain = 0.16, pan = 0, cutoff = 1100, a = 0.5, r = 0.6, send = null, sendAmt = 0.08 } = {}) {
  const f = midiToFreq(midi);
  const detune = 1.0029; // ~5 cents, for width without mud
  let ph1 = 0,
    ph2 = 0;
  const dt1 = (f * detune) / SR;
  const dt2 = (f / detune) / SR;
  const lp = new Biquad();
  lp.lowpass(SR, cutoff, 0.7);
  addMono(
    bus,
    i0,
    lenSamples,
    pan,
    (i) => {
      const p1 = ph1 - Math.floor(ph1);
      const p2 = ph2 - Math.floor(ph2);
      const s = (saw(p1, dt1) + saw(p2, dt2)) * 0.5;
      ph1 += dt1;
      ph2 += dt2;
      const filtered = lp.process(s);
      const env = adsrAt(i, lenSamples, SR, { a, d: 0.15, s: 0.85, r });
      return filtered * env * gain;
    },
    send,
    sendAmt
  );
}

/** Saw-through-LP pluck: used for the arp, chord stabs, and the pin drop. */
function pluckTone(bus, i0, lenSamples, { midi, gain = 0.2, pan = 0, cutoff = 2400, decay = 0.16, send = null, sendAmt = 0.05 } = {}) {
  const f = midiToFreq(midi);
  const dt = f / SR;
  let ph = 0;
  const lp = new Biquad();
  lp.lowpass(SR, cutoff, 0.85);
  addMono(
    bus,
    i0,
    lenSamples,
    pan,
    (i) => {
      const p = ph - Math.floor(ph);
      ph += dt;
      const s = lp.process(saw(p, dt));
      const env = adsrAt(i, lenSamples, SR, { a: 0.002, d: decay, s: 0, r: 0.12 });
      return s * env * gain;
    },
    send,
    sendAmt
  );
}

// ------------------------------------------------------ arrangement (music) --

function renderHookRiser(b, t0, chord) {
  // Snare roll accelerating 8ths -> 16ths -> 32nds into 4.0, then a gap
  // (the last 16th before the hit is silent so the drop has room to land).
  const rollEnd = 4.0 - BEAT / 4;
  const segments = [
    { from: 2.0, to: 3.0, step: BEAT / 2 },
    { from: 3.0, to: 3.5, step: BEAT / 4 },
    { from: 3.5, to: rollEnd, step: BEAT / 8 },
  ];
  let idx = 0;
  let total = 0;
  for (const s of segments) total += Math.round((s.to - s.from) / s.step);
  for (const s of segments) {
    for (let t = s.from; t < s.to - 1e-9; t += s.step) {
      const vel = 0.22 + 0.4 * (idx / Math.max(total - 1, 1));
      snare(musicDirect, toSample(t), { gain: vel, pan: 0, send: musicRevSend, sendAmt: 0.05 });
      idx++;
    }
  }
  // A pad fading up into the hit.
  const padLen = toSample(4.0) - toSample(2.0);
  chordTones(chord, 55).forEach((note, i) => {
    padTone(musicDuckable, toSample(2.0), padLen, {
      midi: note,
      gain: 0.14,
      pan: (i - 1.5) * 0.15,
      cutoff: 900,
      a: 1.6,
      r: 0.3,
      send: musicRevSend,
      sendAmt: 0.1,
    });
  });
}

function renderReveal(b, t0, chord) {
  const barLen = toSample(t0 + BAR) - toSample(t0);
  chordTones(chord, 58).forEach((note, i) => {
    padTone(musicDuckable, toSample(t0), barLen + toSample(0.4), {
      midi: note,
      gain: 0.18,
      pan: (i - 1.5) * 0.18,
      cutoff: 1100,
      a: 0.5,
      r: 0.6,
      send: musicRevSend,
      sendAmt: 0.1,
    });
  });
  const root = chordRootMidi(chord, 33);
  bassNote(musicDuckable, toSample(t0), barLen + toSample(0.3), { midi: root, gain: 0.3, pan: 0, cutoff: 300, send: musicRevSend, sendAmt: 0.05 });
  if (t0 >= 6.0 - 1e-9) {
    for (let beat = 0; beat < 4; beat++) {
      const t = t0 + beat * BEAT;
      kick(musicDirect, toSample(t), { gain: 0.26, pan: 0, send: musicRevSend, sendAmt: 0.03 });
      kickTimes.push(t);
    }
  }
}

function renderGrooveA(b, t0, chord) {
  for (let beat = 0; beat < 4; beat++) {
    const t = t0 + beat * BEAT;
    kick(musicDirect, toSample(t), { gain: 0.4, pan: 0, send: musicRevSend, sendAmt: 0.02 });
    kickTimes.push(t);
  }
  for (let beat = 0; beat < 4; beat++) {
    hat(musicDirect, toSample(t0 + beat * BEAT + BEAT / 2), { open: false, gain: 0.2, pan: 0.15, send: musicRevSend, sendAmt: 0.01 });
  }
  const root = chordRootMidi(chord, 36);
  const pattern = [1, 0, 1, 1, 0, 1, 0, 1]; // 8th grid, syncopated
  const step = BEAT / 2;
  const noteLen = Math.round(step * SR * 0.85);
  pattern.forEach((on, i) => {
    if (!on) return;
    bassNote(musicDuckable, toSample(t0 + i * step), noteLen, { midi: root, gain: 0.3, pan: 0, cutoff: 500, send: musicRevSend, sendAmt: 0.02 });
  });
}

function renderGrooveB(b, t0, chord) {
  renderGrooveA(b, t0, chord);
  [1, 3].forEach((beat) => clap(musicDirect, toSample(t0 + beat * BEAT), { gain: 0.34, pan: 0, send: musicRevSend, sendAmt: 0.04 }));

  const barLen = toSample(t0 + BAR) - toSample(t0);
  chordTones(chord, 60).forEach((note, i) => {
    padTone(musicDuckable, toSample(t0), barLen + toSample(0.3), {
      midi: note,
      gain: 0.11,
      pan: (i - 1.5) * 0.2,
      cutoff: 950,
      a: 0.05,
      r: 0.4,
      send: musicRevSend,
      sendAmt: 0.07,
    });
  });

  const tones = chordTones(chord, 72);
  const step = BEAT / 4;
  const noteLen = Math.round(step * SR * 0.9);
  for (let i = 0; i < 16; i++) {
    const note = tones[i % tones.length];
    pluckTone(arpDry, toSample(t0 + i * step), noteLen, {
      midi: note,
      gain: 0.15,
      pan: i % 2 === 0 ? -0.35 : 0.35,
      cutoff: 2600,
      send: musicRevSend,
      sendAmt: 0.04,
    });
  }
}

function renderDrive(b, t0, chord) {
  for (let beat = 0; beat < 4; beat++) {
    const t = t0 + beat * BEAT;
    kick(musicDirect, toSample(t), { gain: 0.44, pan: 0, send: musicRevSend, sendAmt: 0.02 });
    kickTimes.push(t);
  }
  const step16 = BEAT / 4;
  for (let i = 0; i < 16; i++) {
    const accent = i % 4 === 0 ? 0.32 : 0.16;
    hat(musicDirect, toSample(t0 + i * step16), { open: false, gain: accent, pan: 0.1, send: musicRevSend, sendAmt: 0.01 });
  }
  for (let beat = 0; beat < 4; beat++) {
    hat(musicDirect, toSample(t0 + beat * BEAT + BEAT / 2), { open: true, gain: 0.19, pan: -0.12, send: musicRevSend, sendAmt: 0.015 });
  }
  [1, 3].forEach((beat) => clap(musicDirect, toSample(t0 + beat * BEAT), { gain: 0.37, pan: 0, send: musicRevSend, sendAmt: 0.04 }));

  const root = chordRootMidi(chord, 36);
  const pattern = [1, 0, 1, 1, 0, 1, 0, 1, 1, 0, 0, 1, 1, 0, 1, 0];
  const noteLen16 = Math.round(step16 * SR * 0.8);
  pattern.forEach((on, i) => {
    if (!on) return;
    bassNote(musicDuckable, toSample(t0 + i * step16), noteLen16, { midi: root, gain: 0.29, pan: 0, cutoff: 700, send: musicRevSend, sendAmt: 0.02 });
  });

  const barLen = toSample(t0 + BAR) - toSample(t0);
  chordTones(chord, 60).forEach((note, i) => {
    padTone(musicDuckable, toSample(t0), barLen + toSample(0.3), {
      midi: note,
      gain: 0.12,
      pan: (i - 1.5) * 0.2,
      cutoff: 1300,
      a: 0.04,
      r: 0.35,
      send: musicRevSend,
      sendAmt: 0.07,
    });
  });

  const tones1 = chordTones(chord, 72);
  for (let i = 0; i < 16; i++) {
    pluckTone(arpDry, toSample(t0 + i * step16), noteLen16, {
      midi: tones1[i % tones1.length],
      gain: 0.16,
      pan: i % 2 === 0 ? -0.35 : 0.35,
      cutoff: 2900,
      send: musicRevSend,
      sendAmt: 0.04,
    });
  }
  const tones2 = chordTones(chord, 84);
  for (let i = 0; i < 16; i += 2) {
    pluckTone(arpDry, toSample(t0 + i * step16 + step16 / 2), noteLen16, {
      midi: tones2[(i / 2) % tones2.length],
      gain: 0.1,
      pan: i % 4 === 0 ? 0.5 : -0.5,
      cutoff: 4200,
      decay: 0.1,
      send: musicRevSend,
      sendAmt: 0.05,
    });
  }

  // tiny fill on beat 4
  const fillStart = t0 + 3 * BEAT;
  for (let k = 0; k < 3; k++) {
    snare(musicDirect, toSample(fillStart + (k * BEAT) / 6), { gain: 0.2 + k * 0.06, pan: 0, send: musicRevSend, sendAmt: 0.04 });
  }
}

function renderProof(b, t0, chord) {
  kick(musicDirect, toSample(t0), { gain: 0.38, pan: 0, send: musicRevSend, sendAmt: 0.03 });
  kickTimes.push(t0);

  const barLen = toSample(t0 + BAR) - toSample(t0);
  const root = chordRootMidi(chord, 33);
  bassNote(musicDuckable, toSample(t0), barLen + toSample(0.3), { midi: root, gain: 0.26, pan: 0, cutoff: 280, send: musicRevSend, sendAmt: 0.05 });
  chordTones(chord, 58).forEach((note, i) => {
    padTone(musicDuckable, toSample(t0), barLen + toSample(0.4), {
      midi: note,
      gain: 0.16,
      pan: (i - 1.5) * 0.15,
      cutoff: 700,
      a: 0.4,
      r: 0.6,
      send: musicRevSend,
      sendAmt: 0.1,
    });
  });

  const tones = chordTones(chord, 72);
  const step = BEAT / 2;
  const noteLen = Math.round(step * SR * 0.8);
  for (let i = 0; i < 4; i++) {
    pluckTone(arpDry, toSample(t0 + i * step), noteLen, {
      midi: tones[i % tones.length],
      gain: 0.09,
      pan: i % 2 === 0 ? -0.2 : 0.2,
      cutoff: 800,
      decay: 0.25,
      send: musicRevSend,
      sendAmt: 0.05,
    });
  }
}

function renderCtaHit(b, t0, chord) {
  for (let beat = 0; beat < 4; beat++) {
    const t = t0 + beat * BEAT;
    kick(musicDirect, toSample(t), { gain: 0.4, pan: 0, send: musicRevSend, sendAmt: 0.03 });
    kickTimes.push(t);
  }
  const barLen = toSample(t0 + BAR) - toSample(t0);
  chordTones(chord, 58).forEach((note, i) => {
    padTone(musicDuckable, toSample(t0), barLen + toSample(0.6), {
      midi: note,
      gain: 0.22,
      pan: (i - 1.5) * 0.15,
      cutoff: 1500,
      a: 0.15,
      r: 0.8,
      send: musicRevSend,
      sendAmt: 0.12,
    });
  });
}

function renderResolve(b, t0, chord) {
  const isLast = b === CHORDS.length - 1;
  const barLen = toSample(t0 + BAR) - toSample(t0);
  const ringExtra = isLast ? N - toSample(t0 + BAR) : toSample(0.5);
  chordTones(chord, 55).forEach((note, i) => {
    padTone(musicDuckable, toSample(t0), barLen + ringExtra, {
      midi: note,
      gain: isLast ? 0.3 : 0.22,
      pan: (i - 1.5) * 0.15,
      cutoff: isLast ? 2000 : 1200,
      a: 0.3,
      r: isLast ? Math.max(ringExtra / SR - 0.2, 0.8) : 0.6,
      send: musicRevSend,
      sendAmt: 0.16,
    });
  });
  const root = chordRootMidi(chord, 33);
  bassNote(musicDuckable, toSample(t0), barLen + ringExtra, { midi: root, gain: 0.22, pan: 0, cutoff: 260, send: musicRevSend, sendAmt: 0.06 });
  if (Math.abs(t0 - 62.0) < 1e-6) {
    kick(musicDirect, toSample(62.0), { gain: 0.3, pan: 0, pitchStart: 90, pitchEnd: 40, send: musicRevSend, sendAmt: 0.05 });
  }
}

const SECTION_RENDERERS = {
  "hook-words": null, // silence except the per-word `stab` cues
  "hook-riser": renderHookRiser,
  reveal: renderReveal,
  "groove-a": renderGrooveA,
  "groove-b": renderGrooveB,
  drive: renderDrive,
  proof: renderProof,
  "cta-hit": renderCtaHit,
  resolve: renderResolve,
};

for (let b = 0; b < CHORDS.length; b++) {
  const t0 = b * BAR;
  if (t0 >= args.dur + args.tail) break;
  const section = sectionAt(t0).name;
  const fn = SECTION_RENDERERS[section];
  if (fn) fn(b, t0, CHORDS[b]);
}

// Arp gets a slight stereo delay (insert: dry + wet) before joining the mix.
{
  const arpDelay = new StereoDelay(SR, 0.1875, 0.25, 0.28, 0.32);
  for (let i = 0; i < N; i++) {
    const [dl, dr] = arpDelay.process(arpDry.L[i], arpDry.R[i]);
    musicDirect.L[i] += dl;
    musicDirect.R[i] += dr;
  }
}

// --------------------------------------------------------------------- SFX --

function sfxHit(i0, { gain = 1 } = {}) {
  const subLen = Math.round(3.2 * SR);
  let subPhase = 0;
  addMono(
    sfx,
    i0,
    subLen,
    0,
    (i) => {
      const t = i / SR;
      const f = 35 + (90 - 35) * Math.exp(-t / 0.35);
      subPhase += (TWO_PI * f) / SR;
      return Math.sin(subPhase) * Math.exp(-t / 1.8) * gain;
    },
    sfxRevSend,
    0.15
  );
  const boomLen = Math.round(2.0 * SR);
  let boomPhase = 0;
  addMono(
    sfx,
    i0,
    boomLen,
    0,
    (i) => {
      const t = i / SR;
      boomPhase += (TWO_PI * 55) / SR;
      return Math.sin(boomPhase) * Math.exp(-t / 0.9) * 0.7 * gain;
    },
    sfxRevSend,
    0.2
  );
  const crackLen = Math.round(0.09 * SR);
  const bp = new Biquad();
  bp.bandpass(SR, 1600, 0.7);
  addMono(
    sfx,
    i0,
    crackLen,
    0,
    (i) => {
      const t = i / SR;
      const n = bp.process(noise());
      return softClip(n * 2.2, 0.6) * Math.exp(-t / 0.02) * gain;
    },
    sfxRevSend,
    0.3
  );
}

function renderStab(t, p) {
  const i0 = toSample(t);
  kick(sfx, i0, { gain: 0.48, pan: 0, send: sfxRevSend, sendAmt: 0.04 });
  const chord = chordAt(t);
  const topNote = p.note ?? 64;
  let tones = chordTones(chord, topNote - 14).filter((n) => n <= topNote);
  if (tones.length < 3) tones = [topNote - 7, topNote - 3, topNote];
  tones = tones.slice(-3);
  const lenSamples = Math.round(0.35 * SR);
  tones.forEach((note, idx) => {
    pluckTone(sfx, i0, lenSamples, {
      midi: note,
      gain: 0.2,
      pan: (idx - 1) * 0.25,
      cutoff: 2000,
      decay: 0.18,
      send: sfxRevSend,
      sendAmt: 0.06,
    });
  });
}

function sfxRiser(i0, lenSamples, { gain = 0.6 } = {}) {
  const bp = new Biquad();
  addMono(
    sfx,
    i0,
    lenSamples,
    0,
    (i) => {
      const p = i / lenSamples;
      const fc = 250 + (7000 - 250) * Math.pow(p, 1.6);
      bp.bandpass(SR, fc, 0.9);
      const n = bp.process(noise());
      return n * Math.pow(p, 1.3) * gain;
    },
    sfxRevSend,
    0.1
  );
}

function sfxWhoosh(i0, lenSamples, { gain = 0.6, dir = 1 } = {}) {
  const bp = new Biquad();
  const total = sfx.L.length;
  const start = Math.max(0, i0);
  const end = Math.min(total, i0 + lenSamples);
  for (let i = start; i < end; i++) {
    const p = (i - i0) / lenSamples;
    const fc = dir > 0 ? lerp(400, 4000, p) : lerp(4000, 400, p);
    bp.bandpass(SR, fc, 1.1);
    const n = bp.process(noise());
    const env = Math.sin(Math.PI * clamp(p, 0, 1)) * gain;
    const panPos = clamp(lerp(-0.8, 0.8, p) * dir, -1, 1);
    const [gl, gr] = panGains(panPos);
    sfx.L[i] += n * env * gl;
    sfx.R[i] += n * env * gr;
    sfxRevSend[i] += n * env * 0.08;
  }
}

function sfxClick(i0, { gain = 0.35 } = {}) {
  const len = Math.round(0.03 * SR);
  let phase = 0;
  addMono(
    sfx,
    i0,
    len,
    0,
    (i) => {
      const t = i / SR;
      const n = t < 0.002 ? noise() * Math.exp(-t / 0.0015) : 0;
      phase += (TWO_PI * 1200) / SR;
      const body = Math.sin(phase) * Math.exp(-t / 0.02) * 0.5;
      return (n * 0.8 + body) * gain;
    },
    sfxRevSend,
    0.02
  );
}

function sfxKeys(i0, n, dur, { gain = 0.3 } = {}) {
  const totalSamples = Math.round(dur * SR);
  for (let k = 0; k < n; k++) {
    // one click at the start of each of the picture's n reveal steps
    // (text reveals in quantized steps of dur/n), ±1.5 ms humanize only
    const jitter = Math.round((rand() - 0.5) * 0.003 * SR);
    const t0 = Math.max(0, Math.round((k / n) * totalSamples) + jitter);
    const freq = 900 + rand() * 500;
    const lvl = 0.7 + rand() * 0.3;
    const pan = (rand() - 0.5) * 0.6;
    const len = Math.round(0.025 * SR);
    let phase = 0;
    addMono(
      sfx,
      i0 + t0,
      len,
      pan,
      (i) => {
        const t = i / SR;
        const nn = t < 0.0015 ? noise() * Math.exp(-t / 0.001) : 0;
        phase += (TWO_PI * freq) / SR;
        const body = Math.sin(phase) * Math.exp(-t / 0.012) * 0.4;
        return (nn * 0.7 + body) * gain * lvl;
      },
      sfxRevSend,
      0.015
    );
  }
}

function sfxPop(i0, { note = 69, gain = 0.4 } = {}) {
  const f1 = midiToFreq(note);
  const f0 = f1 * 1.8;
  const len = Math.round(0.12 * SR);
  let phase = 0;
  addMono(
    sfx,
    i0,
    len,
    0,
    (i) => {
      const t = i / SR;
      const f = f1 + (f0 - f1) * Math.exp(-t / 0.02);
      phase += (TWO_PI * f) / SR;
      return Math.sin(phase) * Math.exp(-t / 0.09) * gain;
    },
    sfxRevSend,
    0.05
  );
}

function sfxPin(i0, { note = 60, gain = 0.5 } = {}) {
  const lenP = Math.round(0.35 * SR);
  pluckTone(sfx, i0, lenP, { midi: note, gain: gain * 0.9, pan: 0.15, cutoff: 2400, decay: 0.12, send: sfxRevSend, sendAmt: 0.06 });
  const lenT = Math.round(0.25 * SR);
  let tp = 0;
  addMono(
    sfx,
    i0,
    lenT,
    -0.15,
    (i) => {
      const t = i / SR;
      tp += (TWO_PI * 90) / SR;
      return Math.sin(tp) * Math.exp(-t / 0.09) * gain * 0.6;
    },
    sfxRevSend,
    0.04
  );
}

function sfxTick(i0, { note = 96, gain = 0.25 } = {}) {
  const f = midiToFreq(note ?? 96);
  const len = Math.round(0.02 * SR);
  let ph = 0;
  addMono(
    sfx,
    i0,
    len,
    0,
    (i) => {
      const t = i / SR;
      ph += (TWO_PI * f) / SR;
      return Math.sin(ph) * Math.exp(-t / 0.008) * gain;
    },
    sfxRevSend,
    0.01
  );
}

function sfxDigit(i0, { gain = 0.38 } = {}) {
  const len = Math.round(0.02 * SR);
  const hp = new Biquad();
  hp.highpass(SR, 3500, 0.8);
  addMono(
    sfx,
    i0,
    len,
    0,
    (i) => {
      const t = i / SR;
      const n = hp.process(noise());
      return n * Math.exp(-t / 0.006) * gain;
    },
    sfxRevSend,
    0.01
  );
}

function sfxConfirm(i0, { gain = 0.4 } = {}) {
  const notes = [74, 79];
  notes.forEach((note, idx) => {
    const f = midiToFreq(note);
    const len = Math.round(0.22 * SR);
    const start = idx * Math.round(0.09 * SR);
    let ph = 0;
    addMono(
      sfx,
      i0 + start,
      len,
      0,
      (i) => {
        const t = i / SR;
        ph += (TWO_PI * f) / SR;
        return Math.sin(ph) * Math.exp(-t / 0.16) * gain;
      },
      sfxRevSend,
      0.06
    );
  });
}

function sfxSub(i0, { gain = 0.6 } = {}) {
  const len = Math.round(1.0 * SR);
  let ph = 0;
  addMono(
    sfx,
    i0,
    len,
    0,
    (i) => {
      const t = i / SR;
      ph += (TWO_PI * 50) / SR;
      return Math.sin(ph) * Math.exp(-t / 0.5) * gain;
    },
    sfxRevSend,
    0.08
  );
}

function sfxSwell(i0, lenSamples, { gain = 0.5 } = {}) {
  const hp = new Biquad();
  addMono(
    sfx,
    i0,
    lenSamples,
    0,
    (i) => {
      const p = i / lenSamples;
      const fc = lerp(800, 9000, Math.pow(p, 1.5));
      hp.highpass(SR, fc, 0.7);
      const n = hp.process(noise());
      return n * Math.pow(p, 2.2) * gain;
    },
    sfxRevSend,
    0.1
  );
}

function sfxSend(i0, { gain = 0.5 } = {}) {
  sfxWhoosh(i0, Math.round(0.5 * SR), { gain: gain * 0.8, dir: 1 });
  const len = Math.round(0.3 * SR);
  let ph = 0;
  addMono(
    sfx,
    i0 + Math.round(0.4 * SR),
    len,
    0,
    (i) => {
      const t = i / SR;
      ph += (TWO_PI * 70) / SR;
      return Math.sin(ph) * Math.exp(-t / 0.12) * gain * 0.7;
    },
    sfxRevSend,
    0.05
  );
}

const CUE_HANDLERS = {
  hit: (t, p) => {
    sfxHit(toSample(t), { gain: p.gain ?? 1 });
    bigHitTimes.push(t);
  },
  stab: (t, p) => renderStab(t, p),
  riser: (t, p) => sfxRiser(toSample(t), Math.round((p.dur ?? 1) * SR), {}),
  whoosh: (t, p) => sfxWhoosh(toSample(t), Math.round((p.dur ?? 0.4) * SR), { gain: p.gain ?? 0.6 }),
  click: (t, p) => sfxClick(toSample(t), { gain: p.gain ?? 0.35 }),
  keys: (t, p) => sfxKeys(toSample(t), p.n ?? 8, p.dur ?? 0.6, {}),
  pop: (t, p) => sfxPop(toSample(t), { note: p.note ?? 69 }),
  pin: (t, p) => sfxPin(toSample(t), { note: p.note ?? 60 }),
  tick: (t, p) => sfxTick(toSample(t), { note: p.note ?? 96 }),
  digit: (t, p) => sfxDigit(toSample(t), {}),
  confirm: (t, p) => sfxConfirm(toSample(t), {}),
  sub: (t, p) => {
    sfxSub(toSample(t), { gain: p.gain ?? 0.6 });
    bigHitTimes.push(t);
  },
  swell: (t, p) => sfxSwell(toSample(t), Math.round((p.dur ?? 1) * SR), {}),
  send: (t, p) => {
    sfxSend(toSample(t), {});
    bigHitTimes.push(t);
  },
};

for (const c of CUES) {
  if (toSample(c.t) >= N) continue;
  const h = CUE_HANDLERS[c.type];
  if (!h) {
    console.warn(`[score] unknown cue type "${c.type}" at t=${c.t} — skipping`);
    continue;
  }
  h(c.t, c);
}

// ---------------------------------------------------------------- mixdown --

const kickDuck = buildDuckEnvelope(N, SR, kickTimes, { depthDb: 8, attack: 0.006, release: 0.16 });
const sfxDuck = buildDuckEnvelope(N, SR, bigHitTimes, { depthDb: 3, attack: 0.01, release: 0.45 });

const reverbMusic = new Reverb(SR, { roomSize: 0.82, damp: 0.35 });
const reverbSfx = new Reverb(SR, { roomSize: 0.88, damp: 0.25 });
const musicWet = reverbMusic.renderBuffer(musicRevSend);
const sfxWet = reverbSfx.renderBuffer(sfxRevSend);

const outMusic = makeBus(N);
const outSfx = makeBus(N);
for (let i = 0; i < N; i++) {
  let mL = musicDirect.L[i] + musicDuckable.L[i] * kickDuck[i] + musicWet.L[i] * 0.9;
  let mR = musicDirect.R[i] + musicDuckable.R[i] * kickDuck[i] + musicWet.R[i] * 0.9;
  mL *= sfxDuck[i];
  mR *= sfxDuck[i];
  outMusic.L[i] = mL;
  outMusic.R[i] = mR;
  outSfx.L[i] = sfx.L[i] + sfxWet.L[i];
  outSfx.R[i] = sfx.R[i] + sfxWet.R[i];
}

const MUSIC_GAIN = dbToGain(-1.5);
const SFX_GAIN = dbToGain(0.5); // SFX sits a bit above the music

const finalL = new Float32Array(N);
const finalR = new Float32Array(N);
const limiter = new Limiter(SR, { threshold: dbToGain(-1.0), release: 0.12 });
for (let i = 0; i < N; i++) {
  let l = outMusic.L[i] * MUSIC_GAIN + outSfx.L[i] * SFX_GAIN;
  let r = outMusic.R[i] * MUSIC_GAIN + outSfx.R[i] * SFX_GAIN;
  l = softClip(l, 0.85);
  r = softClip(r, 0.85);
  [l, r] = limiter.processStereo(l, r);
  finalL[i] = l;
  finalR[i] = r;
}

// ------------------------------------------------------------------ output --

const bitDepth = args.bits === 16 ? 16 : 24;
const outPath = path.resolve(process.cwd(), args.out);
writeWav(outPath, finalL, finalR, SR, bitDepth);
console.log(`[score] wrote ${outPath} (${(N / SR).toFixed(2)}s, ${SR}Hz, ${bitDepth}-bit stereo)`);

if (args.stems) {
  const stemMusicL = new Float32Array(N);
  const stemMusicR = new Float32Array(N);
  const stemSfxL = new Float32Array(N);
  const stemSfxR = new Float32Array(N);
  for (let i = 0; i < N; i++) {
    stemMusicL[i] = softClip(outMusic.L[i] * MUSIC_GAIN, 0.9);
    stemMusicR[i] = softClip(outMusic.R[i] * MUSIC_GAIN, 0.9);
    stemSfxL[i] = softClip(outSfx.L[i] * SFX_GAIN, 0.9);
    stemSfxR[i] = softClip(outSfx.R[i] * SFX_GAIN, 0.9);
  }
  const dir = path.dirname(outPath);
  const musicPath = path.join(dir, "stem-music.wav");
  const sfxPath = path.join(dir, "stem-sfx.wav");
  writeWav(musicPath, stemMusicL, stemMusicR, SR, bitDepth);
  writeWav(sfxPath, stemSfxL, stemSfxR, SR, bitDepth);
  console.log(`[score] wrote ${musicPath}`);
  console.log(`[score] wrote ${sfxPath}`);
}
