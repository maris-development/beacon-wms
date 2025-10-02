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
    root: new Route(`${path_prefix}/`),
    updateLayers: new Route(`${path_prefix}/admin/update-layers`),
    defaultWms: new Route(`${path_prefix}/wms`),
    workspaceWms: new Route(`${path_prefix}/workspaces/:workspaceId/wms`),
};