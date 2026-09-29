// node audio/sync-check.mjs [out/stem-sfx.wav] — objective sound-sync check:
// finds transient onsets in the SFX stem (spectral-flux-free energy rise
// detector on 5 ms hops) and reports, for every cue in the sheet, the
// distance to the nearest detected onset. Cues are the picture's own event
// times, so this measures picture↔sound sync directly.
import { readFileSync } from "node:fs";
import { CUES } from "../lib/cues.mjs";
const file = process.argv[2] || "out/stem-sfx.wav";
const buf = readFileSync(file);
// minimal WAV reader (PCM 16/24-bit, any channels)
let off = 12, fmt, data;
while (off < buf.length) {
  const id = buf.toString("ascii", off, off + 4), size = buf.readUInt32LE(off + 4);
  if (id === "fmt ") fmt = { ch: buf.readUInt16LE(off + 10), sr: buf.readUInt32LE(off + 12), bits: buf.readUInt16LE(off + 22) };
  if (id === "data") { data = buf.subarray(off + 8, off + 8 + size); break; }
  off += 8 + size + (size & 1);
}
const bps = fmt.bits / 8, frames = data.length / (bps * fmt.ch);
const hop = Math.round(fmt.sr * 0.005);
const env = new Float32Array(Math.floor(frames / hop));
for (let h = 0; h < env.length; h++) {
  let e = 0;
  for (let i = h * hop; i < (h + 1) * hop; i++) {
    const o = i * bps * fmt.ch;
    const v = bps === 2 ? data.readInt16LE(o) / 32768 : data.readIntLE(o, 3) / 8388608;
    e += v * v;
  }
  env[h] = Math.sqrt(e / hop);
}
// onset = energy jumps by > 2.2x over the previous 30 ms mean and is above a floor
const on = [];
for (let h = 6; h < env.length; h++) {
  let m = 0;
  for (let k = h - 6; k < h; k++) m += env[k];
  m /= 6;
  if (env[h] > 0.004 && env[h] > m * 2.2 && (!on.length || h * 0.005 - on[on.length - 1] > 0.04)) on.push(h * 0.005);
}
const res = CUES.filter((c) => c.type !== "riser" && c.type !== "swell").map((c) => {
  let best = 9;
  for (const o of on) if (Math.abs(o - c.t) < Math.abs(best)) best = o - c.t;
  return { ...c, d: best };
});
const within = (ms) => res.filter((r) => Math.abs(r.d) * 1000 <= ms).length;
console.log(`onsets detected: ${on.length}; cues checked: ${res.length}`);
console.log(`within 10 ms: ${within(10)}  within 20 ms: ${within(20)}  within 40 ms: ${within(40)}`);
const bad = res.filter((r) => Math.abs(r.d) > 0.04);
if (bad.length) console.log("off by > 40 ms:", bad.map((b) => `${b.type}@${b.t}(${(b.d * 1000).toFixed(0)}ms)`).join(" "));
