FROM rust:1-slim-bullseye

# Install PROJ dependencies
RUN apt-get update && apt-get install -y \
    libproj-dev proj-data proj-bin pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy source and build
COPY . .

RUN cargo build --release

CMD ["./target/release/rust-backend"]