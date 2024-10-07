# Stage 1: Base compiler image with necessary dependencies
FROM rust:slim-bullseye AS base

# Install cargo-chef to optimize build caching
RUN cargo install cargo-chef

# Set the working directory to /app
WORKDIR /app

# Stage 2: Planner
FROM base AS planner

# Copy all files from the current directory into the container
COPY . .

# Prepare the recipe for caching dependencies (Cargo.toml/Cargo.lock)
RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder with necessary dependencies for OpenSSL
FROM base AS builder

# Install required dependencies for building Rust projects (OpenSSL, pkg-config, build-essential)
RUN apt-get update && apt-get install -y \
  pkg-config \
  libssl-dev \
  clang \
  build-essential \
  protobuf-compiler

# Copy the generated recipe from the planner stage
COPY --from=planner /app/recipe.json recipe.json

# Cache the dependencies using the cargo-chef recipe
RUN cargo chef cook --release --recipe-path recipe.json

# Copy the entire project into the container to proceed with the actual build
COPY . .

# Build the project in release mode
RUN cargo build --release

# Stage 4: Final runtime image
FROM debian:bullseye-slim

# Set the working directory for the final container
WORKDIR /usr/local/bin

# Copy the compiled binary from the builder stage
COPY --from=builder /app/target/release/helix-cmd /

# Install necessary runtime dependencies (OpenSSL and CA certificates)
RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*

# Define the entrypoint for the container
ENTRYPOINT ["/helix-cmd"]
