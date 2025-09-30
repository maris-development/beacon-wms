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

COPY . .

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
