// Edit these values to change the map preview defaults.

export const BASE_LAYER = {
    url: "https://{s}.maris.nl/cdi/{z}/{x}/{y}.png",
    subdomains: ["tile", "tile-a", "tile-b", "tile-c"],
    attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    maxZoom: 19,
};

export const START_VIEW = {
    center: [52.3, 4.3],
    zoom: 6,
};

export const WMS_VERSION = "1.3.0";
export const FEATURE_COUNT = 10;

/// Delay before a typed value redraws the layer.
export const INPUT_DELAY_MS = 600;

/// Highest number of values that one dimension dropdown holds.
export const MAX_DIMENSION_VALUES = 5000;
