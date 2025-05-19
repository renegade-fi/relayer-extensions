# === Chef === #
FROM --platform=arm64 rust:latest AS chef

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
COPY ./funds-manager ./funds-manager
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
    apt-get install -y pkg-config libssl-dev libclang-dev libpq-dev ca-certificates

# Update Cargo.toml to include only "funds-manager/funds-manager-server" in workspace members
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["funds-manager/funds-manager-server"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin funds-manager

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY --from=sources /build/funds-manager /build/funds-manager
COPY --from=sources /build/price-reporter-client /build/price-reporter-client
WORKDIR /build

RUN cargo build --release -p funds-manager

# === Release stage === #
FROM --platform=arm64 debian:bookworm-slim
RUN apt-get update && \
    apt-get install -y libssl-dev ca-certificates libpq-dev

# Copy the binary from the build stage
COPY --from=builder /build/target/release/funds-manager /bin/funds-manager

ENTRYPOINT ["/bin/funds-manager"]
CMD ["--datadog-logging"]