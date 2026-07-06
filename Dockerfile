# Converge — single-image build: the server binary plus the web bundle it
# serves same-origin. Queries compile against the committed .sqlx cache, so
# no database is needed at build time.

FROM rust:1.88-bookworm AS builder
ARG TRUNK_VERSION=0.21.14
RUN rustup target add wasm32-unknown-unknown \
    && curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-x86_64-unknown-linux-gnu.tar.gz" \
       | tar -xz -C /usr/local/bin trunk
WORKDIR /src
COPY . .
ENV SQLX_OFFLINE=true
RUN cargo build --release -p converge-server
RUN cd crates/converge-web && trunk build --release --features api

FROM debian:bookworm-slim
RUN useradd --system converge
COPY --from=builder /src/target/release/converge-server /usr/local/bin/converge-server
COPY --from=builder /src/crates/converge-web/dist /srv/converge/web
USER converge
# In-container bind must be reachable from outside the container; publish
# it loopback-only on the host until auth lands (see compose.yaml).
ENV CONVERGE_LISTEN=0.0.0.0:8080 \
    CONVERGE_WEB__ASSETS=/srv/converge/web
EXPOSE 8080
ENTRYPOINT ["converge-server"]
