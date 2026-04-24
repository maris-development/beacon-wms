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

## Rust Backend Environment Variables

| Variable | Default | Used for |
| --- | --- | --- |
| `HTTP_ADDRESS` | `0.0.0.0` | Rust backend bind address. |
| `HTTP_PORT` | `8000` | Rust backend HTTP port. |
| `WORKERS` | `12` | Number of Tokio worker threads. |
| `LOG_DIR` | `../logs` | Directory for backend logs. |
| `LOG_LEVEL` | `INFO` | Log verbosity (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`). |
| `CONFIG_DIR` | `../config` | Base directory for config files like `config.json` and `colormaps.json`. |
| `CONFIG_FILE` | `config.json` | Main backend config file name (resolved under `CONFIG_DIR`). |
| `LAYER_DIR` | `../layers` | Directory where generated layer parquet files are stored. |
| `BEACON_TOKEN` | _(none)_ | Auth token used for Beacon API queries. |
| `TILE_CACHE_ENABLED` | `false` | Enables tile image cache when set to `1`, `true`, `yes`, or `on`. |
| `TILE_CACHE_DIR` | `../tile_cache` | Root directory for tile cache files. |

## Node Backend Environment Variables

| Variable | Default | Used for |
| --- | --- | --- |
| `HTTP_ADDRESS` | `0.0.0.0` | Node backend bind address. |
| `HTTP_PORT` | `3000` | Node backend HTTP port. |
| `RUST_BACKEND_URL` | `http://localhost:8000` | Base URL for forwarding WMS requests to Rust backend. |
| `CONFIG_DIR` | `../config` | Base directory for Node config files. |
| `CONFIG_FILE` | `config.json` | Node config file name (resolved under `CONFIG_DIR`). |
| `LOG_DIR` | `../logs` | Directory for Node logs. |
| `PATH_PREFIX` | _(empty)_ | URL prefix prepended to Node routes. |
| `HTTP_HOST` | Request host header | Host used in generated capabilities URLs. |
| `HTTP_PROTOCOL` | Request protocol | Protocol used in generated capabilities URLs. |
| `ADMIN_SECRET` | _(empty)_ | Bearer token required for admin endpoints (must be set to enable). |






