FROM dockerhub.cloud1ful.com/library/debian:trixie-slim AS tokenizer-artifact

ADD --checksum=sha256:4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4 https://huggingface.cloud1ful.com/google/embeddinggemma-2/resolve/f9972ac22e696a105c482202646979e33e4b5a1e/tokenizer.json /tmp/tokenizer.json
RUN test "$(wc -c < /tmp/tokenizer.json)" -eq 32170510

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
COPY --from=tokenizer-artifact /tmp/tokenizer.json /var/lib/context69/tokenizers/4d777ef5bdc1aa36227abdfb77c3e49e7b9c892d16e1b6bda41c393504828be4.json
COPY THIRD_PARTY_NOTICES.md /usr/share/doc/context69/THIRD_PARTY_NOTICES.md
COPY LICENSE /usr/share/doc/context69/LICENSE

ENV RUST_LOG=context69=info,sqlx=warn
ENV CONTEXT69_API_BIND_ADDR=127.0.0.1:8096
ENV CONTEXT69_MCP_BIND_ADDR=0.0.0.0:8097
ENV CONTEXT69_CHUNKING__TOKENIZER_ARTIFACT_DIR=/var/lib/context69/tokenizers
# Writable cache of prepared pinned tokenizer presets. Mount this path
# persistently so a prepared preset survives a restart and stays usable
# offline; it is never the read-only artifact directory.
ENV CONTEXT69_CHUNKING__TOKENIZER_DOWNLOAD_DIR=/var/lib/context69/tokenizers.cache
RUN mkdir -p /var/lib/context69/tokenizers /var/lib/context69/tokenizers.cache

EXPOSE 80 8097

ENTRYPOINT ["tini", "--"]
CMD ["/usr/local/bin/start-context69.sh"]
