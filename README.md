# Beacon WMS

Beacon WMS is a web map service built with a dual-backend architecture for optimal performance and scalability.

## Overview

### Architecture

The application consists of two complementary backends:

- **Node.js Backend**: Handles client communication, templating, and OGC (Open Geospatial Consortium) compliance
- **Rust Backend**: Handles high-performance map drawing and querying operations

---

## Pages

### Map preview

The root URL serves a Leaflet map preview. It reads `/workspaces` for the workspace
list, then reads GetCapabilities for everything else.

The page gives you:

- a workspace select. The choice goes in the `?workspace=` parameter, so a link is shareable;
- one row per layer, with a visibility box, a style select and a viewparams text box;
- a control per TIME and ELEVATION dimension. A dimension with fixed values gets a select;
- the legend of each visible layer;
- feature info. Turn on "Query features on click", then click the map.

### Admin page

`/admin` holds the admin controls. The page asks for the admin secret first. See
[Manual Refresh](#manual-refresh) for what the buttons do.

Serve the site over HTTPS. The secret travels in a request header, and the page keeps
it in `sessionStorage` until the tab closes.

### Admin Secret

`ADMIN_SECRET` holds a bcrypt hash of the secret. Make the hash one time.

```bash
cd node-backend
npx bcrypt "my-secret" 12
```

Put the output in `ADMIN_SECRET`. Use single quotes in the `.env` file. The hash
holds `$` characters, and Docker Compose reads an unquoted `$` as a variable. It
then drops a part of the hash.

```
ADMIN_SECRET='$2b$12$PssOXswO9ldXZ1pO0Bnn7eDRIByomhCVQ/ppxKR8niUh1NV7l1VsK'
```

Clients still send the plain secret, not the hash.

A plaintext `ADMIN_SECRET` also works, for backwards compatibility. The node
backend then writes a warning to the log at startup.

### Page files

The pages are plain static files in `node-backend/public/`. There is no build step.

| Path | Content |
| --- | --- |
| `public/index.html` | Preview page markup. |
| `public/admin.html` | Admin page markup. |
| `public/css/` | `common.css`, `preview.css`, `admin.css`. |
| `public/js/config.js` | Base tile URL, start view, delays. Edit this first. |
| `public/js/api.js` | Backend calls. |
| `public/js/capabilities.js` | GetCapabilities XML parsing. |
| `public/js/preview.js` | Preview page logic. |
| `public/js/admin.js` | Admin page logic. |

The pages use [Alpine.js](https://alpinejs.dev/) and [Leaflet](https://leafletjs.com/).
Both come from `node_modules`, so the pages need no internet access.

Every URL in the HTML is relative, so `PATH_PREFIX` keeps working. Do not write an
absolute path into these files.

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
| `WORKERS` | `4` | Number of Tokio async worker threads. They run protocol and I/O work only. |
| `MAP_WORKERS` | _(CPU cores)_ | Number of GetMap renders that run at the same time. |
| `LABEL_FONT_PATH` | _(none)_ | TrueType font file for the legend labels. The backend searches the system fonts when it is empty. |
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
`Authorization: Bearer <secret>`. Send the plain secret. See
[Admin Secret](#admin-secret).

`GET /admin/queue` reports the queued layers and the layers that run now.

```bash
curl -H "Authorization: Bearer my-secret" http://localhost:3000/admin/queue
```

`GET /admin/update` refreshes one queued layer. The request stays open until the
Beacon query ends, so it can take minutes. Call it again while `queued_count` in
the answer is above zero.

```bash
curl -H "Authorization: Bearer my-secret" http://localhost:3000/admin/update
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

The [admin page](#admin-page) at `/admin` runs the same calls from the browser. Its
"Update all" button calls `/admin/update` again until the queue is empty.

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
| `ADMIN_SECRET` | _(empty)_ | Bcrypt hash of the admin secret. Admin endpoints need it (must be set to enable). Plaintext still works and logs a warning. |






