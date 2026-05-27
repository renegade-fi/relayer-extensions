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
COPY ./pool-runner ./pool-runner
COPY ./price-reporter-client ./price-reporter-client

# === Builder === #
# Pull the sources into their own layer
FROM chef AS builder

# Disable compiler warnings and enable backtraces for panic debugging
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1
ENV CARGO_HTTP_CHECK_REVOKE=false

COPY --from=sources /build /build
WORKDIR /build

# Install protoc, openssl, and pkg-config
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev libclang-dev ca-certificates

# Update Cargo.toml to include only "pool-runner" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["pool-runner"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin pool-runner

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY --from=sources /build/pool-runner /build/pool-runner
COPY --from=sources /build/price-reporter-client /build/price-reporter-client
WORKDIR /build

RUN cargo build --release -p pool-runner

# === Release stage === #
FROM --platform=arm64 debian:trixie-slim
RUN apt-get update && \
    apt-get install -y libssl-dev ca-certificates

# Copy the binary from the build stage
COPY --from=builder /build/target/release/pool-runner /bin/pool-runner

# Set log filtering
ENV RUST_LOG="pool_runner=info,warp=warn,warp::filters::trace=off"
ENTRYPOINT ["/bin/pool-runner"]
