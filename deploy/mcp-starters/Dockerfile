# syntax=docker/dockerfile:1.7
# Panda + Node/npm for npx-driven MCP servers (stdio). Build from repository root.
FROM rust:1.86-bookworm AS builder
WORKDIR /work
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
ARG PANDA_BUILD_FEATURES="mimalloc"
RUN cargo build --release -p panda-server --features "${PANDA_BUILD_FEATURES}"

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates nodejs npm \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /work/target/release/panda /usr/local/bin/panda
COPY deploy/mcp-starters/panda.mcp-starter.yaml /app/panda.yaml

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/panda"]
CMD ["/app/panda.yaml"]
