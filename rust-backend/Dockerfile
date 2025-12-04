FROM rust:1-slim-bullseye AS builder


RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    cmake \
    file \
    libssl-dev \
    libfreetype6-dev \
    libexpat1-dev \
    libxml2-dev \
    libproj-dev proj-data proj-bin \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
# ---------------------------------------------------------------------
# OPTIMIZATION START: Cache Dependencies
# ---------------------------------------------------------------------

# 2. Copy ONLY the manifest files first
COPY Cargo.toml Cargo.lock ./

# 3. Create a dummy main.rs to trick cargo into building only dependencies
RUN mkdir src && echo "fn main() {println!(\"if you see this, the build failed\");}" > src/main.rs

# 4. Build the dependencies
# This layer will be CACHED unless Cargo.toml or Cargo.lock changes.
# ---------------------------------------------------------------------
# DEVELOPMENT TOGGLE:
# For SUPER fast dev builds, comment out the line below and uncomment the "--mount" version.
# The mount version keeps a cache on your hard drive between runs.
# ---------------------------------------------------------------------

RUN cargo build --release
# RUN --mount=type=cache,target=/usr/local/cargo/registry \
#     --mount=type=cache,target=/app/target \
#     cargo build --release

# 5. Clean up the dummy build artifacts so the real code compiles correctly
RUN rm src/*.rs

# ---------------------------------------------------------------------
# OPTIMIZATION END
# ---------------------------------------------------------------------

# 6. NOW copy your actual source code
COPY . .

# 7. Build the actual application
# Because dependencies are already built in the layer above, 
# this will only compile your code (fast!).
# Use the "touch" command to ensure Cargo knows the file is new.
RUN touch src/main.rs 

RUN cargo build --release

FROM debian:bullseye-slim

RUN apt-get update && apt-get install -y \
    libssl1.1 \
    libfreetype6 \
    libexpat1 \
    libxml2 \
    libproj-dev proj-data proj-bin \
    file \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /app

COPY --from=builder /app/target/release/rust-backend .

CMD ["./rust-backend"]
