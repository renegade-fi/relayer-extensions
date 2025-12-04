# === Chef === #
FROM --platform=arm64 rust:1.89-trixie AS chef

# Create a build dir and add local dependencies
WORKDIR /build

# Install protoc, openssl, and pkg-config
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev libclang-dev libpq-dev ca-certificates

COPY ./rust-toolchain ./rust-toolchain
RUN cat rust-toolchain | xargs rustup toolchain install

# Install cargo-chef
RUN cargo install cargo-chef

# === Planner === #
FROM chef AS planner

# Disable compiler warnings and enable backtraces for panic debugging
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1
ENV CARGO_HTTP_CHECK_REVOKE=false

WORKDIR /build

# Copy only the files needed for recipe generation
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./indexer/darkpool-indexer/Cargo.toml ./indexer/darkpool-indexer/
COPY ./indexer/darkpool-indexer/src/main.rs ./indexer/darkpool-indexer/src/
COPY ./indexer/darkpool-indexer/src/lib.rs ./indexer/darkpool-indexer/src/
COPY ./indexer/darkpool-indexer-api/Cargo.toml ./indexer/darkpool-indexer-api/
COPY ./indexer/darkpool-indexer-api/src/lib.rs ./indexer/darkpool-indexer-api/src/

# Update Cargo.toml to include only "indexer/darkpool-indexer" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["indexer/darkpool-indexer"]' Cargo.toml

# Generate the recipe
RUN cargo chef prepare --recipe-path recipe.json --bin darkpool-indexer

# === Builder === #
FROM chef AS builder
WORKDIR /build

# Set the same environment variables as planner
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1
ENV CARGO_HTTP_CHECK_REVOKE=false

# Copy only the recipe from the planner stage
COPY --from=planner /build/recipe.json recipe.json

# Build dependencies
RUN cargo chef cook --tests --features "integration" --recipe-path recipe.json

# Now copy the actual source code
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./indexer ./indexer

# Update Cargo.toml again (same modification as in planner)
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["indexer/darkpool-indexer"]' Cargo.toml

# Build the indexer binary
RUN cargo build -p darkpool-indexer --test integration --features "integration"

# Copy in the deployments file from the Anvil image
COPY --from=anvil /deployments.json /deployments.json

# Create a non-root user for running tests
RUN useradd -m -u 1000 testuser

# Change ownership of the build directory to the test user
RUN chown -R testuser:testuser /build

# Switch to non-root user w/ sufficient permissions to run tests
USER testuser