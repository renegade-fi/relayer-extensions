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
COPY ./compliance ./compliance

# === Builder === #
# Pull the sources into their own layer
FROM chef AS builder

# Disable compiler warnings and enable backtraces for panic debugging
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1

COPY --from=sources /build /build
WORKDIR /build

# Install protoc, openssl, and pkg-config
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev libclang-dev libpq-dev ca-certificates

# Update Cargo.toml to include only "compliance-server" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["compliance/compliance-server"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin compliance-server

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY --from=sources /build/compliance /build/compliance
WORKDIR /build

RUN cargo build --release -p compliance-server

# === Release stage === #
FROM --platform=arm64 debian:trixie-slim

RUN apt-get update && \
    apt-get install -y libssl-dev && \
    apt-get install -y ca-certificates && \
    apt-get install -y libpq-dev

# Copy the binary from the build stage
COPY --from=builder /build/target/release/compliance-server /bin/compliance-server
CMD ["compliance-server"]
