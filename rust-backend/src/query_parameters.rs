
use serde::Deserialize;
use sha2::Digest;

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

impl GetMapRequestParameters {
    pub fn hash(&self) -> String {
        let mut hasher = sha2::Sha256::new();
        hasher.update(self.workspace.as_bytes());
        hasher.update(self.version.as_bytes());
        hasher.update(self.layers.as_bytes());
        hasher.update(self.crs.as_bytes());
        hasher.update(self.bbox.as_bytes());
        hasher.update(self.width.to_le_bytes());
        hasher.update(self.height.to_le_bytes());
        hasher.update(self.format.as_bytes());

        if let Some(styles) = &self.styles {
            hasher.update(styles.as_bytes());
        }
        if let Some(transparent) = self.transparent {
            hasher.update(&[transparent as u8]);
        }
        if let Some(exceptions) = &self.exceptions {
            hasher.update(exceptions.as_bytes());
        }
        if let Some(time) = &self.time {
            hasher.update(time.as_bytes());
        }
        if let Some(elevation) = &self.elevation {
            hasher.update(elevation.as_bytes());
        }
        if let Some(viewparams) = &self.viewparams {
            hasher.update(viewparams.as_bytes());
        }

        format!("{:x}", hasher.finalize())
    }
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

#[derive(Deserialize, Debug)]
pub struct GetLegendGraphicRequestParameters {
    // Custom:
    pub workspace: String,

    // OGC WMS:
    pub layer: String,

    // OGC WMS optional:
    pub style: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}