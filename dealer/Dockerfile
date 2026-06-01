FROM --platform=arm64 rust:latest AS chef

WORKDIR /build
COPY ./rust-toolchain ./rust-toolchain
RUN rustup install $(cat rust-toolchain)

RUN apt-get update && apt-get install -y libssl-dev

# Install chef and generate a recipe
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash && cargo binstall -y cargo-chef

COPY ./Cargo.toml ./Cargo.toml
COPY ./Cargo.lock ./Cargo.lock
COPY ./renegade-dealer ./renegade-dealer
COPY ./renegade-dealer-api ./renegade-dealer-api
RUN cargo chef prepare --recipe-path recipe.json

# Disable compiler warnings and enable backtraces for panic
ENV RUSTFLAGS="-Awarnings -Zthreads=8"
ENV RUST_BACKTRACE=1

# Build only the dependencies to cache them in this layer
RUN cargo chef cook --release --recipe-path recipe.json

# Copy back in the full sources and build the tests
WORKDIR /build
COPY ./Cargo.lock ./Cargo.lock
COPY ./renegade-dealer ./renegade-dealer
COPY ./renegade-dealer-api ./renegade-dealer-api

WORKDIR /build/renegade-dealer
RUN cargo build --release --quiet --all-features

ENTRYPOINT ["cargo", "run", "--release", "--all-features"]

