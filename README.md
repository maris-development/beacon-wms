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
| `DATASET_TTL_SECONDS` | `86400` | Maximum age of a layer parquet file before the backend queues a refresh. |
| `REFRESH_INTERVAL_SECONDS` | `1800` | Time between two runs of the background refresh worker. |
| `REFRESH_CONCURRENCY` | `1` | Number of Beacon refresh queries that run at the same time. |

## Layer Data Refresh

The backend serves the layer parquet file that is on disk. A request never waits
for a refresh.

1. A request finds a layer file that is older than `DATASET_TTL_SECONDS`.
2. The backend returns that file and puts the layer in the refresh queue. The
   queue holds each layer one time.
3. Every `REFRESH_INTERVAL_SECONDS`, the refresh worker queries Beacon for the
   queued layers, `REFRESH_CONCURRENCY` at a time.
4. The worker writes the result to a temporary file and then renames it. The
   rename is atomic, so a reader never sees a part of the new file.
5. A failed query keeps the old file. The next request queues the layer again.

Only a missing layer file makes a request wait for a Beacon query. Other requests
for the same layer wait for that one query.

### Manual Refresh

Two admin endpoints control the queue by hand. Both need
`Authorization: Bearer $ADMIN_SECRET`.

`GET /admin/queue` reports the queued layers and the layers that run now.

```bash
curl -H "Authorization: Bearer $ADMIN_SECRET" http://localhost:3000/admin/queue
```

`GET /admin/update` refreshes one queued layer. The request stays open until the
Beacon query ends, so it can take minutes. Call it again while `queued_count` in
the answer is above zero.

```bash
curl -H "Authorization: Bearer $ADMIN_SECRET" http://localhost:3000/admin/update
```

The `status` field holds one of these values.

| Status | HTTP | Meaning |
| --- | --- | --- |
| `updated` | 200 | The layer has new data. |
| `skipped` | 200 | The worker refreshed the layer already. |
| `empty` | 200 | The queue is empty. There is no work. |
| `busy` | 409 | Another manual update still runs. |
| `failed` | 502 | The query failed. The old file stays. |

A manual update takes only layers that are in the queue. To queue a layer, request
it once after `DATASET_TTL_SECONDS` passed.

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






