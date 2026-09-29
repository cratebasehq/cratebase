# Playwright + ffmpeg, pinned to the Playwright version in package.json so
# the browser build matches the system libs baked into this base image —
# the host's own Chromium is missing system libs and there's no sudo, so
# every capture and render happens in here.
#
# Source is NOT baked in: render.sh mounts marketing/video at /work and runs
# as the host user, so code edits need no rebuild and out/ stays user-owned.
# Dependencies live at /node_modules, which Node's resolver reaches by
# walking up from /work.
FROM mcr.microsoft.com/playwright:v1.48.0-jammy

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/deps
COPY package.json ./
RUN npm install --omit=dev && ln -s /opt/deps/node_modules /node_modules

WORKDIR /work
CMD ["node", "render.mjs"]
