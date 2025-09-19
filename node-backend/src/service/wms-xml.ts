import { Response, Request } from "express";
import { Config } from "./config";
import { WorkspaceConfig } from "../types/config";
import { BeaconWmsService } from "./beacon-wms";


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

    async getCapabilities(req: Request, res: Response, workspace: WorkspaceConfig, availableStyles: string[], wmsVersion: string = "1.3.0") {
        const params = {
            wmsVersion,
            workspace,
            availableStyles,
            baseUrl: req.protocol + '://' + req.get('host') + req.originalUrl.split('?')[0],
            server: await this.config.getServerConfig()
        }
        
        const headers ={
            ...BeaconWmsService.CORS_HEADERS,
            "Cache-Control": "public, max-age=86400, stale-while-revalidate=3600",
            "Expires": new Date(Date.now() + 86400 * 1000).toUTCString() // Explicit expiry: 1 day
        }

        res
            .set(headers) // do this but good
            .contentType("application/xml")
            .render("wms-getcapabilities", params);

    }
}