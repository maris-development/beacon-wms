

import { readFile, writeFile } from "fs/promises";
import { ConfigFile, ServerConfig, WorkspaceConfig } from "../types/config";

const CONFIG_DIR = process.env.CONFIG_DIR || "../config"; // Docker config dir
const CONFIG_FILE_LOCATION = `${CONFIG_DIR}/config.json`; // Config file location

export class Config {
    private config: ConfigFile | null = null;

    constructor() {
    }

    public load = async (): Promise<void> => {
        let data = "{}";
        
        try {

            data = await readFile(CONFIG_FILE_LOCATION, "utf-8");

            // console.log("Loaded config file from " + configLocation, data);

        }catch(err){
            console.error(err);
        }

        this.config = JSON.parse(data);

        // console.log(">>>>>>>>>>>>>> Config loaded", this.config);
    }

    public save = async (): Promise<void> => {
        if (!this.config) {
            throw new Error("Config not loaded");
        }


        await writeFile(CONFIG_FILE_LOCATION, JSON.stringify(this.config, null, 2), "utf-8");
    }

    private async ensureLoaded() {
        if (!this.config) {
            await this.load();
        }
    }

    public getSecret(): string | undefined {
        return this.config?.secret;
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