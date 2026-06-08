use std::time::Duration;

use serde::{Deserialize, Serialize};

/// HTTP client for communicating with the toku-sync server.
pub struct SyncClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub device_id: String,
    pub library_id: String,
    pub auth_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: String,
    pub last_seen: Option<String>,
    pub created_at: String,
}

/// Wire format for a sync op (matches server's OpPayload).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireOp {
    pub op_id: String,
    pub device_id: String,
    pub hlc: String,
    pub entity_type: String,
    pub entity_id: String,
    pub op_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct PushResult {
    pub accepted: usize,
    pub duplicates: usize,
    pub new_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PullResult {
    pub ops: Vec<WireOp>,
    pub cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Deserialize)]
pub struct RekeyResult {
    pub ops_replaced: usize,
    #[allow(dead_code)]
    pub new_salt: String,
}

#[derive(Debug, Deserialize)]
pub struct SaltResult {
    pub salt: Option<String>,
}

impl SyncClient {
    pub fn new(server_url: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()?;

        let mut base = server_url.to_string();
        while base.ends_with('/') {
            base.pop();
        }

        Ok(Self {
            http,
            base_url: base,
        })
    }

    /// Register a new device with the sync server.
    pub async fn register(
        &self,
        library_id: &str,
        device_name: &str,
    ) -> anyhow::Result<RegisterResponse> {
        let url = format!("{}/api/v1/register", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "library_id": library_id,
                "device_name": device_name,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// List all devices registered to this library.
    pub async fn list_devices(&self, token: &str) -> anyhow::Result<Vec<DeviceInfo>> {
        let url = format!("{}/api/v1/devices", self.base_url);
        let resp = self.http.get(&url).bearer_auth(token).send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Deregister a device from the sync server.
    pub async fn deregister_device(&self, token: &str, device_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/devices/{device_id}", self.base_url);
        let resp = self.http.delete(&url).bearer_auth(token).send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(())
    }

    /// Push a batch of ops to the sync server.
    pub async fn push_ops(&self, token: &str, ops: &[WireOp]) -> anyhow::Result<PushResult> {
        let url = format!("{}/api/v1/push", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({ "ops": ops }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Pull ops from the sync server since the given cursor.
    pub async fn pull_ops(&self, token: &str, since: Option<&str>) -> anyhow::Result<PullResult> {
        let mut url = format!("{}/api/v1/pull", self.base_url);
        if let Some(cursor) = since {
            url = format!("{url}?since={cursor}");
        }

        let resp = self.http.get(&url).bearer_auth(token).send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Pull ALL ops for this library (including own device's).
    /// Used during re-keying.
    pub async fn pull_all_ops(&self, token: &str) -> anyhow::Result<PullResult> {
        let url = format!("{}/api/v1/pull/all", self.base_url);
        let resp = self.http.get(&url).bearer_auth(token).send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Get the library salt for key derivation.
    pub async fn get_salt(&self, token: &str) -> anyhow::Result<SaltResult> {
        let url = format!("{}/api/v1/salt", self.base_url);
        let resp = self.http.get(&url).bearer_auth(token).send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Submit re-encrypted ops and new salt to the server.
    pub async fn rekey(
        &self,
        token: &str,
        new_salt: &str,
        ops: &[WireOp],
    ) -> anyhow::Result<RekeyResult> {
        let url = format!("{}/api/v1/rekey", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({
                "new_salt": new_salt,
                "ops": ops,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }
}

async fn extract_error(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    match resp.json::<ErrorBody>().await {
        Ok(body) => anyhow::anyhow!("server error ({status}): {}", body.error),
        Err(_) => anyhow::anyhow!("server error ({status})"),
    }
}
