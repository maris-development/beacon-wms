import { Config } from "./config";
import { Utils } from "./utils";
import { Request, Response } from "express";
import { WmsXmlService } from "./wms-xml";
import { WorkspaceConfig } from "../types/config";
import { request } from "http";
import { WMSGetFeatureInfoParameters, WMSGetMapParameters } from "../types/ogc-wms";
import { ParamsDictionary } from "express-serve-static-core";
import { ParsedQs } from "qs";
import { BeaconWmsService } from "./beacon-wms";
import logger from "./logger";

export class AdminService {

    private checkSecret(req: Request): boolean {
        const adminSecret = process.env.ADMIN_SECRET || "";

        if (!adminSecret || adminSecret.trim().length === 0) {
            throw new Error("Environment variable ADMIN_SECRET not set. Please set it to a non-empty value to enable admin endpoints.");
        }

        const authHeader = req.headers["authorization"];

        if (!authHeader) {
            throw new Error("No authorization header");
        }

        const token = authHeader.split(" ")[1];

        if (token !== adminSecret) {
            throw new Error("Invalid token");
        }

        return true;
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