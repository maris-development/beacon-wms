# AGENTS.md

Instructions for AI agents that work on Beacon WMS.

## 1. What this project is

Beacon WMS is an OGC WMS server. It draws ocean measurement data as map tiles.

The data comes from a Beacon datalake instance (`beacon-datalake.org`). The server sends a JSON
query to the datalake, gets a parquet file back, and draws the points on a PNG image.

Two codebases work together:

| Codebase | Language | Role |
| --- | --- | --- |
| [node-backend/](node-backend/) | TypeScript, Express 5 | Web layer. OGC WMS protocol, validation, EJS templates, CORS and cache headers. |
| [rust-backend/](rust-backend/) | Rust, Axum, Tokio | Heavy work. Datalake queries, parquet reading, reprojection, image drawing. |

The node backend is public. The rust backend is internal. Only the node backend talks to it.

## 2. Request flow

```
browser (WMS client)
  -> node-backend  GET /workspaces/:workspaceId/wms?SERVICE=WMS&REQUEST=GetMap&...
     validates OGC parameters, maps them to internal parameters
  -> rust-backend  GET /get-map?workspace=...&layers=...&bbox=...&viewparams=...
     1. tile cache lookup (hash of all GetMap parameters)
     2. read config/config.json, find workspace + layer
     3. apply viewparams and OGC dimensions (time, elevation) to the layer
     4. hash the resolved viewparams -> layer file name
     5. if the parquet file is absent, query the datalake. If it is only stale,
        serve it and queue a background refresh
     6. read parquet, reproject points, draw on the image
     7. encode PNG, write to the tile cache
  <- PNG
  <- PNG with CORS and cache headers
```

## 3. Public routes (node-backend)

Defined in [routes.ts](node-backend/src/service/routes.ts). `PATH_PREFIX` goes before each route.

| Route | Purpose |
| --- | --- |
| `/` | HTML index page with the workspace list. |
| `/wms` | WMS endpoint of the default workspace. |
| `/workspaces/:workspaceId/wms` | WMS endpoint of one workspace. |
| `/admin/clear-layers` | Deletes the cached parquet layers. Needs `Authorization: Bearer $ADMIN_SECRET`. |

The WMS endpoint supports `GetCapabilities`, `GetMap`, `GetFeatureInfo` and `GetLegendGraphic`.
WMS versions `1.3.0` and `1.1.1` are accepted.

## 4. Internal routes (rust-backend)

| Route | Purpose |
| --- | --- |
| `/` | Health text. |
| `/get-map` | Draws the PNG. |
| `/get-feature-info` | Returns features as JSON, HTML or GML. |
| `/get-legend-graphic` | Draws a vertical color bar. |
| `/available-styles` | Lists the colormaps. Used by `GetCapabilities`. |
| `/clear-layers` | Deletes all parquet files in `LAYER_DIR`. |

## 5. Configuration

All configuration is in [config/](config/). Both backends read the same file.

- `config.json` — default configuration. Other variants are `config.cdi.json` and `config.ihm.json`.
  Select one with the `CONFIG_FILE` environment variable.
- `colormaps.json` — the styles. Each colormap has a name, an interpolation mode
  (`lab`, `linear` or `nearest`) and a scale of stops.

Structure: `server` -> `workspaces[]` -> `layers[]` -> `config`.

Important keys in `layers[].config`:

| Key | Meaning |
| --- | --- |
| `instance_url` | Beacon datalake base URL. The rust backend posts to `{instance_url}/api/query`. |
| `query` | The Beacon query as JSON. It holds `%placeholder%` tokens. |
| `available_viewparams` | Allowed viewparams and their types (`numeric`, `string`, `bool`, `numeric_array`, `string_array`). |
| `dimensions` | OGC `time` and `elevation` dimensions with defaults and accepted values. |
| `default_style` | Colormap name. Fallback is `thermal`. |
| `min_value`, `max_value` | Color scale range. Fallbacks are `-10.0` and `100.0`. |
| `log_style` | Use a logarithmic color scale. |
| `shape` | Point icon. Default is `circle`. |
| `token` | Deprecated. Use the `BEACON_TOKEN` environment variable. |

Both backends cache the parsed config in memory. Each one compares the modification time of the
file before use, and reads the file again only after a change. So a config edit applies without a
restart. If the new content is invalid, the rust backend answers `500` and the node backend keeps
the last good config. Both log the reason.

### Viewparams

The client sends `VIEWPARAMS=year:2020;depth:[0,5]`. Values parse as JSON, with a string fallback.
[viewparams.rs](rust-backend/src/viewparams.rs) validates them against `available_viewparams`,
merges the OGC `TIME` and `ELEVATION` dimensions, and substitutes them into the query string:

- `%year%` — scalar value.
- `"%year%"` — numbers and booleans replace the quotes too.
- `%bbox[0]%` — array element by index.

The `depth` viewparam has an extra rule. The value must match one of the bins in
`available_viewparams.depth.allowed`.

## 6. Caching

Three cache levels exist. Know which one to clear.

1. **Parquet layer cache** — `layers/{workspace}_{layer}_{viewparams_hash}.parquet`.
   A file older than `DATASET_TTL_SECONDS` still goes to the client. The refresh
   worker replaces it later. Clear with `/admin/clear-layers`.
2. **Reprojected batch cache** — in memory LRU, 50000 entries.
   See [cache_engine/mod.rs](rust-backend/src/cache_engine/mod.rs). Cleared on restart.
3. **Tile cache** — `tile_cache/{first 2 hex chars}/{sha256}.png`. PNG only.
   Set `TILE_CACHE_ENABLED=true` to use it. Delete the directory to clear it.

### Layer refresh

Only a missing layer file makes a request wait for a datalake query. A stale file
goes to the client at once and enters the refresh queue.

[refresh.rs](rust-backend/src/refresh.rs) holds the queue and the worker. The
worker starts in `main` and runs every `REFRESH_INTERVAL_SECONDS`. It queries the
datalake for the queued layers, `REFRESH_CONCURRENCY` at a time. Each query writes
a temporary file and then renames it, so a reader never sees a part of the new
file. A failed query keeps the old file.

`DATASET_MAP` in [main.rs](rust-backend/src/main.rs) holds a lock and a generation
per layer file path. See [queries.rs](rust-backend/src/queries.rs).

- The lock keeps concurrent cold misses to one query.
- The generation counts the refreshes of that file. It is part of the reprojected
  batch cache key, so a refreshed file never reuses the batches of the old file.
  Old entries drop out of the LRU cache on their own.

The tile cache has no such key. A refreshed layer keeps its old PNG tiles until
you delete the tile cache directory.

## 7. How to run

Docker (both backends):

```bash
docker compose up --build
```

Local development, two terminals:

```bash
cd rust-backend && cargo run --release
cd node-backend && npm install && npm run dev
```

Use `--release` for the rust backend. A debug build draws maps very slowly.

Visual test page: start a static server in [test/](test/) and open `index.html`.
See [test/README.md](test/README.md).

There is no automated test suite. Verify changes with real WMS requests.

## 8. Environment variables

`.env` in the root feeds `docker-compose.yml`. The file is git-ignored.
The [README.md](README.md) holds the full table. The important ones:

- `BEACON_TOKEN` — datalake bearer token. Required for protected datalakes.
- `ADMIN_SECRET` — bearer token for `/admin/*`. Admin routes stay closed while it is empty.
- `CONFIG_FILE` — selects the config variant, for example `config.ihm.json`.
- `TILE_CACHE_ENABLED` — `1`, `true`, `yes` or `on` turns the tile cache on.
- `HOST_HTTP_PORT` — host port for the node backend. Default is `3000`.
- `DATASET_TTL_SECONDS` — age at which a layer file needs a refresh. Default is `86400`.
- `REFRESH_INTERVAL_SECONDS` — run interval of the refresh worker. Default is `1800`.
- `REFRESH_CONCURRENCY` — parallel refresh queries. Default is `1`.

**Never commit `.env` and never print its content.** It holds a live token and the admin secret.

## 9. Code layout

### node-backend/src

| File | Content |
| --- | --- |
| [index.ts](node-backend/src/index.ts) | Express setup, route handlers, per-request config reload. |
| [service/beacon-wms.ts](node-backend/src/service/beacon-wms.ts) | All four WMS operations. Validation and proxy to rust. |
| [service/wms-xml.ts](node-backend/src/service/wms-xml.ts) | Renders capabilities XML and error XML. |
| [service/config.ts](node-backend/src/service/config.ts) | Reads and caches `config.json`. |
| [service/admin.ts](node-backend/src/service/admin.ts) | Bearer token check and clear-layers proxy. |
| [service/logger.ts](node-backend/src/service/logger.ts) | Winston, daily rotate to `LOG_DIR`. |
| [types/](node-backend/src/types/) | Config types and OGC WMS parameter types. |
| [templates/](node-backend/templates/) | EJS templates. |

### rust-backend/src

| File | Content |
| --- | --- |
| [main.rs](rust-backend/src/main.rs) | Axum routes, Tokio runtime, global statics. |
| [query_parameters.rs](rust-backend/src/query_parameters.rs) | Request structs and the tile cache hash. |
| [config/mod.rs](rust-backend/src/config/mod.rs) | Serde structs for `config.json`. |
| [viewparams.rs](rust-backend/src/viewparams.rs) | Viewparam and dimension parse, validation, substitution. |
| [queries.rs](rust-backend/src/queries.rs) | Layer file state: fetch lock, generation, freshness check, atomic replace. |
| [refresh.rs](rust-backend/src/refresh.rs) | Refresh queue and the background worker. |
| [beacon_api/mod.rs](rust-backend/src/beacon_api/mod.rs) | Posts the query, streams parquet to disk. |
| [data_utils.rs](rust-backend/src/data_utils.rs) | Parquet reader helpers. |
| [cache_engine/mod.rs](rust-backend/src/cache_engine/mod.rs) | LRU cache of reprojected record batches. |
| [map_drawing/mod.rs](rust-backend/src/map_drawing/mod.rs) | Point drawing. Shapes, radius per zoom, color LUT. |
| [map_querying/](rust-backend/src/map_querying/) | GetFeatureInfo hit test and output formats. |
| [color_maps/mod.rs](rust-backend/src/color_maps/mod.rs) | Colormap load, interpolation, LUT build. |
| [boundingbox/mod.rs](rust-backend/src/boundingbox/mod.rs) | BBOX parse, axis order, reprojection. |
| [legend/mod.rs](rust-backend/src/legend/mod.rs) | Legend graphic. |
| [misc.rs](rust-backend/src/misc.rs) | Projections, file paths, hashes, logger, env vars. |
| [tile_cache/mod.rs](rust-backend/src/tile_cache/mod.rs) | PNG tile cache on disk. |

## 10. Data contract

The datalake query must alias its columns. The rust backend expects these names in the parquet
file: `longitude`, `latitude` and `value`. See the constants in
[map_drawing/mod.rs](rust-backend/src/map_drawing/mod.rs). Source coordinates are always
`EPSG:4326`. The backend reprojects them to the requested CRS.

An empty parquet file is valid. It means the query found no data, and the layer draws nothing.

## 11. Rules for changes

- Keep the two backends aligned. `config.json` has a TypeScript type and a Rust struct.
  A new config key needs both, or the rust backend fails to deserialize.
- Add new WMS parameters in three places: the type in
  [types/ogc-wms.ts](node-backend/src/types/ogc-wms.ts), the forward call in
  [beacon-wms.ts](node-backend/src/service/beacon-wms.ts), and the struct in
  [query_parameters.rs](rust-backend/src/query_parameters.rs).
- Add a parameter that changes the image to `GetMapRequestParameters::hash()` too.
  If you forget it, the tile cache returns a stale image.
- Do the validation in the node backend and give an OGC XML error. The rust backend returns
  plain text, which WMS clients do not read.
- The node backend uses 4-space indentation. Keep the style of the file you edit.
- Do not commit `layers/*.parquet`, `tile_cache/` or `logs/`.
- Use the `log` crate in the rust backend and the winston logger in the node backend.
  Do not use `println!` or `console.log`.

## 12. Known rough edges

- Three tests fail on `main`: `map_drawing::tests::test_mercator_projection` and two tests in
  `viewparams::test_check_dimensions`. The failures pre-date the current work.
- `ColorMapsConfig::load()` reads `colormaps.json` on every cache miss. No modification time check.
