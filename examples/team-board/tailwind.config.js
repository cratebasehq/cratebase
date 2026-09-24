/** Design tokens — see README.md "Design" for the reasoning: a shipping
 * crate/manifest vocabulary (Cratebase → crates), flat surfaces with
 * borders doing structural work instead of soft ambient shadows, and one
 * bold accent (crate orange) spent sparingly. */
/** @type {import('tailwindcss').Config} */
export default {
  darkMode: "media",
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: { DEFAULT: "#16181D", 700: "#1B1E25", 600: "#2B2F38" },
        paper: { DEFAULT: "#F6F4EF", 100: "#FFFFFF", 200: "#EFEBE2" },
        slate: { DEFAULT: "#C9C2B4", dark: "#2B2F38" },
        crate: { DEFAULT: "#E8622C", dark: "#F07440", 50: "#FDECE4" },
        manifest: { DEFAULT: "#3A5B8C", light: "#6C8FC7" },
        moss: { DEFAULT: "#3E8B63" },
      },
      fontFamily: {
        display: ["Archivo", "system-ui", "sans-serif"],
        body: ["Inter", "system-ui", "sans-serif"],
        mono: ["\"IBM Plex Mono\"", "ui-monospace", "monospace"],
      },
      borderRadius: {
        card: "8px",
        control: "6px",
        panel: "12px",
      },
      boxShadow: {
        // A hard-edged "lifted" shadow for a dragged card — a tactile,
        // physically-picked-up read, not a soft ambient blur.
        lift: "3px 3px 0 rgba(22, 24, 29, 0.18)",
      },
    },
  },
  plugins: [],
};
