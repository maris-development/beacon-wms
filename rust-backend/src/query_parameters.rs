
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct GetMapRequestParameters {
    // Custom:
    pub workspace: String,
    
    // OGC WMS:
    pub version: String,
    pub layers: String,
    pub crs: String,
    pub bbox: String,
    pub width: u32,
    pub height: u32,
    pub format: String,

    //OGC WMS optional:
    pub styles: Option<String>,
    pub transparent: Option<bool>,
    pub exceptions: Option<String>,
    pub time: Option<String>,
    pub elevation: Option<String>,

    pub viewparams: Option<String> // jaar:2020;otherparam:value
}

#[derive(Deserialize, Debug)]
pub struct GetFeatureInfoRequestParameters {
    // Custom:
    pub workspace: String,
    
    // OGC WMS:
    pub version: String,
    pub layers: String,
    pub query_layers: String,
    pub crs: String,
    pub bbox: String,
    pub width: u32,
    pub height: u32,
    pub info_format: String,
    pub x: u32,
    pub y: u32,

    //OGC WMS optional:
    pub feature_count: Option<u32>,
    pub styles: Option<String>,
    pub transparent: Option<bool>,
    pub exceptions: Option<String>,
    pub time: Option<String>,
    pub elevation: Option<String>,

    pub viewparams: Option<String> // jaar:2020;otherparam:value
}