import { WMS_VERSION } from "./config.js";

/// Base URL of the backend, taken from the page location.
///
/// Every page URL holds PATH_PREFIX already, so a relative URL resolves to the
/// right place without the server writing the prefix into the HTML.
export function apiBase() {
    return new URL(".", document.baseURI);
}

export function apiUrl(path) {
    return new URL(path, apiBase()).toString();
}

export async function getWorkspaces() {
    const response = await fetch(apiUrl("workspaces"));

    if (!response.ok) {
        throw new Error(`The workspace list failed with status ${response.status}`);
    }

    return response.json();
}

/// Read GetCapabilities of one workspace and return the parsed XML document.
export async function getCapabilities(wmsUrl) {
    const params = new URLSearchParams({
        SERVICE: "WMS",
        VERSION: WMS_VERSION,
        REQUEST: "GetCapabilities",
    });

    const response = await fetch(`${wmsUrl}?${params}`);

    if (!response.ok) {
        throw new Error(`GetCapabilities failed with status ${response.status}`);
    }

    const xml = new DOMParser().parseFromString(await response.text(), "text/xml");

    if (xml.querySelector("parsererror")) {
        throw new Error("GetCapabilities returned invalid XML");
    }

    return xml;
}

/// Call an admin endpoint with the secret.
///
/// It returns the response, so the caller can act on a 401 itself.
export function adminFetch(path, secret) {
    return fetch(apiUrl(path), {
        headers: { Authorization: `Bearer ${secret}` },
    });
}
