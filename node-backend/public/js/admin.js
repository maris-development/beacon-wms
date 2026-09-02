import Alpine from "../vendor/alpine/module.esm.js";
import { adminFetch } from "./api.js";

const SECRET_KEY = "beacon-wms-admin-secret";

/// Highest number of updates that one "Update all" press runs.
const MAX_UPDATE_STEPS = 100;

const BADGE_BY_STATUS = {
    updated: "badge-ok",
    skipped: "badge-muted",
    empty: "badge-muted",
    busy: "badge-warn",
    failed: "badge-error",
};

function admin() {
    return {
        input: "",
        secret: "",
        authorized: false,
        checking: false,
        gateError: "",
        queue: null,
        loadingQueue: false,
        busy: false,
        action: "",
        log: [],

        async init() {
            const stored = sessionStorage.getItem(SECRET_KEY);

            if (!stored) {
                return;
            }

            this.input = stored;

            await this.signIn(true);
        },

        /// Send the secret to the server. `silent` keeps a stale stored secret quiet.
        async signIn(silent = false) {
            if (!this.input) {
                this.gateError = "Enter the admin secret.";

                return;
            }

            this.checking = true;
            this.gateError = "";

            try {
                const response = await adminFetch("admin/check", this.input);

                if (response.status === 401) {
                    sessionStorage.removeItem(SECRET_KEY);
                    this.input = "";

                    if (!silent) {
                        this.gateError = "The server refused the secret.";
                    }

                    return;
                }

                if (!response.ok) {
                    this.gateError = `The server answered ${response.status}.`;

                    return;
                }

                this.secret = this.input;
                sessionStorage.setItem(SECRET_KEY, this.secret);
                this.authorized = true;

                await this.loadQueue();
            } catch (err) {
                this.gateError = err.message;
            } finally {
                this.checking = false;
            }
        },

        signOut(message = "") {
            sessionStorage.removeItem(SECRET_KEY);

            this.secret = "";
            this.input = "";
            this.authorized = false;
            this.queue = null;
            this.gateError = message;
        },

        /// Call an admin endpoint. It parses JSON, and keeps plain text as it is.
        async call(path) {
            const response = await adminFetch(path, this.secret);

            if (response.status === 401) {
                this.signOut("The secret is no longer valid. Enter it again.");

                throw new Error("The server refused the secret.");
            }

            const text = await response.text();

            try {
                return { ok: response.ok, status: response.status, data: JSON.parse(text) };
            } catch {
                return { ok: response.ok, status: response.status, data: null, text };
            }
        },

        async loadQueue() {
            this.loadingQueue = true;

            try {
                const result = await this.call("admin/queue");

                if (!result.data) {
                    this.note(`The queue answered ${result.status}: ${result.text}`, "badge-error");

                    return;
                }

                this.queue = result.data;
            } catch (err) {
                this.note(err.message, "badge-error");
            } finally {
                this.loadingQueue = false;
            }
        },

        async updateOne() {
            this.busy = true;
            this.action = "Refreshing one dataset. This can take minutes.";

            try {
                await this.runUpdate();
                await this.loadQueue();
            } catch (err) {
                this.note(err.message, "badge-error");
            } finally {
                this.busy = false;
                this.action = "";
            }
        },

        /// Call update again until the queue is empty or a step does not succeed.
        async updateAll() {
            this.busy = true;

            try {
                for (let step = 0; step < MAX_UPDATE_STEPS; step++) {
                    this.action = `Refreshing dataset ${step + 1}. This can take minutes.`;

                    const result = await this.runUpdate();

                    if (!result) {
                        break;
                    }

                    if (result.status !== "updated" && result.status !== "skipped") {
                        break;
                    }

                    if (result.queued_count === 0) {
                        break;
                    }
                }

                await this.loadQueue();
            } catch (err) {
                this.note(err.message, "badge-error");
            } finally {
                this.busy = false;
                this.action = "";
            }
        },

        /// Run one update and write the answer to the log.
        async runUpdate() {
            const result = await this.call("admin/update");

            if (!result.data) {
                this.note(`Update answered ${result.status}: ${result.text}`, "badge-error");

                return null;
            }

            const data = result.data;
            const seconds = data.duration_seconds ? ` (${data.duration_seconds.toFixed(1)}s)` : "";

            // The log shows the newest entry first, so the detail goes in before the message.
            if (data.error) {
                this.note(data.error, "badge-error");
            }

            this.note(`${data.message}${seconds}`, BADGE_BY_STATUS[data.status] || "badge-muted", data.status);

            return data;
        },

        async clearLayers() {
            const ok = confirm(
                "This deletes every parquet layer file. The next map request waits for a live query. Continue?"
            );

            if (!ok) {
                return;
            }

            this.busy = true;
            this.action = "Deleting the layer files.";

            try {
                const result = await this.call("admin/clear-layers");
                const message = result.data ? JSON.stringify(result.data) : result.text;

                this.note(message, result.ok ? "badge-ok" : "badge-error", result.ok ? "cleared" : "failed");

                await this.loadQueue();
            } catch (err) {
                this.note(err.message, "badge-error");
            } finally {
                this.busy = false;
                this.action = "";
            }
        },

        note(text, badge = "badge-muted", label = "") {
            this.log.unshift({
                time: new Date().toLocaleTimeString(),
                text,
                badge,
                label,
            });
        },

        formatSeconds(value) {
            if (value === null || value === undefined) {
                return "-";
            }

            if (value < 120) {
                return `${value}s`;
            }

            if (value < 7200) {
                return `${Math.round(value / 60)}m`;
            }

            return `${Math.round(value / 3600)}h`;
        },
    };
}

Alpine.data("admin", admin);
Alpine.start();
