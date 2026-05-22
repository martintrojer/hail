# syntax=docker/dockerfile:1.7

# Multi-stage image for hail.  Both deployable hail binaries are shipped in the
# same runtime image; select the process by overriding the command:
#   API (default):    podman run --rm hail:latest
#   Worker variant:  podman run --rm hail:latest /usr/local/bin/hail-worker
# Docker accepts the same form when building through the Dockerfile symlink.

ARG CARGO_CHEF_VERSION=0.1.72

FROM rust:1.95-slim-bookworm AS rust-builder
ARG CARGO_CHEF_VERSION
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        pkg-config \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install cargo-chef --version "${CARGO_CHEF_VERSION}" --locked

FROM rust-builder AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM rust-builder AS cacher
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --workspace --bins --recipe-path recipe.json

FROM rust-builder AS builder
COPY --from=cacher /app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
COPY . .
RUN cargo build --release --workspace --bins \
    && strip target/release/hail-api target/release/hail-worker \
    && install -Dm0755 target/release/hail-api /usr/local/bin/hail-api \
    && install -Dm0755 target/release/hail-worker /usr/local/bin/hail-worker

FROM node:24-bookworm-slim AS webapp-builder
WORKDIR /app/webapp
COPY webapp/package*.json ./
RUN npm ci
COPY webapp/ ./
RUN npm run build

FROM debian:bookworm-slim AS config-builder
WORKDIR /src
COPY . .
RUN mkdir -p /out/etc/hail \
    && if [ -f deploy/hail.example.toml ]; then \
        cp deploy/hail.example.toml /out/etc/hail/hail.example.toml; \
    else \
        printf '%s\n' '# Example hail config placeholder.' '# deploy/hail.example.toml was not present in this source revision.' > /out/etc/hail/hail.example.toml; \
    fi

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
COPY --from=builder /usr/local/bin/hail-api /usr/local/bin/hail-api
COPY --from=builder /usr/local/bin/hail-worker /usr/local/bin/hail-worker
COPY --from=webapp-builder /app/webapp/dist /srv/hail/webapp
COPY --from=config-builder /out/etc/hail/hail.example.toml /etc/hail/hail.example.toml
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT []
CMD ["/usr/local/bin/hail-api"]
