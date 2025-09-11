import { Response, Request } from "express";
import { Config } from "./config";
import { WorkspaceConfig } from "../types/config";


export class WmsXmlService {
    constructor(
        private readonly config: Config
    ) { }

    async error(res: Response, errorId: string, errorMessage: string, wmsVersion: string = "1.3.0", httpCode: number = 400) {
        const params = {
            errorId,
            errorMessage,
            wmsVersion,
            server: await this.config.getServerConfig()
        }

        res.status(httpCode).contentType("application/xml").render("wms-error", params);
    }

    async getCapabilities(req: Request, res: Response, workspace: WorkspaceConfig, wmsVersion: string = "1.3.0") {
        const params = {
            wmsVersion,
            workspace: workspace,
            baseUrl: req.protocol + '://' + req.get('host') + req.originalUrl.split('?')[0],
            server: await this.config.getServerConfig()
        }

        res
            .header("Access-Control-Allow-Origin", "*") // Allow requests from any origin
            .header("Access-Control-Allow-Methods", "GET, OPTIONS") // Specify allowed HTTP methods
            .header("Access-Control-Allow-Headers", "Content-Type, Authorization") // Specify allowed headers
            .header("Access-Control-Max-Age", "86400") // Cache preflight response for 24 hours
            .header("Cache-Control", "public, max-age=86400, stale-while-revalidate=3600") // Client cache: 1 day
            .header("Expires", new Date(Date.now() + 86400 * 1000).toUTCString()) // Explicit expiry: 1 day
            .contentType("application/xml")
            .render("wms-getcapabilities", params);

    }
}