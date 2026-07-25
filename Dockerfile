FROM rust:1.97.1-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets
RUN cargo build --locked --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/redroid-helper /usr/local/bin/redroid-helper

ENV DOCKER_HOST=unix:///var/run/docker.sock
ENTRYPOINT ["redroid-helper"]

