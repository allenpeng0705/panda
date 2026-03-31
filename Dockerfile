# syntax=docker/dockerfile:1.7

FROM rust:1.86-bookworm AS builder
WORKDIR /work

# Prime dependency build cache first.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p panda-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /work/target/release/panda /usr/local/bin/panda
COPY panda.example.yaml /app/panda.yaml

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/panda"]
CMD ["/app/panda.yaml"]
