import { compile } from "path-to-regexp";

const path_prefix = process.env.PATH_PREFIX || "";


export class Route {
    private route: string;

    constructor(route: string) {
        this.route = route;
    }
    
    public getRoute = (): string => {
        return this.route;
    }
    public toPath = (params?: Record<string, string>): string => {
        
        const toPath = compile(this.route);

        return toPath(params || {});
    }

}

export const routes = {
    workspaces: new Route(`${path_prefix}/workspaces`),
    adminPage: new Route(`${path_prefix}/admin`),
    adminCheck: new Route(`${path_prefix}/admin/check`),
    clearLayers: new Route(`${path_prefix}/admin/clear-layers`),
    refreshQueue: new Route(`${path_prefix}/admin/queue`),
    refreshUpdate: new Route(`${path_prefix}/admin/update`),
    defaultWms: new Route(`${path_prefix}/wms`),
    workspaceWms: new Route(`${path_prefix}/workspaces/:workspaceId/wms`),
};