#!/usr/bin/env bash
# Every Playwright step runs in the Docker image (host Chromium lacks libs).
# Source is mounted at /work and the container runs as the host user, so
# code edits need no image rebuild and out/ stays user-owned.
#
#   ./render.sh build                          # build the image (once)
#   ./render.sh stills [--w 1080 --h 1920] t1 t2 ...   # PNGs -> out/stills/
#   ./render.sh node <script.mjs> [args...]    # run any script in the image
#   ./render.sh master [16x9|9x16] [fps]       # full render -> out/master-*.mp4
#   ./render.sh all                            # masters + audio + every deliverable (see README)
set -euo pipefail
cd "$(dirname "$0")"
IMAGE=cb-video-engine
dk() { docker run --rm --init --ipc=host --user "$(id -u):$(id -g)" -e HOME=/tmp -v "$(pwd):/work" -w /work "$IMAGE" "$@"; }
case "${1:-}" in
  build) docker build -t "$IMAGE" -f Dockerfile . ;;
  stills) shift; dk node stills.mjs "$@" ;;
  node) shift; dk node "$@" ;;
  master)
    fmt="${2:-16x9}"; fps="${3:-60}"
    if [ "$fmt" = "9x16" ]; then wh="--w 1080 --h 1920"; else wh="--w 1920 --h 1080"; fi
    dk node render.mjs $wh --fps "$fps" --workers "${WORKERS:-4}" --out "out/master-$fmt.mp4" ;;
  all) ./render.sh master 16x9 60 && ./render.sh master 9x16 60 && ./encode.sh ;;
  *) sed -n 2,11p "$0"; exit 1 ;;
esac
