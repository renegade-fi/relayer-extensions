# === Chef === #
FROM --platform=arm64 rust:1.89-trixie AS chef

# Create a build dir and add local dependencies
WORKDIR /build

# Build the rust toolchain before adding any dependencies; this is the slowest
# step and we would like to cache it before anything else
COPY ./rust-toolchain ./rust-toolchain
RUN cat rust-toolchain | xargs rustup toolchain install

# Install cargo-chef
RUN cargo install cargo-chef

# === Sources === #
FROM chef AS sources
WORKDIR /build
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./price-reporter ./price-reporter

# === Builder === #
FROM chef AS builder

# Install protoc, openssl, and pkg-config
RUN apt-get update && \
    apt-get install -y pkg-config protobuf-compiler libssl-dev libclang-dev

# Disable compiler warnings and enable backtraces for panic debugging
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1
ENV CARGO_HTTP_CHECK_REVOKE=false

COPY --from=sources /build /build
WORKDIR /build

# Update Cargo.toml to include only "price-reporter" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["price-reporter"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin renegade-price-reporter

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY --from=sources /build/price-reporter /build/price-reporter
WORKDIR /build
RUN cargo build --release -p renegade-price-reporter

# === Release === #
FROM --platform=arm64 debian:trixie-slim

RUN apt-get update && \
    apt-get install -y libssl-dev ca-certificates

# Copy the binary from the build stage
COPY --from=builder /build/target/release/renegade-price-reporter /bin/renegade-price-reporter
CMD ["renegade-price-reporter"]
