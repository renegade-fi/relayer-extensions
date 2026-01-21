# === Chef === #
FROM --platform=arm64 rust:1.92-trixie AS chef

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
COPY ./prover-service ./prover-service

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

# Update Cargo.toml to include only "prover-service" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["prover-service/server"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin prover-service

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY --from=sources /build/prover-service /build/prover-service
WORKDIR /build

RUN cargo build --release -p prover-service --features "$CARGO_FEATURES"

# === Release stage === #
FROM --platform=arm64 debian:trixie-slim
RUN apt-get update && \
    apt-get install -y libssl-dev ca-certificates libpq-dev

# Copy the binary from the build stage
COPY --from=builder /build/target/release/prover-service /bin/prover-service

ENV RUST_LOG="info,warp::filters::trace=warn"
ENTRYPOINT ["/bin/prover-service"]
