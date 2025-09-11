export type WMSGetMapParameters = {
    // Mandatory parameters
    service: string;      // Must be "WMS"
    request: string;      // Must be "GetMap"
    version: string;      // e.g., "1.3.0"
    layers: string;       // Comma-separated layer names
    styles?: string;      // Optional: Comma-separated styles (default: "")
    crs: string;          // Coordinate Reference System (e.g., "EPSG:3857")
    bbox: string;          // Bounding box (e.g., "minx,miny,maxx,maxy")
    width: string;         // Width in pixels (e.g., "256")
    height: string;        // Height in pixels (e.g., "256")
    format: string;        // Output format (e.g., "image/png")

    // Optional parameters
    transparent?: string;  // "true" or "false" (not implemented in Rust)
    bgcolor?: string;      // Background color (e.g., "0xFFFFFF")
    exceptions?: string;   // Error format (e.g., "application/vnd.ogc.se_xml")
    time?: string;         // Time parameter (e.g., "2023-01-01")
    elevation?: string;    // Elevation parameter (optional)
};


export type WMSGetFeatureInfoParameters = {
    // Mandatory parameters
    service: string;      // Must be "WMS"
    request: string;      // Must be "GetFeatureInfo"
    version: string;      // e.g., "1.3.0"
    layers: string;       // Comma-separated layer names
    styles?: string;      // Comma-separated styles (optional)
    query_layers: string; // Comma-separated layer names to query (can be same as `layers`)
    info_format: string;  // Output format (e.g., "text/plain", "application/vnd.ogc.gml")
    crs: string;          // Coordinate Reference System (e.g., "EPSG:3857")
    bbox: string;         // Bounding box (e.g., "minx,miny,maxx,maxy")
    width: string;        // Width in pixels (e.g., "256")
    height: string;       // Height in pixels (e.g., "256")
    x: string;            // X-coordinate of the query point (in pixels)
    y: string;            // Y-coordinate of the query point (in pixels)

    // Optional parameters
    feature_count?: string; // Maximum number of features to return (optional)
    exceptions?: string;  // Error format (e.g., "application/vnd.ogc.se_xml")
    time?: string;        // Time parameter (optional)
    elevation?: string;   // Elevation parameter (optional)
};
