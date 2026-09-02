import { MAX_DIMENSION_VALUES } from "./config.js";

const DIMENSION_NAMES = ["TIME", "ELEVATION"];

function directChildrenByName(node, localName) {
    return Array.from(node.children).filter(child => child.localName === localName);
}

function firstDirectChildText(node, localName) {
    const child = directChildrenByName(node, localName)[0];

    return child ? child.textContent.trim() : "";
}

function parseISO8601Duration(text) {
    const match = text.match(/^P(?:(\d+)Y)?(?:(\d+)M)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/);

    if (!match) {
        return null;
    }

    return {
        years: parseInt(match[1] || 0, 10),
        months: parseInt(match[2] || 0, 10),
        days: parseInt(match[3] || 0, 10),
        hours: parseInt(match[4] || 0, 10),
        minutes: parseInt(match[5] || 0, 10),
        seconds: parseInt(match[6] || 0, 10),
    };
}

function addDuration(date, duration) {
    const result = new Date(date);

    result.setUTCFullYear(result.getUTCFullYear() + duration.years);
    result.setUTCMonth(result.getUTCMonth() + duration.months);
    result.setUTCDate(result.getUTCDate() + duration.days);
    result.setUTCHours(result.getUTCHours() + duration.hours);
    result.setUTCMinutes(result.getUTCMinutes() + duration.minutes);
    result.setUTCSeconds(result.getUTCSeconds() + duration.seconds);

    return result;
}

/// Walk from a start date and collect one value per step.
function collectSteps(start, duration, limit, stopAt) {
    const values = [];
    let cursor = new Date(start);

    for (let i = 0; i < limit; i++) {
        if (stopAt && cursor.getTime() > stopAt.getTime()) {
            break;
        }

        values.push(cursor.toISOString());

        const next = addDuration(cursor, duration);

        // Guard against a zero or negative step, which never ends.
        if (next.getTime() <= cursor.getTime()) {
            break;
        }

        cursor = next;
    }

    return values;
}

/**
 * Expand an ISO 8601 time interval into single values.
 *
 * Accepted forms:
 *   Rn/start/period    n values from start. GetCapabilities uses this form.
 *   R/start/period     values from start until now
 *   start/end/period   values from start to end
 *   start/period       values from start until now
 *   period/end         values back from end
 *
 * Returns null when the text is not an interval.
 */
export function expandTimeInterval(spec) {
    const parts = spec.split("/");

    if (parts.length < 2 || parts.length > 3) {
        return null;
    }

    if (parts.length === 3 && /^R\d*$/.test(parts[0])) {
        const count = parts[0].length > 1 ? parseInt(parts[0].slice(1), 10) : MAX_DIMENSION_VALUES;
        const start = new Date(parts[1]);
        const duration = parseISO8601Duration(parts[2]);

        if (!duration || isNaN(start.getTime())) {
            return null;
        }

        return collectSteps(start, duration, Math.min(count, MAX_DIMENSION_VALUES), null);
    }

    if (parts.length === 3) {
        const start = new Date(parts[0]);
        const end = new Date(parts[1]);
        const duration = parseISO8601Duration(parts[2]);

        if (!duration || isNaN(start.getTime()) || isNaN(end.getTime())) {
            return null;
        }

        return collectSteps(start, duration, MAX_DIMENSION_VALUES, end);
    }

    if (parts[1].startsWith("P")) {
        const start = new Date(parts[0]);
        const duration = parseISO8601Duration(parts[1]);

        if (!duration || isNaN(start.getTime())) {
            return null;
        }

        return collectSteps(start, duration, MAX_DIMENSION_VALUES, new Date());
    }

    if (parts[0].startsWith("P")) {
        const end = new Date(parts[1]);
        const duration = parseISO8601Duration(parts[0]);

        if (!duration || isNaN(end.getTime())) {
            return null;
        }

        const back = {
            years: -duration.years,
            months: -duration.months,
            days: -duration.days,
            hours: -duration.hours,
            minutes: -duration.minutes,
            seconds: -duration.seconds,
        };

        const values = [];
        let cursor = new Date(end);

        for (let i = 0; i < MAX_DIMENSION_VALUES; i++) {
            values.push(cursor.toISOString());

            const previous = addDuration(cursor, back);

            if (previous.getTime() >= cursor.getTime()) {
                break;
            }

            cursor = previous;
        }

        return values.reverse();
    }

    return null;
}

function readDimensions(layerEl) {
    const dimensions = [];

    for (const dimEl of directChildrenByName(layerEl, "Dimension")) {
        const name = (dimEl.getAttribute("name") || "").toUpperCase();

        if (!DIMENSION_NAMES.includes(name)) {
            continue;
        }

        const text = dimEl.textContent.trim();
        let values = text ? text.split(",").map(value => value.trim()).filter(Boolean) : [];

        if (name === "TIME") {
            const expanded = [];

            for (const value of values) {
                const steps = value.includes("/") ? expandTimeInterval(value) : null;

                if (steps) {
                    expanded.push(...steps);
                } else {
                    expanded.push(value);
                }
            }

            values = expanded;
        }

        dimensions.push({
            name,
            label: name === "ELEVATION" ? "Elevation" : "Time",
            default: dimEl.getAttribute("default") || "",
            values,
            discrete: isDiscrete(name, values),
        });
    }

    return dimensions;
}

/// Decide if a dimension gets a dropdown or a text input.
///
/// An ELEVATION value holds a depth range, so a slash is normal there. A TIME value
/// with a slash is an interval that stayed unexpanded, so it needs a text input.
function isDiscrete(name, values) {
    if (values.length === 0) {
        return false;
    }

    if (name === "ELEVATION") {
        return true;
    }

    return !values.some(value => value.includes("/"));
}

function readStyles(layerEl) {
    const styles = [];
    let legendUrl = "";

    for (const styleEl of directChildrenByName(layerEl, "Style")) {
        const name = firstDirectChildText(styleEl, "Name");

        if (!name) {
            continue;
        }

        styles.push(name);

        const legendEl = directChildrenByName(styleEl, "LegendURL")[0];

        if (!legendEl || legendUrl) {
            continue;
        }

        const resource = directChildrenByName(legendEl, "OnlineResource")[0];
        const href = resource ? resource.getAttribute("xlink:href") : "";

        if (href) {
            legendUrl = href;
        }
    }

    return { styles, legendUrl };
}

function collectLeafLayers(layerEl, out) {
    const childLayers = directChildrenByName(layerEl, "Layer");
    const name = firstDirectChildText(layerEl, "Name");

    if (name && childLayers.length === 0) {
        const { styles, legendUrl } = readStyles(layerEl);

        out.push({
            id: name,
            title: firstDirectChildText(layerEl, "Title") || name,
            styles,
            legendUrl,
            dimensions: readDimensions(layerEl),
        });

        return;
    }

    for (const child of childLayers) {
        collectLeafLayers(child, out);
    }
}

/// Read the named layers out of a GetCapabilities document.
export function readLayers(xml) {
    const capability = Array.from(xml.getElementsByTagName("*"))
        .find(node => node.localName === "Capability");

    const root = capability ? directChildrenByName(capability, "Layer")[0] : null;

    if (!root) {
        throw new Error("GetCapabilities holds no Capability/Layer element");
    }

    const layers = [];

    collectLeafLayers(root, layers);

    return layers;
}
