import { Config } from "./config";
import { Utils } from "./utils";
import { Request, Response } from "express";
import { WmsXmlService } from "./wms-xml";
import { WorkspaceConfig } from "../types/config";
import { request } from "http";
import { WMSGetFeatureInfoParameters, WMSGetMapParameters } from "../types/ogc-wms";


export class BeaconWmsService {
    private wmsXml: WmsXmlService;
    private allowedOgcVersions = ['1.1.1', '1.3.0'];

    constructor(
        private readonly config: Config
    ) {
        this.wmsXml = new WmsXmlService(this.config);
    }

    async handleWmsRequest(req: Request, res: Response) {
        const workspace = req.params['workspaceId'] || '';
        const wsConfig = await this.config.getWorkspaceConfig(workspace);

        if (!wsConfig) {
            this.wmsXml.error(res, "InvalidParameterValue", `Workspace '${workspace}' not found`, undefined, 404);
            return;
        }

        const queryParameters = Utils.lowerCaseKeys(req.query);

        const ogcVersion = queryParameters['version'];

        if (!ogcVersion || !this.allowedOgcVersions.includes(ogcVersion.toString())) {
            this.wmsXml.error(res, "InvalidParameterValue", `The 'version' parameter is required and must be one of: ${this.allowedOgcVersions.join(", ")}`);
            return;
        }

        const ogcService = (queryParameters['service'] || '').toString().toLowerCase();

        if (ogcService !== 'wms') {
            this.wmsXml.error(res, "InvalidParameterValue", "The 'service' parameter must be 'WMS'");
            return;
        }

        const wmsRequest = (queryParameters['request'] || '').toString().toLowerCase();

        if (!wmsRequest) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'request' parameter is required");
            return;
        }

        switch (wmsRequest) {
            case 'getcapabilities':
                this.handleGetCapabilities(req, res, wsConfig, queryParameters);
                break;
            case 'getmap':
                this.handleGetMap(req, res, wsConfig, queryParameters);
                break;
            case 'getfeatureinfo':
                this.handleGetFeatureInfo(req, res, wsConfig, queryParameters);
                break;
            default:
                this.wmsXml.error(res, "InvalidParameterValue", `The 'request' parameter value '${wmsRequest}' is not supported`);
                return;
        }
    }

    private async handleGetCapabilities(req: Request, res: Response, workspace: WorkspaceConfig, queryParameters: Record<string, any>) {
        this.wmsXml.getCapabilities(req, res, workspace, queryParameters['version']);
    }

    private handleGetMap(req: Request, res: Response, workspace: WorkspaceConfig, queryParameters: Record<string, any>) {

        //gather all getMap parameters here from queryParameters (lc keys)

        const wmsGetMapParams: WMSGetMapParameters = {
            service: queryParameters['service'], //WMS
            request: queryParameters['request'], //GetMap
            version: queryParameters['version'],

            layers: queryParameters['layers'],
            styles: queryParameters['styles'],
            crs: queryParameters['crs'] || queryParameters['srs'],
            bbox: queryParameters['bbox'],
            width: queryParameters['width'],
            height: queryParameters['height'],
            format: queryParameters['format'],

            //OPTIONAL:
            transparent: queryParameters['transparent'], //true/false, not implemented in rust
            bgcolor: queryParameters['bgcolor'], // not implemented in rust
            exceptions: queryParameters['exceptions'], // XML or JSON
            time: queryParameters['time'], // optional, not implemented yet
            elevation: queryParameters['elevation'],  //optional, not implemented yet
        };

        if (!wmsGetMapParams.layers) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'layers' parameter is required for GetMap requests");
            return;
        }

        if (wmsGetMapParams.styles) {
            //styles can be empty, but if provided, should match number of layers
            const layerCount = wmsGetMapParams.layers.split(',').length;
            const styleCount = wmsGetMapParams.styles.split(',').length;
            if (styleCount !== layerCount) {
                this.wmsXml.error(res, "InvalidParameterValue", "The 'styles' parameter must have the same number of entries as the 'layers' parameter");
                return;
            }
        }


        if (!wmsGetMapParams.crs) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'crs' (or 'srs') parameter is required for GetMap requests");
            return;
        }

        if (!wmsGetMapParams.bbox) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'bbox' parameter is required for GetMap requests");
            return;
        }

        if (!wmsGetMapParams.width || !wmsGetMapParams.height) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'width' and 'height' parameters are required for GetMap requests");
            return;
        }

        if (!wmsGetMapParams.format) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'format' parameter is required for GetMap requests");
            return;
        }

        const layers =  wmsGetMapParams.layers.split(',').map(l => l.trim()).filter(l => l.length > 0);

        for(let i = 0; i < layers.length; i++) {
            const layerId = layers[i];
            const layer = workspace.layers.find(l => l.id === layerId);
            if (!layer) {
                this.wmsXml.error(res, "InvalidParameterValue", `Layer '${layerId}' not found in workspace '${workspace.id}'`);
                return;
            }   
        }

        //placeholder:
        res.send('GetMap request received. Parameters: ' + JSON.stringify(wmsGetMapParams));
    }

    private handleGetFeatureInfo(
        req: Request,
        res: Response,
        workspace: WorkspaceConfig,
        queryParameters: Record<string, any>
    ) {
        const wmsGetFeatureInfoParams: WMSGetFeatureInfoParameters = {
            service: queryParameters['service'],
            request: queryParameters['request'],
            version: queryParameters['version'],
            layers: queryParameters['layers'],
            query_layers: queryParameters['query_layers'],
            info_format: queryParameters['info_format'],
            crs: queryParameters['crs'] || queryParameters['srs'],
            bbox: queryParameters['bbox'],
            width: queryParameters['width'],
            height: queryParameters['height'],
            x: queryParameters['x'] ?? queryParameters['i'],
            y: queryParameters['y'] ?? queryParameters['j'],
            styles: queryParameters['styles'],
            feature_count: queryParameters['feature_count'],
            exceptions: queryParameters['exceptions'],
            time: queryParameters['time'],
            elevation: queryParameters['elevation'],
        };

        // Validate mandatory parameters
        if (!wmsGetFeatureInfoParams.layers) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'layers' parameter is required for GetFeatureInfo requests");
            return;
        }
        if (!wmsGetFeatureInfoParams.query_layers) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'query_layers' parameter is required for GetFeatureInfo requests");
            return;
        }
        if (!wmsGetFeatureInfoParams.info_format) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'info_format' parameter is required for GetFeatureInfo requests");
            return;
        }
        if (!wmsGetFeatureInfoParams.crs) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'crs' (or 'srs') parameter is required for GetFeatureInfo requests");
            return;
        }
        if (!wmsGetFeatureInfoParams.bbox) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'bbox' parameter is required for GetFeatureInfo requests");
            return;
        }
        if (!wmsGetFeatureInfoParams.width || !wmsGetFeatureInfoParams.height) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'width' and 'height' parameters are required for GetFeatureInfo requests");
            return;
        }
        if (!wmsGetFeatureInfoParams.x || !wmsGetFeatureInfoParams.y) {
            this.wmsXml.error(res, "MissingParameterValue", "The 'x' and 'y' (or 'i' and 'j') parameters are required for GetFeatureInfo requests");
            return;
        }

        const layers =  wmsGetFeatureInfoParams.layers.split(',').map(l => l.trim()).filter(l => l.length > 0);
        const queryLayers =  wmsGetFeatureInfoParams.query_layers.split(',').map(l => l.trim()).filter(l => l.length > 0);
        const allLayers = [...new Set([...layers, ...queryLayers])];

        for(let i = 0; i < allLayers.length; i++) {
            const layerId = allLayers[i];
            const layer = workspace.layers.find(l => l.id === layerId);
            if (!layer) {
                this.wmsXml.error(res, "InvalidParameterValue", `Layer '${layerId}' not found in workspace '${workspace.id}'`);
                return;
            }   
        }

    

        // Placeholder response
        res.send('GetFeatureInfo request received. Parameters: ' + JSON.stringify(wmsGetFeatureInfoParams));
    }

}