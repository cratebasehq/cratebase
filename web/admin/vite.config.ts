import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  // The server mounts the dashboard at `/_/` (PocketBase's path), so the
  // bundle's asset URLs have to be absolute under that prefix. The Vite
  // default of `/` emits `/assets/...`, which the server does not route —
  // the page then loads, renders nothing, and looks like a blank screen.
  // A relative `./` base is not an option either: the SPA has nested
  // routes, so `/_/collections/posts` would resolve `./assets/...`
  // against `/_/collections/`.
  base: "/_/",
  // Rolldown (this Vite version's bundler) computes each chunk's
  // content-hash placeholder non-deterministically across separate
  // build invocations - confirmed directly: two builds from byte-
  // identical source produced byte-identical chunk *content* but
  // different hash suffixes in the filename, which broke the "the
  // committed bundle must match a fresh build" CI check on every other
  // build regardless of source changes (see rolldown/rolldown's open
  // hash-related issues). The whole dashboard bundle ships embedded in
  // one versioned server binary (rust-embed, `crates/server/src/
  // dashboard.rs`) rather than deployed incrementally behind a CDN, so
  // there is no per-file long-term-caching need the hash was buying -
  // dropping it trades away nothing this deployment model uses, and
  // makes the build reproducible again.
  plugins: [react(), tailwindcss()],
  build: {
    // The `__vitePreload` polyfill chunk Vite injects for `<link
    // rel="modulepreload">`-unaware browsers had its own internal
    // non-determinism (two runtime-helper functions emitted in a
    // different order across otherwise-identical builds) independent
    // of the content-hash issue above. This dashboard only ever runs
    // in the browser the operator opens it in - there's no matrix of
    // legacy targets to support - so the polyfill buys nothing worth
    // keeping the remaining nondeterminism for.
    modulePreload: false,
    rollupOptions: {
      output: {
        entryFileNames: "assets/[name].js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/[name][extname]",
      },
    },
  },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
      "@cratebase/client": fileURLToPath(new URL("../../sdk/js/client/src/index.ts", import.meta.url)),
    },
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:8090",
    },
  },
});
