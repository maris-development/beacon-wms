import { Config } from "./config";
import { Utils } from "./utils";
import { Request, type Response } from "express";
import { WmsXmlService } from "./wms-xml";
import { WorkspaceConfig } from "../types/config";
import { request } from "http";
import { WMSGetFeatureInfoParameters, WMSGetMapParameters } from "../types/ogc-wms";
import { ParamsDictionary } from "express-serve-static-core";
import { ParsedQs } from "qs";
import { BeaconWmsService } from "./beacon-wms";
import logger from "./logger";
import { AdminSecret } from "./admin-secret";

export class AdminService {

    private checkSecret(req: Request): boolean {
        if (!AdminSecret.isSet()) {
            throw new Error("Environment variable ADMIN_SECRET not set. Please set it to a non-empty value to enable admin endpoints.");
        }

        const authHeader = req.headers["authorization"];

        if (!authHeader) {
            throw new Error("No authorization header");
        }

        const token = authHeader.split(" ")[1] || "";

        if (!AdminSecret.verify(token)) {
            throw new Error("Invalid token");
        }

        return true;
    }

    /// Validate the admin secret. The admin page calls it before it shows anything.
    check(req: Request, res: Response) {
        if (!this.authorize(req, res)) {
            return;
        }

        res.status(200).json({ status: "ok" });
    }

    /// Report the datasets that wait for a refresh.
    queue(req: Request, res: Response) {
        if (!this.authorize(req, res)) {
            return;
        }

        this.proxyGet("/queue", res);
    }

    /// Refresh one queued dataset. The request stays open until the query ends.
    update(req: Request, res: Response) {
        if (!this.authorize(req, res)) {
            return;
        }

        this.proxyGet("/update", res);
    }

    private authorize(req: Request, res: Response): boolean {
        try {
            this.checkSecret(req);
        } catch (err) {
            logger.info(`Unauthorized attempt to reach ${req.path}`, err);
            res.status(401).send("Unauthorized");
            return false;
        }

        return true;
    }

    /// Forward a GET to the rust backend.
    ///
    /// It uses http.request, not fetch, because fetch drops the connection after
    /// 5 minutes and a dataset query can take longer.
    private proxyGet(path: string, res: Response) {
        const url = new URL(path, BeaconWmsService.getBaseUrl());

        const proxyRequest = request(url, { method: "GET" }, (proxyResponse) => {
            res.status(proxyResponse.statusCode || 502);
            res.setHeader(
                "Content-Type",
                proxyResponse.headers["content-type"] || "application/json"
            );
            proxyResponse.pipe(res);
        });

        proxyRequest.on("error", (err) => {
            logger.error(`Error calling ${url.href}:`, err);

            if (res.headersSent) {
                res.end();
                return;
            }

            res.status(502).json({
                error: "The rust backend did not answer",
                message: err.message,
            });
        });

        proxyRequest.end();
    }

    clearLayers(req: Request, res: Response) {

        try {
            this.checkSecret(req);
        } catch (err) {
            logger.info("Unauthorized attempt to update layers", err);
            res.status(401).send("Unauthorized");
            return;
        }

        const url = new URL('/clear-layers', BeaconWmsService.getBaseUrl());

        fetch(url)
            .then(async (response) => {
                if (!response.ok) {
                    return Promise.reject(response);
                }
                return response.text();
            })
            .then((data) => {
                logger.info("Layers updated", data);
                res.status(200).send(data);
            })
            .catch((err) => {
                let errorMsg = "Unknown error";

                if (err instanceof Response) {
                    errorMsg = `Error ${err.status}: ${err.statusText}`;
                }

                logger.error("Error updating layers:", errorMsg, err);

                // Send a more structured error response
                res.status(err.response?.status || 500).json({
                    error: "Error updating layers",
                    message: errorMsg,
                });
            });
    }
}