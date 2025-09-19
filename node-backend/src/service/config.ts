

import { readFile, writeFile } from "fs/promises";
import { ConfigFile, ServerConfig, WorkspaceConfig } from "../types/config";

const CONFIG_FILE_LOCATION = "/../conf/config.json"; // Docker location

export class Config {
    private config: ConfigFile | null = null;

    constructor() {
    }

    public load = async (): Promise<void> => {
        let data = "{}";
        
        try {
            const configLocation = process.env.CONFIG_FILE_LOCATION || (process.cwd() + CONFIG_FILE_LOCATION);

            data = await readFile(configLocation, "utf-8");

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

        const configLocation = process.env.CONFIG_FILE_LOCATION || (process.cwd() + CONFIG_FILE_LOCATION);

        await writeFile(configLocation, JSON.stringify(this.config, null, 2), "utf-8");
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