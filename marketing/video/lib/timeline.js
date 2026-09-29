// The film's beat grid and cue sheet — the single source of truth that
// both the picture (main.js / scenes) and the synthesized score
// (audio/score.mjs) read, so every cut, key motion and sound lands on the
// same grid by construction instead of being lined up by eye.
//
// 120 BPM, 4/4: beat = 0.5 s, bar = 2 s. 32 bars = 64 s.

export const BPM = 120;
export const BEAT = 60 / BPM; // 0.5 s
export const BAR = BEAT * 4; // 2 s
export const DUR = 64;

/** Absolute time of bar `n` (1-based) plus `beats` beats (may be fractional). */
export const at = (n, beats = 0) => (n - 1) * BAR + beats * BEAT;

export const CHAPTERS = [
  { id: "hook", from: 0, to: 8 },
  { id: "binary", from: 8, to: 14 },
  { id: "data", from: 14, to: 22 },
  { id: "auth", from: 22, to: 30 },
  { id: "email", from: 30, to: 36 },
  { id: "super", from: 36, to: 48 },
  { id: "proof", from: 48, to: 56 },
  { id: "cta", from: 56, to: 64 },
];

// Arrangement: how dense the score is under each stretch of picture.
// `audio/score.mjs` maps each section name to a set of parts.
export const SECTIONS = [
  { from: 0, to: 2, name: "hook-words" }, // a kick + minor stab on each word, nothing else
  { from: 2, to: 4, name: "hook-riser" }, // noise riser + rising pad, snare roll into 4.0
  { from: 4, to: 8, name: "reveal" }, // the hit, then pad + sub, soft pulse from 6.0
  { from: 8, to: 14, name: "groove-a" }, // four-on-floor (soft), 8th hats, bass
  { from: 14, to: 36, name: "groove-b" }, // + clap on 2 & 4, pluck arp
  { from: 36, to: 48, name: "drive" }, // 16th hats, second arp, open hats — the peak
  { from: 48, to: 56, name: "proof" }, // drums drop out: sub + pad + count ticks
  { from: 56, to: 58, name: "cta-hit" }, // the second hit, pad swells
  { from: 58, to: 64, name: "resolve" }, // final chord, long tail, no drums
];

// One chord per bar (2 s). D minor, voiced by audio/score.mjs.
export const CHORDS = [
  "Dm9", "Bbmaj7", "Fmaj9", "C6", // bars 1-4
  "Dm9", "Bbmaj7", "Fmaj9", "C6",
  "Dm9", "Bbmaj7", "Fmaj9", "C6",
  "Gm9", "Bbmaj7", "Fmaj9", "A7sus4",
  "Dm9", "Bbmaj7", "Fmaj9", "C6",
  "Dm9", "Bbmaj7", "Gm9", "A7sus4",
  "Bbmaj7", "Bbmaj7", "Gm9", "A7sus4", // proof: slower harmonic motion
  "Dm9", "Bbmaj7", "Fmaj9", "Fmaj9", // resolve on F (relative major) and let it ring
];

// SFX cue sheet. Types understood by audio/score.mjs:
//   hit        big impact (sub drop + noise crack + long tail)   {gain}
//   stab       short minor chord stab + kick                     {note}
//   riser      filtered-noise riser ending at t + dur            {dur}
//   whoosh     band-passed noise sweep centred on t               {dur, gain}
//   click      UI press (short tick + low body)                   {gain}
//   keys       typing burst: n key clicks spread over dur         {n, dur}
//   pop        small UI element landing (sine blip)               {note}
//   pin        map pin drop (pitched pluck + thud)                 {note}
//   tick       badge/counter tick (tiny high blip)                 {note}
//   digit      count-up tick                                       {}
//   confirm    two-note rising chime                              {}
//   sub        sub-bass boom only                                  {gain}
//   swell      reverse-cymbal swell ending at t + dur              {dur}
//   send       mail "whoosh-thunk"                                 {}
// `note` is a MIDI note number.
const cues = [];
const cue = (t, type, p = {}) => cues.push({ t: +t.toFixed(4), type, ...p });

// Hook — words on beats 0,1,2,3.
[0, 0.5, 1.0, 1.5].forEach((t, i) => cue(t, "stab", { note: [62, 65, 69, 72][i] }));
cue(2.0, "stab", { note: 57 });
cue(2.5, "pop", { note: 69 });
cue(2.0, "riser", { dur: 2.0 });
cue(3.25, "swell", { dur: 0.75 });
cue(4.0, "hit", { gain: 1.0 });
cue(4.5, "pop", { note: 74 });
cue(5.0, "whoosh", { dur: 0.4, gain: 0.5 });
cue(6.0, "pop", { note: 69 });
cue(7.0, "whoosh", { dur: 1.0, gain: 0.8 });

export const CUES = cues;
export function addCue(t, type, p) {
  cue(t, type, p);
}
