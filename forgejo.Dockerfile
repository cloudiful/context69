FROM dockerhub.cloud1ful.com/library/debian:trixie-slim AS runtime
ARG TARGETARCH

COPY docker/apt-cloud1ful-insecure.conf /etc/apt/apt.conf.d/99apt-cloud1ful-insecure
COPY docker/debian.sources /etc/apt/sources.list.d/debian.sources

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates nginx tini \
    && rm -f /etc/nginx/sites-enabled/default \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --chmod=755 ci-image-input/context69 /usr/local/bin/context69
COPY ci-image-input/frontend-dist /app/frontend/dist
COPY docker/nginx-context69.conf /etc/nginx/conf.d/default.conf
COPY --chmod=755 docker/start-context69.sh /usr/local/bin/start-context69.sh

ENV RUST_LOG=context69=info,sqlx=warn
ENV CONTEXT69_API_BIND_ADDR=127.0.0.1:8096
ENV CONTEXT69_MCP_BIND_ADDR=0.0.0.0:8097

EXPOSE 80 8097

ENTRYPOINT ["tini", "--"]
CMD ["/usr/local/bin/start-context69.sh"]
