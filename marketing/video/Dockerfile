# Playwright + ffmpeg, pinned to the Playwright version in package.json so
# the browser build matches the system libs baked into this base image —
# this machine's own Chromium is missing system libs and there's no sudo,
# so every capture and every render happens in here, never on the host.
#
# Used for two things (see render.sh):
#   1. capture/capture.mjs  — screenshot the real dashboard (needs
#      --network host to reach the cratebase dev server on localhost)
#   2. render.mjs            — walk window.seek(t) and pipe frames to ffmpeg
FROM mcr.microsoft.com/playwright:v1.48.0-jammy

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY package.json ./
RUN npm install --omit=dev

COPY . .

CMD ["node", "render.mjs"]
