use serde::{Deserialize, Serialize};

// ── Requests ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub library_id: String,
    pub device_name: String,
}

#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub ops: Vec<OpPayload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpPayload {
    pub op_id: String,
    pub device_id: String,
    pub hlc: String,
    pub entity_type: String,
    pub entity_id: String,
    pub op_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct PullQuery {
    pub since: Option<String>,
}

// ── Responses ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub device_id: String,
    pub library_id: String,
    pub auth_token: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceResponse {
    pub device_id: String,
    pub device_name: String,
    pub last_seen: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PushResponse {
    pub accepted: usize,
    pub duplicates: usize,
    pub new_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PullResponse {
    pub ops: Vec<OpPayload>,
    pub cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}
