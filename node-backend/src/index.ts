import express, { Request, Response, NextFunction } from "express";
import { Config } from "./service/config";
import { routes } from "./service/routes";
import path from "path";
import { BeaconWmsService } from "./service/beacon-wms";

const config = new Config();
const wmsService: BeaconWmsService = new BeaconWmsService(config);

config.load(); // async Load config at startup

const http_address = process.env.HTTP_ADDRESS || "0.0.0.0";
const http_port: number = parseInt(process.env.HTTP_PORT || '3000');
const template_dir = process.env.TEMPLATE_DIR || path.join(__dirname, "../templates");

const app = express();
app.set("views", template_dir);
app.set("view engine", "ejs");
app.disable("x-powered-by");
app.use(appMiddleware)
app.get(routes.root.getRoute(), homepage);
app.get(routes.defaultWms.getRoute(), defaultWms);
app.get(routes.workspaceWms.getRoute(), workspaceWms);
app.listen(http_port, http_address, () => {
  console.log(`Node backend listening at http://${http_address}:${http_port}`);
  console.log(`Template dir: ${template_dir}`);
});




// Route Handlers

async function homepage(req: Request, res: Response) {
    const workspaces = (await config.getWorkspaces()).map(ws => {
        return {
            'ws': ws,
            'url': routes.workspaceWms.toPath({ workspaceId: ws.id })
        }
    });

    const params = { 
        workspaces, 
        server: await config.getServerConfig()
    };

    res.render("index", params);
}

function defaultWms(req: Request, res: Response){
    let defaultWorkspaceConfig = config.getWorkspaceConfig("default");

    if(!defaultWorkspaceConfig){
        res.status(404).send("Default workspace not found");
        return;
    }

    workspaceWms(req, res);
}

function workspaceWms(req: Request, res: Response){
    wmsService.handleWmsRequest(req, res);

}

function appMiddleware(req: Request, res: Response, next: NextFunction) {
    let promises = [];
    
    promises.push(config.load()); // Ensure config is (re)loaded every request

    Promise.all(promises).then(() => next());
}