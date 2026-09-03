import express, { Request, Response, NextFunction } from "express";
import { Config } from "./service/config";
import { routes } from "./service/routes";
import path from "path";
import { BeaconWmsService } from "./service/beacon-wms";
import { AdminService } from "./service/admin";
import { AdminSecret } from "./service/admin-secret";
import { WorkspaceConfig } from "./types/config";
import logger from "./service/logger";

const config = new Config();
const wmsService: BeaconWmsService = new BeaconWmsService(config);
const adminService: AdminService = new AdminService();


config.load(); // async Load config at startup

AdminSecret.reportFormat();

const http_address = process.env.HTTP_ADDRESS || "0.0.0.0";
const http_port: number = parseInt(process.env.HTTP_PORT || '3000');
const path_prefix = process.env.PATH_PREFIX || "";
const template_dir = path.join(__dirname, "../templates");
const public_dir = path.join(__dirname, "../public");
const module_dir = path.join(__dirname, "../node_modules");

const app = express();
app.set("views", template_dir);
app.set("view engine", "ejs");
app.disable("x-powered-by");

// The pages use relative URLs, so a missing slash breaks every asset. Express matches
// the prefix with and without the slash, so pass the slashed form on instead.
if (path_prefix) {
    app.get(path_prefix, (req: Request, res: Response, next: NextFunction) => {
        if (req.path.endsWith("/")) {
            next();

            return;
        }

        res.redirect(302, `${path_prefix}/${req.originalUrl.slice(req.path.length)}`);
    });
}

// Static assets come before appMiddleware, so they skip the config reload.
app.use(`${path_prefix}/vendor/leaflet`, express.static(path.join(module_dir, "leaflet/dist")));
app.use(`${path_prefix}/vendor/alpine`, express.static(path.join(module_dir, "alpinejs/dist")));
app.use(path_prefix || "/", express.static(public_dir));

app.use(appMiddleware)
app.get(routes.workspaces.getRoute(), workspaces);
app.get(routes.defaultWms.getRoute(), defaultWms);
app.get(routes.workspaceWms.getRoute(), workspaceWms);
app.get(routes.adminPage.getRoute(), adminPage);
app.get(routes.adminCheck.getRoute(), adminCheck);
app.get(routes.clearLayers.getRoute(), clearLayers);
app.get(routes.refreshQueue.getRoute(), refreshQueue);
app.get(routes.refreshUpdate.getRoute(), refreshUpdate);

const server = app.listen(http_port, http_address, () => {
  logger.info(`Node backend listening at http://${http_address}:${http_port}`);
});

server.on("error", (err: NodeJS.ErrnoException) => {
  logger.error(`Failed to bind to ${http_address}:${http_port} — ${err.message} (code: ${err.code})`);
  process.exit(1);
});




// Route Handlers

/// List the workspaces. The preview page reads the layers from GetCapabilities.
async function workspaces(req: Request, res: Response) {
    const list = (await config.getWorkspaces()).map(ws => {
        return {
            id: ws.id,
            name: ws.name,
            description: ws.description,
            wmsUrl: routes.workspaceWms.toPath({ workspaceId: ws.id })
        };
    });

    res.json({
        server: await config.getServerConfig() ?? {},
        workspaces: list
    });
}

function adminPage(req: Request, res: Response) {
    res.sendFile(path.join(public_dir, "admin.html"));
}

function adminCheck(req: Request, res: Response) {
    adminService.check(req, res);
}

async function defaultWms(req: Request, res: Response){
    let defaultWorkspaceConfig: WorkspaceConfig | undefined = await config.getDefaultWorkspaceConfig(); //first find the one with default.

    if(!defaultWorkspaceConfig){
        res.status(404).send("Default workspace not found");
        return;
    }

    req.params['workspaceId'] = defaultWorkspaceConfig.id;

    workspaceWms(req, res);
}

function workspaceWms(req: Request, res: Response){
    wmsService.handleWmsRequest(req, res);

}

function clearLayers(req: Request, res: Response){
    adminService.clearLayers(req, res);
}

function refreshQueue(req: Request, res: Response){
    adminService.queue(req, res);
}

function refreshUpdate(req: Request, res: Response){
    adminService.update(req, res);
}

function appMiddleware(req: Request, res: Response, next: NextFunction) {
    let promises = [];

    logger.info(`${req.method} ${req.url}`);
    
    promises.push(config.load()); // Ensure config is (re)loaded every request

    Promise.all(promises).then(() => next());
}