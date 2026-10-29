# Staw — single-image build: the server binary plus the web bundle it
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
RUN cargo build --release -p staw-server
RUN cd crates/staw-web && trunk build --release --features api

FROM debian:bookworm-slim
# ca-certificates: the expert layer makes outbound TLS (model APIs);
# without a trust store the client panics at boot (found by the first
# real deployment).
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system staw
COPY --from=builder /src/target/release/staw-server /usr/local/bin/staw-server
COPY --from=builder /src/crates/staw-web/dist /srv/staw/web
USER staw
# In-container bind must be reachable from outside the container; publish
# it loopback-only on the host until auth lands (see compose.yaml).
ENV STAW_LISTEN=0.0.0.0:8080 \
    STAW_WEB__ASSETS=/srv/staw/web
EXPOSE 8080
ENTRYPOINT ["staw-server"]
