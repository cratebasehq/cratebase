#!/usr/bin/env bash
# Thin wrapper around the Docker image for the three things this project
# renders. See README.md for the full pipeline (capture, seeding, the
# critique loop) — this script only covers the "render something" step
# once capture/shots/ already has real screenshots in it.
#
# Usage:
#   ./render.sh build                 # build/rebuild the Docker image
#   ./render.sh stills                # one PNG per beat -> out/stills/
#   ./render.sh animatic [fps]        # full timeline, low fps, for critique (default 8)
#   ./render.sh final                 # full 30fps, sub=4 motion-blur render
#   ./render.sh clean                 # remove root-owned out/ contents (see README "Docker writes out/ as root")

set -euo pipefail
cd "$(dirname "$0")"

IMAGE=cb-video-engine

case "${1:-}" in
  build)
    docker build -t "$IMAGE" -f Dockerfile .
    ;;
  stills)
    mkdir -p out
    docker run --rm -v "$(pwd)/out:/app/out" "$IMAGE" node stills.mjs
    ;;
  animatic)
    mkdir -p out
    fps="${2:-8}"
    docker run --rm -v "$(pwd)/out:/app/out" "$IMAGE" node render.mjs --fps "$fps" --out out/animatic.mp4
    ;;
  final)
    mkdir -p out
    docker run --rm -v "$(pwd)/out:/app/out" "$IMAGE" \
      node render.mjs --fps 30 --sub 4 --out out/cratebase-launch-16x9.mp4
    ;;
  clean)
    docker run --rm -v "$(pwd)/out:/shots" "$IMAGE" sh -c 'rm -rf /shots/*'
    ;;
  *)
    echo "Usage: $0 {build|stills|animatic [fps]|final|clean}" >&2
    exit 1
    ;;
esac
