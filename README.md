# Beacon WMS

Beacon WMS is a web map service built with a dual-backend architecture for optimal performance and scalability.

## Overview

### Architecture

The application consists of two complementary backends:

- **Node.js Backend**: Handles client communication, templating, and OGC (Open Geospatial Consortium) compliance
- **Rust Backend**: Handles high-performance map drawing and querying operations

---

## Getting Started

### Prerequisites

Before running Beacon WMS, ensure you have the following installed:

- [Node.js](https://nodejs.org/)
- [Rust](https://www.rust-lang.org/tools/install)

---

## Running Beacon WMS

### Production (with Docker)

To build and run both backends using Docker:

```bash
docker compose up --build
```

### Development (Local)

Run both backends in separate terminal windows:

**Terminal 1 - Rust Backend:**
```bash
cd rust-backend
cargo run --release  # Use --release for better performance
```

**Terminal 2 - Node.js Backend:**
```bash
cd node-backend
npm install
npm run dev
```






