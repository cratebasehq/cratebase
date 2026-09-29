// audio/theory.mjs — tiny chord-name parser for the D-minor progression in
// lib/timeline.js. Only needs to understand the qualities actually used
// there, with a sane fallback for anything else.

const PITCH_CLASS = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 };

const QUALITY = {
  "m9": [0, 3, 7, 10, 14],
  "maj9": [0, 4, 7, 11, 14],
  "maj7": [0, 4, 7, 11],
  "6": [0, 4, 7, 9],
  "m6": [0, 3, 7, 9],
  "7sus4": [0, 5, 7, 10],
  "sus4": [0, 5, 7],
  "m7": [0, 3, 7, 10],
  "7": [0, 4, 7, 10],
  "m": [0, 3, 7],
  "": [0, 4, 7],
};

function parseName(name) {
  const m = name.match(/^([A-G])([b#]?)(.*)$/);
  if (!m) return { pc: 2, quality: "m" }; // fall back to D minor
  let pc = PITCH_CLASS[m[1]];
  if (m[2] === "#") pc += 1;
  else if (m[2] === "b") pc -= 1;
  pc = ((pc % 12) + 12) % 12;
  const quality = m[3] in QUALITY ? m[3] : "";
  return { pc, quality };
}

export function chordIntervals(name) {
  return QUALITY[parseName(name).quality] || QUALITY[""];
}

export function chordRootPc(name) {
  return parseName(name).pc;
}

/** Lowest MIDI note with the chord's root pitch class at or above baseMidi. */
export function chordRootMidi(name, baseMidi = 36) {
  const pc = chordRootPc(name);
  let root = baseMidi - (baseMidi % 12) + pc;
  if (root < baseMidi) root += 12;
  return root;
}

/** Chord tones (root + intervals) voiced with the root at/above baseMidi. */
export function chordTones(name, baseMidi = 60) {
  const root = chordRootMidi(name, baseMidi);
  return chordIntervals(name).map((iv) => root + iv);
}
