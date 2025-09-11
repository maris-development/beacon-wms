import express, { Request, Response, NextFunction } from "express";
import { Config } from "./service/config";
import { routes } from "./service/routes";
import { Utils } from "./service/utils";
import path from "path";
import { BeaconWmsService } from "./service/beacon-wms";

const config = new Config();
const wmsService: BeaconWmsService = new BeaconWmsService(config);

config.load(); // async Load config at startup

const app = express();
const port = 3000;

app.set("views", path.join(__dirname, "../templates"));
app.set("view engine", "ejs");

app.use(appMiddleware)
app.get(routes.root.getRoute(), homepage);
app.get(routes.defaultWms.getRoute(), defaultWms);
app.get(routes.workspaceWms.getRoute(), workspaceWms);
app.listen(port, () => {
  console.log(`Node backend listening at http://localhost:${port}`);
});


// Example proxy route to Rust backend
// app.get("/map", async (req, res) => {
//   const rustBackend = process.env.RUST_BACKEND_URL || "http://localhost:8080";
//   const response = await fetch(`${rustBackend}/`);
//   const text = await response.text();
//   res.send(`Rust says: ${text}`);
// });



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