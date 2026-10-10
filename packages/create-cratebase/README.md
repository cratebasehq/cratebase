# create-cratebase

Scaffold a new app on [Cratebase](https://cratebase.dev), a self-hostable Rust
backend, in one command.

```sh
bun create cratebase my-app
# or: npm create cratebase@latest my-app
# or: pnpm create cratebase my-app
# or: yarn create cratebase my-app
```

Interactive by default (project name, template, package manager, whether to
install dependencies and run `git init`), or fully non-interactive:

```sh
bun create cratebase my-app --template nextjs --pm bun --yes
```

```
Usage:
  bun create cratebase <project-directory> [options]

Options:
  -t, --template <name>   Template: nextjs, vite-react, tanstack-start, expo
      --pm <name>         Package manager: bun, npm, pnpm, yarn (default: the one you ran this with)
  -y, --yes               Skip prompts, accept defaults for anything unset
      --force             Empty the target directory if it isn't empty (keeps .git)
      --no-install        Skip installing dependencies
      --no-git            Skip `git init`
  -h, --help              Print this help
  -v, --version           Print the CLI version
```

Without a terminal (CI, pipes) the prompts are skipped as if `--yes` was passed.
A non-empty target directory is never touched unless you confirm in the prompt
or pass `--force`.

## Templates

- **`nextjs`** — Next.js (App Router), TypeScript, Tailwind. Full feature
  set: password/OTP/magic-link/OAuth auth with TOTP challenge handling, a
  protected realtime dashboard, file upload, full-text search, a
  notifications bell, and an SSR-safe `CratebaseProvider` boundary.
- **`vite-react`** — Vite + React, TypeScript, Tailwind. The same feature
  set as `nextjs`, as a client-only SPA (`react-router-dom`).
- **`tanstack-start`** — TanStack Start on Cloudflare Workers, TypeScript,
  Tailwind. Server-rendered pages with HttpOnly cookie sessions via
  `@cratebase/start` (sign-in/up/out server functions, `authMiddleware`, a
  public feed and a guarded dashboard).
- **`expo`** — Expo Router, TypeScript. Kept lean: password + email-code
  (OTP) sign-in, `AsyncAuthStore`, and a realtime notes list. Its README
  documents how to add magic link, OAuth, TOTP, file upload, search, and
  notifications with the same `@cratebase/client`/`@cratebase/react` calls
  the web templates use.

Every template ships a `schema.json`, `cb_seed/` demo data,
`cb_hooks/welcome.cb.js`, `.env.example`, and a `dev` script that runs
`cratebase dev` and the frontend together — see each template's own
README for its 60-second quickstart and deploy notes. Full comparison:
[Starter kits](https://cratebase.dev/docs/getting-started/starter-kits/)
on the docs site.

## Development

This package is part of the Cratebase repo's root bun workspace.

```sh
bun install
bun run --cwd packages/create-cratebase typecheck
bun run --cwd packages/create-cratebase test    # bun test
bun run --cwd packages/create-cratebase build   # emit dist/
bun run --cwd packages/create-cratebase dev -- /tmp/my-app --template nextjs --yes
```

`bun run create-cratebase:check` from the repo root runs typecheck, test,
and build in one command (also wired into CI, alongside a matrix job that
scaffolds each template and runs its own typecheck/build).

## Versioning

This package versions independently from the `cratebase` server binary and
from the SDK packages (`@cratebase/client`, `@cratebase/react`, `@cratebase/start`) — it's a
scaffolding CLI, not a build artifact of the Rust workspace or a runtime
dependency of anything else. Publishing is triggered by pushing a
`create-cratebase-v<version>` tag (see
`.github/workflows/publish-create-cratebase.yml`), kept separate from the
server's own `v*` tags and the SDK packages' `client-v*`/`react-v*`/
`start-v*`/`schema-v*`, so none of these release trains block on each other.
