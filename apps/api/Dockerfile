# syntax=docker/dockerfile:1
#
# API image with the `server` and `migrate` binaries. Build from the repository root:
#   docker build -f apps/api/Dockerfile -t copilot-api .
# See docs/deployment-container.md.

FROM rust:1-slim-bookworm AS build
WORKDIR /app

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked -p copilot-api -p copilot-database \
    && mkdir -p /out \
    && cp target/release/server target/release/migrate /out/

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home app

COPY --from=build /out/ /usr/local/bin/

USER app
ENV HOST=0.0.0.0 \
    PORT=3000
EXPOSE 3000

# Run `migrate` as a one-off task before rolling out a new version.
CMD ["server"]
