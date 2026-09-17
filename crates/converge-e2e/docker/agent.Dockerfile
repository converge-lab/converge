# One Dockerfile for every agent tool. What differs — the base image and
# the command that installs the CLI — arrives as build arguments, so a
# second agent is a second set of arguments rather than a second file.
#
# Installing at build time rather than at container start keeps it in
# Docker's layer cache: a test run does not reinstall the CLI, and does
# not depend on a package registry being reachable.

ARG BASE_IMAGE

# Converge's own CLI, built from the checkout: the suite exists to
# exercise this branch's code, so it must not install a published
# release. Kept in this image because `converge` runs where the agent
# runs — the server keeps its own build in server.Dockerfile.
FROM rust:1.88-bookworm AS converge-builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release -p converge-cli

FROM ${BASE_IMAGE}

ARG AGENT_INSTALL

# npm-shaped for now: a global prefix inside the unprivileged user's home
# so the install needs no root. A non-npm agent will have to lift this
# into the arguments too.
ENV NPM_CONFIG_PREFIX=/home/e2e/.npm-global \
    PATH=/home/e2e/.npm-global/bin:${PATH}

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates coreutils curl git gzip tar \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /bin/sh e2e \
    && mkdir /workspace \
    && chown e2e:e2e /workspace

USER e2e
RUN sh -c "${AGENT_INSTALL}"

# Last, and as root: this binary changes with every commit, so it must
# not sit below the agent install — otherwise every source edit re-pulls
# the agent from its package registry.
USER root
COPY --from=converge-builder /src/target/release/converge /usr/local/bin/converge

USER e2e
WORKDIR /workspace

CMD ["sleep", "infinity"]
