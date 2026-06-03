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

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: String,
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
}

async fn extract_error(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    match resp.json::<ErrorBody>().await {
        Ok(body) => anyhow::anyhow!("server error ({status}): {}", body.error),
        Err(_) => anyhow::anyhow!("server error ({status})"),
    }
}
