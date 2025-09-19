import { Config } from "./config";
import { Utils } from "./utils";
import { Request, Response } from "express";
import { WmsXmlService } from "./wms-xml";
import { WorkspaceConfig } from "../types/config";
import { request } from "http";
import { WMSGetFeatureInfoParameters, WMSGetMapParameters } from "../types/ogc-wms";


export class BeaconWmsService {
    private wmsXml: WmsXmlService;
    private beaconWmsBaseUrl = 'http://localhost:8000'; // Rust service base URL
    private allowedOgcVersions = ['1.1.1', '1.3.0'];

    public static CORS_HEADERS = {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type, Authorization",
        "Access-Control-Max-Age": "86400" // Cache preflight response for 24 hours
    };

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

        const ogcVersion = queryParameters['version'] ?? '1.3.0';

        if (!ogcVersion || !this.allowedOgcVersions.includes(ogcVersion.toString())) {
            this.wmsXml.error(res, "InvalidParameterValue", `The 'version' parameter is required and must be one of: ${this.allowedOgcVersions.join(", ")}, is: '${ogcVersion}'`);
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
        const url = new URL("/available-styles", this.beaconWmsBaseUrl);

        const availableStyles: string[] = await fetch(url)
            .then(r => {
                if (r.ok) {
                    return r.json();
                }
                return Promise.reject(r);
            })
            .catch(response => {
                try{
                    response.text().then((text: string) => console.error(text));
                } catch(_){
                    console.error(response);
                }
                return [];
            });

        if(!availableStyles){
            this.wmsXml.error(res, "ServerError", `Error fetching styles from Rust service`);
            return;
        }

        this.wmsXml.getCapabilities(req, res, workspace, availableStyles, queryParameters['version']);
    }

    private handleGetMap(req: Request, res: Response, workspace: WorkspaceConfig, queryParameters: Record<string, any>) {
        //example uRL: http://localhost:3000/workspaces/default/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetMap&LAYERS=example-layer&STYLES=&CRS=EPSG:4326&BBOX=-4.5,50.0,9.5,62.0&WIDTH=800&HEIGHT=600&FORMAT=image/png
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
            // bgcolor: queryParameters['bgcolor'], // not implemented in rust
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

        const layers = wmsGetMapParams.layers.split(',').map(l => l.trim()).filter(l => l.length > 0);

        for (let i = 0; i < layers.length; i++) {
            const layerId = layers[i];
            const layer = workspace.layers.find(l => l.id === layerId);
            if (!layer) {
                this.wmsXml.error(res, "InvalidParameterValue", `Layer '${layerId}' not found in workspace '${workspace.id}'`);
                return;
            }
        }


        const url = new URL("/get-map", this.beaconWmsBaseUrl);

        url.searchParams.append("workspace", workspace.id);
        url.searchParams.append("version", wmsGetMapParams.version ?? '1.3.0');
        url.searchParams.append("layers", wmsGetMapParams.layers);
        url.searchParams.append("crs", wmsGetMapParams.crs);
        url.searchParams.append("bbox", wmsGetMapParams.bbox);
        url.searchParams.append("width", wmsGetMapParams.width.toString());
        url.searchParams.append("height", wmsGetMapParams.height.toString());
        url.searchParams.append("format", wmsGetMapParams.format);

        if (wmsGetMapParams.styles) url.searchParams.append("styles", wmsGetMapParams.styles);
        // if (wmsGetMapParams.transparent) url.searchParams.append("transparent", wmsGetMapParams.transparent);
        // if (wmsGetMapParams.exceptions) url.searchParams.append("exceptions", wmsGetMapParams.exceptions);
        if (wmsGetMapParams.time) url.searchParams.append("time", wmsGetMapParams.time);
        if (wmsGetMapParams.elevation) url.searchParams.append("elevation", wmsGetMapParams.elevation);

        
        fetch(url)
            .then(r => {
                if (r.ok) {
                    return r.arrayBuffer().then(buf => ({ buf, headers: r.headers }));
                }
                return Promise.reject(r);
            })
            .then(({ buf, headers }) => {
                const nodeBuf = Buffer.from(buf);
                const contentType = headers.get("Content-Type") || "image/png";
                const contentLength = headers.get("Content-Length") || nodeBuf.length.toString();
                const urlHash = Utils.hashCode(url.toString());

                res.writeHead(200, {
                    "Content-Disposition": `inline; filename="map_${urlHash}.png"`,
                    "Content-Type": contentType,
                    "Content-Length": contentLength,
                    ...BeaconWmsService.CORS_HEADERS
                });
                            
                res.end(nodeBuf);
            })
            .catch(response => {
                try{
                    response.text().then((text: string) => console.error(text));
                } catch(_){
                    console.error(response);
                }
                this.wmsXml.error(res, "ServerError", `Error fetching map from Rust service: ${response.statusText || response}`);
            });



    }

    private handleGetFeatureInfo(
        req: Request,
        res: Response,
        workspace: WorkspaceConfig,
        queryParameters: Record<string, any>
    ) {
        //example url: http://10.0.0.33:3000/workspaces/default/wms?SERVICE=WMS&VERSION=1.3.0&REQUEST=GetFeatureInfo&FORMAT=image%2Fpng&TRANSPARENT=true&QUERY_LAYERS=example-layer&LAYERS=example-layer&INFO_FORMAT=text%2Fhtml&FEATURE_COUNT=20&I=232&J=231&WIDTH=256&HEIGHT=256&CRS=EPSG%3A3857&STYLES=&BBOX=-1721973.373208452%2C6261721.357121639%2C-1565430.3392804111%2C6418264.39104968

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

        const layers = wmsGetFeatureInfoParams.layers.split(',').map(l => l.trim()).filter(l => l.length > 0);
        const queryLayers = wmsGetFeatureInfoParams.query_layers.split(',').map(l => l.trim()).filter(l => l.length > 0);
        const allLayers = [...new Set([...layers, ...queryLayers])];

        for (let i = 0; i < allLayers.length; i++) {
            const layerId = allLayers[i];
            const layer = workspace.layers.find(l => l.id === layerId);
            if (!layer) {
                this.wmsXml.error(res, "InvalidParameterValue", `Layer '${layerId}' not found in workspace '${workspace.id}'`);
                return;
            }
        }


        
        const url = new URL("/get-feature-info", this.beaconWmsBaseUrl);

        url.searchParams.append("workspace", workspace.id);
        url.searchParams.append("version", wmsGetFeatureInfoParams.version ?? '1.3.0');
        url.searchParams.append("layers", wmsGetFeatureInfoParams.layers);
        url.searchParams.append("query_layers", wmsGetFeatureInfoParams.query_layers);
        url.searchParams.append("info_format", wmsGetFeatureInfoParams.info_format);
        url.searchParams.append("crs", wmsGetFeatureInfoParams.crs);
        url.searchParams.append("bbox", wmsGetFeatureInfoParams.bbox);
        url.searchParams.append("width", wmsGetFeatureInfoParams.width.toString());
        url.searchParams.append("height", wmsGetFeatureInfoParams.height.toString());
        url.searchParams.append("x", wmsGetFeatureInfoParams.x.toString());
        url.searchParams.append("y", wmsGetFeatureInfoParams.y.toString());


        if (wmsGetFeatureInfoParams.styles) url.searchParams.append("styles", wmsGetFeatureInfoParams.styles);
        // if (wmsGetMapParams.transparent) url.searchParams.append("transparent", wmsGetMapParams.transparent);
        // if (wmsGetMapParams.exceptions) url.searchParams.append("exceptions", wmsGetMapParams.exceptions);
        if (wmsGetFeatureInfoParams.time) url.searchParams.append("time", wmsGetFeatureInfoParams.time);
        if (wmsGetFeatureInfoParams.elevation) url.searchParams.append("elevation", wmsGetFeatureInfoParams.elevation);

        // Placeholder response
        // res.send('GetFeatureInfo request received. Parameters: ' + JSON.stringify(wmsGetFeatureInfoParams));
        // return;
            
        fetch(url)
            .then(r => {
                if (r.ok) {
                    return r.arrayBuffer().then(buf => ({ buf, headers: r.headers }));
                }
                return Promise.reject(r);
            })
            .then(({ buf, headers }) => {
                const nodeBuf = Buffer.from(buf);
                const contentType = headers.get("Content-Type") || "image/png";
                const contentLength = headers.get("Content-Length") || nodeBuf.length.toString();
                const urlHash = Utils.hashCode(url.toString());

                res.writeHead(200, {
                    "Content-Disposition": `inline; filename="get_feature_info_${urlHash}"`,
                    "Content-Type": contentType,
                    "Content-Length": contentLength,
                    ...BeaconWmsService.CORS_HEADERS
                });
                            
                res.end(nodeBuf);
            })
            .catch(response => {
                try{
                    response.text().then((text: string) => console.error(text));
                } catch(_){
                    console.error(response);
                }
                this.wmsXml.error(res, "ServerError", `Error fetching feature info from Rust service: ${response.statusText || response}`);
            });
    }

}