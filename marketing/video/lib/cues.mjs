// Node-side aggregator: importing every scene module runs its top-level
// addCue() calls, so the score (audio/score.mjs) sees exactly the cue
// sheet the picture uses. Scenes touch the DOM only inside build/draw.
import "../scenes/hook.js";
import "../scenes/binary.js";
import "../scenes/data.js";
import "../scenes/auth.js";
import "../scenes/email.js";
import "../scenes/superpowers.js";
import "../scenes/proof.js";
import "../scenes/cta.js";
export * from "./timeline.js";
