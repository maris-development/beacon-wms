import { compile } from "path-to-regexp";

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
    root: new Route("/"),
    updateLayers: new Route("/admin/update-layers"),
    defaultWms: new Route("/wms"),
    workspaceWms: new Route("/workspaces/:workspaceId/wms"),
};