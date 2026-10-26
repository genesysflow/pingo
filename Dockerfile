# syntax=docker/dockerfile:1
# Pingo in one image: the server (chat, auth, storage and the media server), the web app,
# and the browser noise suppressor. See docs/SELF_HOSTING.md.

FROM rust:1-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends cmake clang unzip \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
COPY --from=oven/bun:1 /usr/local/bin/bun /usr/local/bin/bun
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p pingo-server \
 && cp target/release/pingo-server /usr/local/bin/pingo-server
# `bun run build` also compiles crates/pingo-denoise to WebAssembly.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cd apps/web \
 && bun install --frozen-lockfile \
 && bun run build \
 && cp -r dist /web

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data pingo \
 && mkdir -p /data && chown pingo:pingo /data
COPY --from=build /usr/local/bin/pingo-server /usr/local/bin/pingo-server
COPY --from=build /web /app/web
ENV PINGO_WEB_DIR=/app/web \
    PINGO_UDP_PORT=40000 \
    RUST_LOG=info
USER pingo
WORKDIR /data
VOLUME /data
# HTTP (web app, API, WebSockets) and the media server's single UDP port.
EXPOSE 8090/tcp 40000/udp
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD ["pingo-server", "health"]
ENTRYPOINT ["pingo-server"]
CMD ["serve", "--http", "0.0.0.0:8090", "--dir", "/data"]
