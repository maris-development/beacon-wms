use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigFile {
    pub server: Option<ServerConfig>,
    pub workspaces: Option<Vec<WorkspaceConfig>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub title: Option<String>,
    pub description: Option<String>,
    pub contact: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub contact: Option<String>,
    pub layers: Vec<LayerConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerConfig {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub config: LayerInnerConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerInnerConfig {
    pub dimensions: Option<HashMap<String, DimensionConfig>>,

    pub available_viewparams: Option<HashMap<String, Value>>,

    // #[serde(skip)]
    pub assigned_viewparams: Option<HashMap<String, Value>>,

    pub default_style: Option<String>,
    pub instance_url: String,
    pub token: String,
    pub query: HashMap<String, Value>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub shape: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DimensionConfig {
    pub accepted: Option<AcceptedValues>
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum AcceptedValues {
    Single(String),
    Multiple(Vec<String>),
}
