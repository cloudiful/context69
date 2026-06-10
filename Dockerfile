FROM oven/bun:1 AS frontend-build

WORKDIR /app/frontend
ARG TARGETARCH

COPY frontend/package.json frontend/bun.lock ./
RUN --mount=type=cache,target=/root/.cache/bun,sharing=locked \
    --mount=type=cache,target=/root/.bun/install/cache,sharing=locked \
    bun install --frozen-lockfile

COPY frontend ./
RUN bun run build

FROM rust:1-bookworm AS build

WORKDIR /app
ARG TARGETARCH

COPY Cargo.toml Cargo.lock ./
COPY crates/context69-contracts/Cargo.toml crates/context69-contracts/Cargo.toml
COPY crates/context69-sdk/Cargo.toml crates/context69-sdk/Cargo.toml

RUN mkdir -p src crates/context69-contracts/src crates/context69-sdk/src \
    && printf 'fn main() {}\n' > src/main.rs \
    && printf '\n' > src/lib.rs \
    && printf '\n' > crates/context69-contracts/src/lib.rs \
    && printf '\n' > crates/context69-sdk/src/lib.rs

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release

COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY src src
COPY migrations migrations

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release \
    && install -Dm755 target/release/context69 /app/out/context69

FROM debian:bookworm-slim AS runtime
ARG TARGETARCH

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates nginx tini \
    && rm -f /etc/nginx/sites-enabled/default \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=build /app/out/context69 /usr/local/bin/context69
COPY --from=frontend-build /app/frontend/dist /app/frontend/dist
COPY docker/nginx-context69.conf /etc/nginx/conf.d/default.conf
COPY --chmod=755 docker/start-context69.sh /usr/local/bin/start-context69.sh

ENV RUST_LOG=context69=info,sqlx=warn
ENV CONTEXT69_API_BIND_ADDR=127.0.0.1:8096
ENV CONTEXT69_MCP_BIND_ADDR=0.0.0.0:8097

EXPOSE 80 8097

ENTRYPOINT ["tini", "--"]
CMD ["/usr/local/bin/start-context69.sh"]
