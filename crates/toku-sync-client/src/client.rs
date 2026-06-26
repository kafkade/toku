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

/// Response from `POST /api/v1/auth/enroll`.
#[derive(Debug, Deserialize)]
pub struct EnrollResponse {
    pub device_id: String,
    pub library_id: String,
}

/// Response from `POST /api/v1/auth/challenge`.
#[derive(Debug, Deserialize)]
pub struct SrpChallengeResponse {
    pub challenge_id: String,
    /// Hex-encoded server public ephemeral B.
    pub server_public_b: String,
    /// Hex-encoded SRP salt stored at enrollment. Used by the client to
    /// recompute `x = H(salt || H(library_id || ":" || password))`.
    pub srp_salt: String,
}

/// Response from `POST /api/v1/auth/verify`.
#[derive(Debug, Deserialize)]
pub struct SrpVerifyResponse {
    pub session_token: String,
    /// Hex-encoded server proof M2 — the client MUST verify this.
    pub server_proof_m2: String,
    pub expires_at: String,
    pub device_id: String,
    pub library_id: String,
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

#[derive(Debug, Deserialize)]
pub struct UploadSnapshotResult {
    pub ops_pruned: usize,
    #[allow(dead_code)]
    pub hlc_at_snapshot: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DownloadSnapshotResult {
    pub snapshot_json: String,
    pub hlc_at_snapshot: String,
    #[allow(dead_code)]
    pub created_at: String,
    #[allow(dead_code)]
    pub created_by_device: String,
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

    /// Register a new device with the sync server. When `salt` is provided
    /// (base64-encoded), it is offered as the library's key-derivation salt;
    /// the server keeps it only if the library has no salt yet.
    ///
    /// For SRP-protected libraries (created via [`enroll`]) pass `session_token`
    /// as a Bearer token; for passwordless libraries omit it (`None`).
    pub async fn register(
        &self,
        library_id: &str,
        device_name: &str,
        salt: Option<&str>,
        session_token: Option<&str>,
    ) -> anyhow::Result<RegisterResponse> {
        let url = format!("{}/api/v1/register", self.base_url);
        let mut body = serde_json::json!({
            "library_id": library_id,
            "device_name": device_name,
        });
        if let Some(salt) = salt {
            body["salt"] = serde_json::Value::String(salt.to_string());
        }
        let mut req = self.http.post(&url).json(&body);
        if let Some(token) = session_token {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    // ── SRP-6a authentication ────────────────────────────────────────────────

    /// Enroll the first device for a library with SRP-6a. The client provides
    /// only the verifier (`v = g^x mod N`, hex-encoded) and salt — the password
    /// never leaves the client. Pass `encryption_salt` (base64) to set the
    /// library-wide key-derivation salt; the server keeps it if not already set.
    pub async fn enroll(
        &self,
        library_id: &str,
        device_name: &str,
        srp_salt_hex: &str,
        srp_verifier_hex: &str,
        encryption_salt_b64: Option<&str>,
    ) -> anyhow::Result<EnrollResponse> {
        let url = format!("{}/api/v1/auth/enroll", self.base_url);
        let mut body = serde_json::json!({
            "library_id": library_id,
            "device_name": device_name,
            "srp_salt": srp_salt_hex,
            "srp_verifier": srp_verifier_hex,
        });
        if let Some(salt) = encryption_salt_b64 {
            body["encryption_salt"] = serde_json::Value::String(salt.to_string());
        }
        let resp = self.http.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Start an SRP-6a login. Send client public ephemeral A; receive server
    /// ephemeral B and a `challenge_id` to use in [`srp_verify`].
    pub async fn srp_challenge(
        &self,
        library_id: &str,
        client_public_a_hex: &str,
    ) -> anyhow::Result<SrpChallengeResponse> {
        let url = format!("{}/api/v1/auth/challenge", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "library_id": library_id,
                "client_public_a": client_public_a_hex,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Complete an SRP-6a login. Send client proof M1; receive session token
    /// and server proof M2. The caller MUST verify M2 before trusting the
    /// session token.
    pub async fn srp_verify(
        &self,
        challenge_id: &str,
        client_proof_m1_hex: &str,
    ) -> anyhow::Result<SrpVerifyResponse> {
        let url = format!("{}/api/v1/auth/verify", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "challenge_id": challenge_id,
                "client_proof_m1": client_proof_m1_hex,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    // ── Devices ──────────────────────────────────────────────────────────────

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

    /// Upload a snapshot to the server and prune old ops.
    pub async fn upload_snapshot(
        &self,
        token: &str,
        snapshot_json: &str,
        hlc_at_snapshot: &str,
    ) -> anyhow::Result<UploadSnapshotResult> {
        let url = format!("{}/api/v1/snapshot", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(token)
            .json(&serde_json::json!({
                "snapshot_json": snapshot_json,
                "hlc_at_snapshot": hlc_at_snapshot,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Download the latest snapshot from the server.
    #[allow(dead_code)]
    pub async fn download_snapshot(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<DownloadSnapshotResult>> {
        let url = format!("{}/api/v1/snapshot", self.base_url);
        let resp = self.http.get(&url).bearer_auth(token).send().await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(Some(resp.json().await?))
    }
}

async fn extract_error(resp: reqwest::Response) -> anyhow::Error {
    let status = resp.status();
    match resp.json::<ErrorBody>().await {
        Ok(body) => anyhow::anyhow!("server error ({status}): {}", body.error),
        Err(_) => anyhow::anyhow!("server error ({status})"),
    }
}
