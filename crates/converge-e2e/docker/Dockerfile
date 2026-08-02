FROM rust:1.88-bookworm AS converge-builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .sqlx .sqlx
COPY crates crates

ENV SQLX_OFFLINE=true
RUN cargo build --release -p converge-server

FROM node:22-bookworm-slim

ARG CLAUDE_CODE_VERSION

ENV DISABLE_AUTOUPDATER=1 \
    NPM_CONFIG_PREFIX=/home/e2e/.npm-global \
    PATH=/home/e2e/.npm-global/bin:${PATH}

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates coreutils curl git gzip tar \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /bin/sh e2e \
    && mkdir /workspace \
    && chown e2e:e2e /workspace

USER e2e
RUN npm install --global "@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}" \
    && npm cache clean --force

USER root
COPY --from=converge-builder /src/target/release/converge-server /usr/local/bin/converge-server

USER e2e
WORKDIR /workspace

EXPOSE 8080
CMD ["sleep", "infinity"]
