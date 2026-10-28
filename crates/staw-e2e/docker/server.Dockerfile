# The Staw server under test, built from the checked-out workspace.
# Kept apart from the agent image so every agent the harness grows does
# not carry a Rust toolchain build of the server.

FROM rust:1.88-bookworm AS staw-builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .sqlx .sqlx
COPY crates crates

ENV SQLX_OFFLINE=true
RUN cargo build --release -p staw-server

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=staw-builder /src/target/release/staw-server /usr/local/bin/staw-server

EXPOSE 8080
CMD ["staw-server"]
