use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct LayerConfig {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub config: LayerInnerConfig,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LayerInnerConfig {
    pub instance_url: String,
    pub token: String,
    pub query: HashMap<String, serde_json::Value>,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
}
