# syntax=docker/dockerfile:1
#
# The Copilot image: the `server` and `migrate` binaries plus the web bundle at /app/web, which
# the server serves (WEB_DIR). Build from the repository root:
#   docker build -t copilot .
# deploy/compose.yml runs it with Postgres; see docs/deployment-container.md.
#
# Each base image is pinned by digest (the tag says what it is, the digest what is used), so a
# rebuild never picks up a new upstream image unseen; Dependabot proposes digest updates.

# Web bundle (Node and pnpm). The client calls the API with relative `/api` URLs.
FROM node:22-slim@sha256:be979e5523c76fa4d5ca4ebd3099078f220aa47ec39c48ad5b5df9314debd6eb AS web
WORKDIR /app
RUN corepack enable
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.json biome.json ./
COPY apps/web/package.json apps/web/
COPY packages/contracts/package.json packages/contracts/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --ignore-scripts
COPY apps/web apps/web
COPY packages/contracts packages/contracts
COPY scripts/compress-web.mjs scripts/
RUN pnpm --filter @copilot/web build && node scripts/compress-web.mjs apps/web/dist

# Rust binaries (the `dist` profile: release with fat LTO, one codegen unit, and no symbols).
FROM rust:1-slim-bookworm@sha256:bb875b0f8ce726053e259034848c471840e96bfe591fd52556178ae9a05e6126 AS rust
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --profile dist --locked -p copilot-api -p copilot-database \
    && mkdir -p /out \
    && cp target/dist/server target/dist/migrate /out/

# Runtime: distroless (glibc, libgcc, CA roots for providers and tool servers, tzdata; no shell
# or package manager), as its unprivileged `nonroot` user.
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f AS runtime

COPY --from=rust /out/ /usr/local/bin/
COPY --from=web /app/apps/web/dist /app/web

USER nonroot
# Buffers of 1 MiB and more (an upload's bytes) come from mmap and go back to the system when
# freed, so the server shrinks to its idle size after a burst.
ENV HOST=0.0.0.0 \
    PORT=3000 \
    WEB_DIR=/app/web \
    MALLOC_MMAP_THRESHOLD_=1048576
EXPOSE 3000

# `migrate` (rustauth's `auth` migrations, then Copilot's) runs once before `server`; the compose
# file's `migrate` service does that on every `docker compose up`.
CMD ["server"]
