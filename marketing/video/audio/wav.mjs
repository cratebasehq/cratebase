// audio/wav.mjs — minimal stereo PCM WAV writer (16 or 24-bit), no deps.
import fs from "node:fs";
import path from "node:path";
import { clamp } from "./dsp.mjs";

export function writeWav(filePath, L, R, sr, bitDepth = 24) {
  const N = L.length;
  const bytesPerSample = bitDepth / 8;
  const blockAlign = 2 * bytesPerSample;
  const dataSize = N * blockAlign;
  const buf = Buffer.alloc(44 + dataSize);

  buf.write("RIFF", 0, "ascii");
  buf.writeUInt32LE(36 + dataSize, 4);
  buf.write("WAVE", 8, "ascii");
  buf.write("fmt ", 12, "ascii");
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(2, 22); // stereo
  buf.writeUInt32LE(sr, 24);
  buf.writeUInt32LE(sr * blockAlign, 28);
  buf.writeUInt16LE(blockAlign, 32);
  buf.writeUInt16LE(bitDepth, 34);
  buf.write("data", 36, "ascii");
  buf.writeUInt32LE(dataSize, 40);

  const maxVal = Math.pow(2, bitDepth - 1) - 1;
  let off = 44;
  if (bitDepth === 24) {
    for (let i = 0; i < N; i++) {
      buf.writeIntLE(Math.round(clamp(L[i], -1, 1) * maxVal), off, 3);
      buf.writeIntLE(Math.round(clamp(R[i], -1, 1) * maxVal), off + 3, 3);
      off += 6;
    }
  } else {
    for (let i = 0; i < N; i++) {
      buf.writeInt16LE(Math.round(clamp(L[i], -1, 1) * maxVal), off);
      buf.writeInt16LE(Math.round(clamp(R[i], -1, 1) * maxVal), off + 2);
      off += 4;
    }
  }

  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, buf);
}
