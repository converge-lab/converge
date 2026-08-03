FROM rust:1.94-bookworm AS sqlx

RUN cargo install sqlx-cli --version 0.9.0 --no-default-features --features postgres,rustls

FROM postgres:16-bookworm

COPY --from=sqlx /usr/local/cargo/bin/sqlx /usr/local/bin/sqlx
COPY crates/converge-storage-postgres/migrations /migrations
COPY crates/converge-e2e/docker/migrate.sh /docker-entrypoint-initdb.d/10-migrate.sh
COPY crates/converge-e2e/docker/fixtures.sql /docker-entrypoint-initdb.d/20-fixtures.sql
