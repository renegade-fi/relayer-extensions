# === Chef === #
ARG CARGO_FEATURES
FROM --platform=arm64 lukemathwalker/cargo-chef:latest-rust-1 AS chef

# Create a build dir and add local dependencies
WORKDIR /build
COPY ./rust-toolchain ./rust-toolchain
RUN cat rust-toolchain | xargs rustup toolchain install

# === Planner === #
FROM chef AS planner
WORKDIR /build

# Copy everything to generate the recipe
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./renegade-solver ./renegade-solver
RUN sed -i '/members[[:space:]]*=[[:space:]]*\[/,/\]/c\members = ["renegade-solver"]' Cargo.toml
RUN cargo chef prepare --recipe-path recipe.json --bin renegade-solver

# === Builder === #
FROM chef AS builder
WORKDIR /build

# Disable compiler warnings and enable backtraces for panic debugging
ENV RUSTFLAGS=-Awarnings
ENV RUST_BACKTRACE=1
ENV CARGO_HTTP_CHECK_REVOKE=false

# Install protoc, openssl, and pkg-config
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev libclang-dev libpq-dev ca-certificates

# Copy the recipe and workspace metadata from planner stage
COPY --from=planner /build/recipe.json recipe.json
COPY --from=planner /build/Cargo.toml /build/Cargo.lock ./

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

# Now copy the actual source code and build the application
COPY ./renegade-solver ./renegade-solver
RUN cargo build --release -p renegade-solver --features "$CARGO_FEATURES"

# === Release stage === #
FROM --platform=arm64 debian:bookworm-slim
RUN apt-get update && \
    apt-get install -y libssl-dev ca-certificates libpq-dev

# Copy the binary from the build stage
COPY --from=builder /build/target/release/renegade-solver /bin/renegade-solver

ENV RUST_LOG="info,warp::filters::trace=warn"
ENTRYPOINT ["/bin/renegade-solver"]
