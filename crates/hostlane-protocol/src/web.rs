use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostProfile {
    pub host: String,
    pub system: String,
    pub cpu: String,
    pub memory: String,
    pub disk: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostNoteParams {
    pub host: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostNote {
    pub host: String,
    pub content: String,
    pub markdown: String,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DomainRoute {
    pub proxy: String,
    pub domain: String,
    pub upstream: String,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct WebReport {
    pub host: String,
    pub routes: Vec<DomainRoute>,
    pub firewall: String,
    pub listeners: Vec<Exposure>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Exposure {
    pub port: u16,
    pub addresses: Vec<String>,
    pub assessment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthCheck {
    pub host: String,
    pub port: u16,
    pub interval_secs: u32,
    pub healthy: Option<bool>,
    pub detail: String,
    #[ts(type = "number")]
    pub checked_at: u64,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HealthParams {
    pub host: String,
    pub port: u16,
    pub interval_secs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReverseForward {
    pub host: String,
    pub remote_port: u16,
    pub local_port: u16,
}
