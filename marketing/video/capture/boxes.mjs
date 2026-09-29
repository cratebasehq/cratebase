// node capture/boxes.mjs <shot> [regex] — list measured element boxes (CSS px)
// from capture/shots/<shot>.boxes.json, to pick camera ROIs and callout anchors.
import { readFileSync } from "node:fs";
const [shot, re] = process.argv.slice(2);
const { boxes, width, height, dpr } = JSON.parse(readFileSync(new URL(`./shots/${shot}.boxes.json`, import.meta.url)));
console.log(`${shot}: ${width}x${height} @${dpr}x`);
for (const b of boxes) if (!re || new RegExp(re, "i").test(b.text)) console.log(`${b.kind.padEnd(8)} ${String(b.x).padStart(7)} ${String(b.y).padStart(7)} ${String(b.w).padStart(7)} ${String(b.h).padStart(6)}  ${b.text.slice(0, 70)}`);
