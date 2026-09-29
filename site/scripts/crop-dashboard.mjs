// Camera-style crops of the real dashboard captures in src/assets/dashboard/
// (3200x2000 @2x PNGs, see manifest.json). Nothing here is redrawn: each crop
// is a rectangle of an actual screenshot, cut tighter so the landing page can
// show it large and legible instead of shrinking a full 1600px screen.
//
//   node scripts/crop-dashboard.mjs
//
// Rectangles are in the captures' CSS pixels (1600x1000 viewport); the script
// doubles them for the @2x source. Light and dark captures share a layout, so
// one rectangle serves both themes.
import sharp from "sharp";
import { mkdirSync } from "node:fs";

const crops = [
  // Hero: the places collection's schema tab, the fields list and the API
  // rules editor (the page's own content column, sidebar/top bar trimmed).
  { out: "hero-schema", src: "03-collections-places-schema-rules", x: 412, y: 330, w: 1016, h: 590 },
  // Hero overlay: the posts records table, title/body/published columns.
  { out: "hero-records", src: "11-collections-posts-records-search", x: 456, y: 124, w: 610, h: 132 },
  // Tour: visual email template editor.
  { out: "tour-email", src: "08-settings-email-template-editor", x: 0, y: 0, w: 1600, h: 900 },
  // Tour: users auth collection, sign-in methods.
  { out: "tour-auth", src: "06-users-auth-options", x: 240, y: 216, w: 1200, h: 675 },
  // Tour: settings, grouped into seven sections.
  { out: "tour-settings", src: "05-settings-index", x: 0, y: 0, w: 1320, h: 742 },
];

for (const theme of ["light", "dark"]) {
  mkdirSync(`src/assets/crops/${theme}`, { recursive: true });
  for (const c of crops) {
    await sharp(`src/assets/dashboard/${theme}/${c.src}.png`)
      .extract({ left: c.x * 2, top: c.y * 2, width: c.w * 2, height: c.h * 2 })
      .png({ compressionLevel: 9 })
      .toFile(`src/assets/crops/${theme}/${c.out}.png`);
    console.log(theme, c.out, `${c.w * 2}x${c.h * 2}`);
  }
}
