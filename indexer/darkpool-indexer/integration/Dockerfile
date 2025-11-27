# === Chef === #
FROM --platform=arm64 rust:1.89-trixie AS chef

# Create a build dir and add local dependencies
WORKDIR /build

COPY ./rust-toolchain ./rust-toolchain
RUN cat rust-toolchain | xargs rustup toolchain install

# Install cargo-chef
RUN cargo install cargo-chef

# === Sources === #
FROM chef AS sources
WORKDIR /build
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./indexer ./indexer

# === Builder === #
# Pull the sources into their own layer
FROM chef AS builder

# Add build argument for cargo features
ARG CARGO_FEATURES

# Disable compiler warnings and enable backtraces for panic debugging
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1
ENV CARGO_HTTP_CHECK_REVOKE=false

COPY --from=sources /build /build
WORKDIR /build

# Install protoc, openssl, and pkg-config
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev libclang-dev libpq-dev ca-certificates

# Update Cargo.toml to include only "indexer/darkpool-indexer" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["indexer/darkpool-indexer"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin darkpool-indexer

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --tests --features "integration" --recipe-path recipe.json

COPY --from=sources /build/indexer /build/indexer
WORKDIR /build

RUN cargo build -p darkpool-indexer --test integration --features "integration"

# Copy in the deployments file from the Anvil image
COPY --from=anvil /deployments.json /deployments.json
