

import { readFile, stat, writeFile } from "fs/promises";
import { ConfigFile, ServerConfig, WorkspaceConfig } from "../types/config";
import logger from "./logger";

const CONFIG_FILE = process.env.CONFIG_FILE || "config.json"; // Config file name, can be set via environment variable
const CONFIG_DIR = process.env.CONFIG_DIR || "../config"; // Docker config dir
const CONFIG_FILE_LOCATION = `${CONFIG_DIR}/${CONFIG_FILE}`; // Config file location

export class Config {
    private config: ConfigFile | null = null;
    private loadedModifiedMs: number | null = null;

    constructor() {
    }

    /**
     * Read and parse the config file.
     *
     * The parsed result is kept in memory. The file is only read again when its
     * modification time changes, so an edit still applies without a restart.
     * On a read or parse error the last good config stays active.
     */
    public load = async (): Promise<void> => {
        try {
            const { mtimeMs } = await stat(CONFIG_FILE_LOCATION);

            if (this.config && this.loadedModifiedMs === mtimeMs) {
                return;
            }

            const data = await readFile(CONFIG_FILE_LOCATION, "utf-8");

            this.config = JSON.parse(data);
            this.loadedModifiedMs = mtimeMs;

        } catch (err) {
            logger.error(`Failed to load config file '${CONFIG_FILE_LOCATION}': ${err}`);

            this.config = this.config ?? {};
        }
    }

    public save = async (): Promise<void> => {
        if (!this.config) {
            throw new Error("Config not loaded");
        }


        await writeFile(CONFIG_FILE_LOCATION, JSON.stringify(this.config, null, 2), "utf-8");

        // Force a re-read on the next load, the file changed.
        this.loadedModifiedMs = null;
    }

    private async ensureLoaded() {
        if (!this.config) {
            await this.load();
        }
    }

    public async getServerConfig(): Promise<ServerConfig | undefined> {
        await this.ensureLoaded();
        return this.config?.server;
    }

    public async getDefaultWorkspaceConfig(): Promise<WorkspaceConfig | undefined> {
        await this.ensureLoaded();

        let defaultWorkspace = this.config?.workspaces?.find((ws: WorkspaceConfig) => ws.id === "default");
        let firstWorkspace = this.config?.workspaces ? this.config.workspaces[0] : undefined;

        return defaultWorkspace ? defaultWorkspace : firstWorkspace;
    }

    public async getWorkspaceConfig(workspaceId: string): Promise<WorkspaceConfig | undefined> {
        await this.ensureLoaded();
        return this.config?.workspaces?.find((ws: WorkspaceConfig) => ws.id === workspaceId);
    }

    public async getWorkspaces(): Promise<WorkspaceConfig[]> {
        await this.ensureLoaded();
        return this.config?.workspaces ?? [];
    }

    public async getWorkspaceIds(): Promise<string[]> {
        await this.ensureLoaded();

        return this.config?.workspaces ? this.config.workspaces.map((ws: WorkspaceConfig) => ws.id) : [];
    }

}