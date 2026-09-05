// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import sitemap from "@astrojs/sitemap";

// Pure static output — no adapter. Cloudflare Pages deploys this as
// plain static files (see local://landing-docs-design-direction.md §5).
export default defineConfig({
  site: "https://cratebase.dev",
  integrations: [
    starlight({
      title: "Cratebase",
      description:
        "A fast, self-hostable backend with dynamic collections, auth, files, and realtime, in one Rust binary.",
      customCss: ["./src/styles/starlight-theme.css"],
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/cratebasehq/cratebase",
        },
      ],
      // All Starlight-generated pages are nested under content/docs/docs/
      // so their routes land at /docs/... while src/pages/index.astro
      // keeps the plain "/" for the marketing landing page. This is the
      // documented workaround for a subpath-mounted Starlight instance;
      // see manual-setup.mdx in the Starlight repo.
      sidebar: [
        {
          label: "Getting started",
          items: [{ autogenerate: { directory: "docs/getting-started" } }],
        },
        {
          label: "Concepts",
          items: [
            { label: "Collections", slug: "docs/concepts/collections" },
            { label: "Fields", slug: "docs/concepts/fields" },
            { label: "Records API", slug: "docs/concepts/records-api" },
            { label: "API rules", slug: "docs/concepts/api-rules" },
            { label: "Filter syntax", slug: "docs/concepts/filter-syntax" },
            { label: "Batch API", slug: "docs/concepts/batch-api" },
            { label: "Realtime", slug: "docs/concepts/realtime" },
            { label: "Files", slug: "docs/concepts/files" },
            {
              label: "Authentication",
              items: [{ autogenerate: { directory: "docs/concepts/authentication" } }],
            },
          ],
        },
        {
          label: "Extending",
          items: [{ autogenerate: { directory: "docs/extending" } }],
        },
        {
          label: "AI",
          items: [{ autogenerate: { directory: "docs/ai" } }],
        },
        {
          label: "Dashboard",
          items: [{ autogenerate: { directory: "docs/dashboard" } }],
        },
        {
          label: "Deploy & operate",
          items: [{ autogenerate: { directory: "docs/deploy" } }],
        },
        {
          label: "Migrating from PocketBase",
          items: [{ autogenerate: { directory: "docs/migrating" } }],
        },
        {
          label: "Reference",
          items: [{ autogenerate: { directory: "docs/reference" } }],
        },
        {
          label: "Benchmarks",
          slug: "docs/benchmarks",
        },
        {
          label: "Project",
          items: [{ autogenerate: { directory: "docs/project" } }],
        },
      ],
    }),
    sitemap(),
  ],
});
