# syntax=docker/dockerfile:1

# ---- frontend build -----------------------------------------------------
# Builds the admin dashboard's static assets, which get embedded straight
# into the Rust binary in the next stage (rust-embed). The dashboard talks
# to Cratebase with the official `pocketbase` npm client — no in-house SDK
# to build here anymore.
FROM oven/bun:1-slim AS frontend
WORKDIR /app
COPY package.json bun.lock ./
COPY web/admin web/admin
COPY web/email web/email
RUN bun install && bun run admin:build && bun run email:build

# ---- deps cache layer -------------------------------------------------
# Copies only the manifests first so `cargo build` for dependencies is
# cached across rebuilds that only touch application source.
FROM rust:1-slim-bookworm AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY crates/filter/Cargo.toml crates/filter/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/storage/Cargo.toml crates/storage/Cargo.toml
COPY crates/auth/Cargo.toml crates/auth/Cargo.toml
COPY crates/mailer/Cargo.toml crates/mailer/Cargo.toml
COPY crates/server/Cargo.toml crates/server/Cargo.toml
# rust-embed's `#[folder = "web/admin/dist"]` and `#[folder = "web/email/dist"]` just
# need the directories to exist at compile time for this dependency-only
# stub build - not real content, not even a `.gitkeep` placeholder that
# keeps a directory present in git. Depending on such a file via COPY
# was fragile: `bun run admin:build` (run by the `frontend` stage right
# above, or by any local dev build) deletes it from the working tree
# before writing real output, so a `docker compose build` run right
# after a local build - which reads the host filesystem, not git
# history - failed here even though the placeholder was fine in the
# actual commit.
RUN mkdir -p crates/core/src crates/filter/src crates/db/src crates/storage/src crates/auth/src crates/mailer/src crates/server/src web/admin/dist web/email/dist \
    && for c in core filter db storage auth mailer; do echo "fn _stub() {}" > crates/$c/src/lib.rs; done \
    && echo "fn main() {}" > crates/server/src/main.rs \
    && cargo build --release --workspace 2>/dev/null || true

# ---- builder ------------------------------------------------------------
FROM planner AS builder
COPY crates crates
COPY Cargo.toml Cargo.lock ./
COPY --from=frontend /app/web/admin/dist web/admin/dist
COPY --from=frontend /app/web/email/dist web/email/dist
# Touch sources so cargo doesn't skip the real build using the stub mtimes.
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release -p cratebase-server

# ---- runtime --------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 cratebase
WORKDIR /app
COPY --from=builder /app/target/release/cratebase /usr/local/bin/cratebase
RUN mkdir -p /app/data && chown -R cratebase:cratebase /app
USER cratebase
ENV CRATEBASE_DATA_DIR=/app/data
EXPOSE 8090
ENTRYPOINT ["cratebase"]
CMD ["serve"]
