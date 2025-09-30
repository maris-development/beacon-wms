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


export class AdminService {

    private config: Config;

    constructor(config: Config) {
        this.config = config;
    }

    private checkSecret(req: Request): boolean {
        const secret = this.config.getSecret();

        if (!secret || secret.trim().length === 0) {
            throw new Error("No secret set in config");
        }

        const authHeader = req.headers["authorization"];

        if (!authHeader) {
            throw new Error("No authorization header");
        }

        const token = authHeader.split(" ")[1];

        if (token !== secret) {
            throw new Error("Invalid token");
        }

        return true;
    }

    updateLayers(req: Request, res: Response) {

        try {
            this.checkSecret(req);
        } catch (err) {
            console.log("Unauthorized attempt to update layers", err);
            res.status(401).send("Unauthorized");
            return;
        }

        const url = new URL('/update-layers', BeaconWmsService.getBaseUrl());

        fetch(url)
            .then(async (response) => {
                if (!response.ok) {
                    // Always read the response body as text, even for non-OK responses
                    const text = await response.text();
                    throw new Error(
                        `Error updating layers: ${response.status} ${response.statusText} - ${text}`
                    );
                }
                return response.text();
            })
            .then((data) => {
                console.log("Layers updated", data);
                res.status(200).send(data);
            })
            .catch((err) => {
                console.error("Error updating layers:", err);
                // Send a more structured error response
                res.status(err.response?.status || 500).json({
                    error: "Error updating layers",
                    message: err.message,
                    status: err.response?.status,
                });
            });


    }
}