use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    #[serde(rename = "trackId")]
    pub id: i64,
    #[serde(rename = "bundleId")]
    pub bundle_id: String,
    #[serde(rename = "trackName")]
    pub name: String,
    pub version: Option<String>,
    #[serde(default)]
    pub price: f64,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    #[serde(rename = "resultCount")]
    pub count: u32,
    pub results: Vec<App>,
}
