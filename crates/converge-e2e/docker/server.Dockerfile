# The Converge server under test, built from the checked-out workspace.
# Kept apart from the agent image so every agent the harness grows does
# not carry a Rust toolchain build of the server.

FROM rust:1.88-bookworm AS converge-builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY .sqlx .sqlx
COPY crates crates

ENV SQLX_OFFLINE=true
RUN cargo build --release -p converge-server

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=converge-builder /src/target/release/converge-server /usr/local/bin/converge-server

EXPOSE 8080
CMD ["converge-server"]
