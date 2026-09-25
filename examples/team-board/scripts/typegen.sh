#!/usr/bin/env bash
# Regenerates src/cratebase-types.d.ts from the local data directory.
# `bun run dev` (scripts/dev.ts) also starts the server with `--dev` and
# `CB_TYPEGEN_OUT` set, so schema edits made live through the dashboard
# keep this file in sync without re-running this script by hand.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

CRATEBASE_BIN="${CRATEBASE_BIN:-cratebase}"
CB_DATA_DIR="${CB_DATA_DIR:-./pb_data}"

"$CRATEBASE_BIN" typegen -o src/cratebase-types.d.ts --dir "$CB_DATA_DIR"
