#!/usr/bin/env sh
# Vendor the film's fonts into assets/fonts/ (gitignored, see .gitignore)
# so every render is offline and deterministic — index.html loads them via
# @font-face from here, never from a font CDN at render time.
#   Bricolage Grotesque (all axes) + Red Hat Mono — brand (site/src/styles/tokens.css)
#   IBM Plex Sans + JetBrains Mono — dashboard (web/admin/src/index.css)
#   Inter + Archivo — examples/team-board (tailwind.config.js)
set -eu
cd "$(dirname "$0")"
mkdir -p assets/fonts
CDN=https://cdn.jsdelivr.net/fontsource/fonts
for spec in \
  "bricolage-grotesque:vf@latest/latin-standard-normal bricolage-grotesque-latin-standard-normal" \
  "red-hat-mono:vf@latest/latin-wght-normal red-hat-mono-latin-wght-normal" \
  "ibm-plex-sans:vf@latest/latin-wght-normal ibm-plex-sans-latin-wght-normal" \
  "jetbrains-mono:vf@latest/latin-wght-normal jetbrains-mono-latin-wght-normal" \
  "inter:vf@latest/latin-wght-normal inter-latin-wght-normal" \
  "archivo:vf@latest/latin-wght-normal archivo-latin-wght-normal"; do
  set -- $spec
  [ -s "assets/fonts/$2.woff2" ] || curl -fsSL -o "assets/fonts/$2.woff2" "$CDN/$1.woff2"
done
ls -la assets/fonts
