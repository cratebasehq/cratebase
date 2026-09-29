#!/usr/bin/env node
// Runs the local Cratebase server for development: `cratebase dev` creates
// the data directory, provisions a superuser, applies ./schema.json,
// seeds ./pb_seed/ on a first run, and keeps ./cratebase-types.d.ts up to
// date — all in one command. See https://cratebase.dev/docs/getting-started/first-run/.
//
// This wrapper's only job beyond that is to fail with a helpful message
// (instead of "command not found") when the `cratebase` binary isn't
// installed yet.
import { spawnSync } from "node:child_process";

function hasCratebase() {
  const result = spawnSync("cratebase", ["--version"], { stdio: "ignore" });
  return result.status === 0;
}

if (!hasCratebase()) {
  console.error(
    [
      "",
      "The `cratebase` binary isn't on your PATH.",
      "",
      "Install it with:",
      "",
      "  curl -fsSL https://cratebase.dev/install.sh | sh",
      "",
      "Then re-run `dev` (this installs to ~/.local/bin — make sure that's on your PATH).",
      "Docs: https://cratebase.dev/docs/getting-started/install/",
      "",
    ].join("\n"),
  );
  process.exit(1);
}

const result = spawnSync("cratebase", ["dev"], { stdio: "inherit" });
process.exit(result.status ?? 1);
