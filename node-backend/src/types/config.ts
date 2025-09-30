export type ConfigFile = {
    secret?: string;
    server?: ServerConfig;
    workspaces?: WorkspaceConfig[];
}

export type ServerConfig = {
    title?: string;
    description?: string;
    contact?: string;
}

export type WorkspaceConfig = {
    id: string;
    name: string;
    description?: string;
    contact?: string;
    layers: LayerConfig[];
}


export type LayerConfig = {
    id: string;
    name: string;
    description?: string;
    config: {
        instance_url: string; // URL of the WMS instance
        token: string;        // API token for authentication
        query: Record<string, any>; // query being executed to fetch data
    }
}