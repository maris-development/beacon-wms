import Alpine from "../vendor/alpine/module.esm.js";
import { BASE_LAYER, START_VIEW, WMS_VERSION, FEATURE_COUNT, INPUT_DELAY_MS } from "./config.js";
import { getWorkspaces, getCapabilities } from "./api.js";
import { readLayers } from "./capabilities.js";

// Leaflet objects stay out of the Alpine data. Alpine wraps its data in proxies, and
// a proxied layer no longer matches the instance that Leaflet holds, so removeLayer fails.
let map = null;
const tiles = new Map();
const inputTimers = new Map();
let clickMarker = null;

function preview() {
    return {
        title: "Beacon WMS",
        workspaces: [],
        workspaceId: "",
        layers: [],
        loading: true,
        error: "",
        queryOn: false,
        querying: false,
        status: "Ready",
        clickCoords: "",
        features: null,
        featureError: "",

        async init() {
            this.createMap();
            await this.loadWorkspaces();
        },

        createMap() {
            map = L.map("map", {
                center: START_VIEW.center,
                zoom: START_VIEW.zoom,
            });

            L.tileLayer(BASE_LAYER.url, {
                subdomains: BASE_LAYER.subdomains,
                attribution: BASE_LAYER.attribution,
                maxZoom: BASE_LAYER.maxZoom,
            }).addTo(map);

            clickMarker = L.circleMarker([0, 0], {
                radius: 6,
                color: "#c0392b",
                fillColor: "#e74c3c",
                fillOpacity: 0,
            });

            map.on("click", event => this.queryFeatures(event));

            // The toolbar height changes after the layers load, so recheck the size.
            requestAnimationFrame(() => map.invalidateSize());
        },

        async loadWorkspaces() {
            try {
                const data = await getWorkspaces();

                this.title = data.server?.title || "Beacon WMS";
                document.title = this.title;
                this.workspaces = data.workspaces || [];

                if (this.workspaces.length === 0) {
                    this.error = "The configuration holds no workspaces.";
                    this.loading = false;

                    return;
                }

                const wanted = new URLSearchParams(location.search).get("workspace");
                const found = this.workspaces.find(workspace => workspace.id === wanted);

                this.workspaceId = found ? found.id : this.workspaces[0].id;

                await this.loadLayers();
            } catch (err) {
                this.error = err.message;
                this.loading = false;
            }
        },

        get workspace() {
            return this.workspaces.find(workspace => workspace.id === this.workspaceId);
        },

        get visibleLayers() {
            return this.layers.filter(layer => layer.visible);
        },

        get legends() {
            return this.visibleLayers.filter(layer => layer.legendUrl);
        },

        async selectWorkspace() {
            const url = new URL(location.href);

            url.searchParams.set("workspace", this.workspaceId);
            history.replaceState(null, "", url);

            await this.loadLayers();
        },

        /// Read the layers of the selected workspace out of GetCapabilities.
        async loadLayers() {
            this.removeAllTiles();
            this.layers = [];
            this.loading = true;
            this.error = "";

            try {
                const xml = await getCapabilities(this.workspace.wmsUrl);

                this.layers = readLayers(xml).map((layer, index) => ({
                    ...layer,
                    visible: index === 0,
                    style: "",
                    viewparams: "",
                    dims: Object.fromEntries(
                        layer.dimensions.map(dimension => [dimension.name, dimension.default])
                    ),
                }));

                for (const layer of this.layers) {
                    this.syncLayer(layer);
                }

                if (this.layers.length === 0) {
                    this.error = "GetCapabilities holds no named layers.";
                }
            } catch (err) {
                this.error = err.message;
            } finally {
                this.loading = false;
            }
        },

        wmsOptions(layer) {
            const options = {
                layers: layer.id,
                styles: layer.style || "",
                format: "image/png",
                transparent: true,
                version: WMS_VERSION,
                attribution: "Beacon WMS",
            };

            if (layer.viewparams) {
                options.viewparams = layer.viewparams;
            }

            for (const dimension of layer.dimensions) {
                const value = layer.dims[dimension.name];

                if (!value) {
                    continue;
                }

                if (dimension.name === "TIME") {
                    options.time = value;
                }

                if (dimension.name === "ELEVATION") {
                    options.elevation = value;
                }
            }

            return options;
        },

        /// Put the map layer in step with the controls. It handles both a toggle and a redraw.
        syncLayer(layer) {
            const current = tiles.get(layer.id);

            if (current) {
                map.removeLayer(current);
                tiles.delete(layer.id);
            }

            if (!layer.visible) {
                return;
            }

            const tile = L.tileLayer.wms(this.workspace.wmsUrl, this.wmsOptions(layer));

            tiles.set(layer.id, tile);
            tile.addTo(map);
        },

        /// Wait for the user to stop typing before the layer redraws.
        delayedSync(layer) {
            clearTimeout(inputTimers.get(layer.id));
            inputTimers.set(layer.id, setTimeout(() => this.syncLayer(layer), INPUT_DELAY_MS));
        },

        removeAllTiles() {
            for (const tile of tiles.values()) {
                map.removeLayer(tile);
            }

            tiles.clear();
        },

        /// GetCapabilities gives a LegendURL for the default style only. Swap the
        /// STYLE parameter to get the legend of another style.
        legendFor(layer) {
            if (!layer.legendUrl) {
                return "";
            }

            if (!layer.style) {
                return layer.legendUrl;
            }

            const url = new URL(layer.legendUrl, document.baseURI);

            url.searchParams.set("STYLE", layer.style);

            return url.toString();
        },

        toggleQuery() {
            const container = map.getContainer();

            if (this.queryOn) {
                container.classList.add("query-active");
                this.status = "Click on the map to query features.";

                return;
            }

            container.classList.remove("query-active");
            this.status = "Ready";
            this.features = null;
            this.featureError = "";
            this.clickCoords = "";

            if (map.hasLayer(clickMarker)) {
                map.removeLayer(clickMarker);
            }
        },

        async queryFeatures(event) {
            if (!this.queryOn) {
                return;
            }

            const active = this.visibleLayers;

            if (active.length === 0) {
                this.featureError = "";
                this.features = [];
                this.status = "No layer is visible.";

                return;
            }

            const latlng = event.latlng;

            clickMarker.setLatLng(latlng).addTo(map);

            this.clickCoords = `${latlng.lat.toFixed(5)}, ${latlng.lng.toFixed(5)}`;
            this.status = `Querying at ${latlng.lat.toFixed(4)}, ${latlng.lng.toFixed(4)}`;
            this.querying = true;
            this.featureError = "";

            try {
                const collection = await this.requestFeatureInfo(latlng, active);

                this.features = collection.features || [];
                this.status = `Found ${this.features.length} feature(s) at ${this.clickCoords}`;
            } catch (err) {
                this.features = null;
                this.featureError = err.message;
                this.status = "The query failed.";
            } finally {
                this.querying = false;
            }
        },

        async requestFeatureInfo(latlng, active) {
            const size = map.getSize();
            const bounds = map.getBounds();
            const point = map.latLngToContainerPoint(latlng);

            // WMS 1.3.0 with EPSG:3857 wants the bounding box in metres.
            const southWest = L.CRS.EPSG3857.project(bounds.getSouthWest());
            const northEast = L.CRS.EPSG3857.project(bounds.getNorthEast());

            const layerNames = active.map(layer => layer.id).join(",");

            const params = new URLSearchParams({
                SERVICE: "WMS",
                VERSION: WMS_VERSION,
                REQUEST: "GetFeatureInfo",
                LAYERS: layerNames,
                QUERY_LAYERS: layerNames,
                INFO_FORMAT: "application/json",
                CRS: "EPSG:3857",
                BBOX: `${southWest.x},${southWest.y},${northEast.x},${northEast.y}`,
                WIDTH: size.x.toString(),
                HEIGHT: size.y.toString(),
                I: Math.round(point.x).toString(),
                J: Math.round(point.y).toString(),
                FEATURE_COUNT: FEATURE_COUNT.toString(),
                STYLES: active.map(layer => layer.style || "").join(","),
            });

            const viewparams = active.map(layer => layer.viewparams).filter(Boolean).join(";");

            if (viewparams) {
                params.set("VIEWPARAMS", viewparams);
            }

            // The request holds one value per dimension. Take the first layer that has it.
            for (const name of ["TIME", "ELEVATION"]) {
                const layer = active.find(item => item.dims[name]);

                if (layer) {
                    params.set(name, layer.dims[name]);
                }
            }

            const response = await fetch(`${this.workspace.wmsUrl}?${params}`);

            if (!response.ok) {
                throw new Error(`Server error ${response.status}: ${await response.text()}`);
            }

            return response.json();
        },

        featureRows(feature) {
            return Object.entries(feature.properties || {});
        },

        featureTitle(feature, index) {
            const coordinates = feature.geometry?.coordinates;

            if (!coordinates) {
                return `Feature ${index + 1}`;
            }

            return `Feature ${index + 1} — ${coordinates[1].toFixed(5)}, ${coordinates[0].toFixed(5)}`;
        },
    };
}

Alpine.data("preview", preview);
Alpine.start();
