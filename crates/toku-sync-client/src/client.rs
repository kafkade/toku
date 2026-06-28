use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Wire-protocol version this client speaks (account model = 2). Sent on every
/// request so a migrated server can reject pre-account clients with 426 (#126).
pub const SYNC_PROTOCOL_VERSION: i64 = 2;
/// Header carrying [`SYNC_PROTOCOL_VERSION`].
pub const SYNC_PROTOCOL_HEADER: &str = "x-toku-sync-protocol";

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

// ── Account (1Password-style) auth DTOs ──────────────────────────────────────

/// Response from `POST /api/v1/account/signup`.
#[derive(Debug, Deserialize)]
pub struct SignupResult {
    pub user_id: String,
    pub email: String,
    pub role: String,
    /// Relay libraries adopted under this account during admin bootstrap (#126).
    #[serde(default)]
    pub adopted_libraries: i64,
    /// Relay devices adopted under this account during admin bootstrap (#126).
    #[serde(default)]
    pub adopted_devices: i64,
}

/// Response from `POST /api/v1/account/challenge`.
#[derive(Debug, Deserialize)]
pub struct AccountChallengeResult {
    pub challenge_id: String,
    /// Hex-encoded server public ephemeral B.
    pub server_public_b: String,
    /// Hex-encoded SRP salt stored at signup.
    pub srp_salt: String,
}

/// Response from `POST /api/v1/account/verify` — a user-session token.
#[derive(Debug, Deserialize)]
pub struct AccountVerifyResult {
    pub session_token: String,
    /// Hex-encoded server proof M2 — the client MUST verify this.
    pub server_proof_m2: String,
    pub expires_at: String,
    pub user_id: String,
    pub role: String,
}

/// Response from `POST /api/v1/devices/enroll`.
#[derive(Debug, Deserialize)]
pub struct EnrollDeviceResult {
    pub device_id: String,
    pub library_id: String,
    /// `"active"` (ready to sync) or `"pending"` (awaiting approval).
    pub status: String,
    pub session_token: Option<String>,
    pub expires_at: Option<String>,
}

/// Response from `POST /api/v1/devices/{id}/session`.
#[derive(Debug, Deserialize)]
pub struct DeviceSessionResult {
    pub device_id: String,
    pub library_id: String,
    pub session_token: String,
    pub expires_at: String,
}

/// A device as exposed to its owning account (`GET /api/v1/account/devices`).
#[derive(Debug, Deserialize, Serialize)]
pub struct AccountDeviceInfo {
    pub device_id: String,
    pub library_id: String,
    pub device_name: String,
    pub status: String,
    pub last_seen: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
struct AccountDeviceListBody {
    devices: Vec<AccountDeviceInfo>,
}

/// The account key bundle returned by `GET /api/v1/account/keys`.
///
/// **Contract is owned by issue #143** (persist `wrapped_data_key` + expose this
/// endpoint). Until that lands the endpoint does not exist server-side and these
/// calls will fail at runtime; the field names below are this client's assumed
/// contract and must be reconciled with #143 when it is finalized. Each field is
/// an opaque string the client deserializes back into the `toku_core` key types.
#[derive(Debug, Deserialize)]
pub struct AccountKeyBundle {
    /// JSON-serialized `toku_core::AccountKdfParams`.
    pub kdf_params: String,
    /// Base64 account X25519 public key (`AccountKeys::public_key`).
    pub account_public_key: String,
    /// JSON-serialized `toku_core::WrappedAccountPrivateKey`.
    pub wrapped_private_key: String,
    /// JSON-serialized `toku_core::WrappedDataKey`.
    pub wrapped_data_key: String,
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
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            SYNC_PROTOCOL_HEADER,
            reqwest::header::HeaderValue::from_static("2"),
        );
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .default_headers(headers)
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

    // ── Account (1Password-style) auth ───────────────────────────────────────

    /// Create an account on the server (`POST /api/v1/account/signup`).
    ///
    /// The SRP identity is the account `email`; only the verifier + salt are
    /// uploaded, so the server never sees the password. The wrapped key material
    /// is opaque to the server (zero-knowledge): it stores the strings as-is.
    #[allow(clippy::too_many_arguments)]
    pub async fn account_signup(
        &self,
        email: &str,
        srp_salt_hex: &str,
        srp_verifier_hex: &str,
        wrapped_private_key: &str,
        account_public_key: &str,
        kdf_params: &str,
        wrapped_data_key: &str,
    ) -> anyhow::Result<SignupResult> {
        let url = format!("{}/api/v1/account/signup", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "email": email,
                "srp_salt": srp_salt_hex,
                "srp_verifier": srp_verifier_hex,
                "wrapped_private_key": wrapped_private_key,
                "account_public_key": account_public_key,
                "kdf_params": kdf_params,
                // Owned by issue #143; harmless extra field on older servers.
                "wrapped_data_key": wrapped_data_key,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Start an account SRP login (`POST /api/v1/account/challenge`).
    pub async fn account_challenge(
        &self,
        email: &str,
        client_public_a_hex: &str,
    ) -> anyhow::Result<AccountChallengeResult> {
        let url = format!("{}/api/v1/account/challenge", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "email": email,
                "client_public_a": client_public_a_hex,
            }))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Complete an account SRP login (`POST /api/v1/account/verify`). The caller
    /// MUST verify the returned `server_proof_m2` before trusting the token.
    pub async fn account_verify(
        &self,
        challenge_id: &str,
        client_proof_m1_hex: &str,
    ) -> anyhow::Result<AccountVerifyResult> {
        let url = format!("{}/api/v1/account/verify", self.base_url);
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

    /// Fetch the authenticated user's wrapped key bundle
    /// (`GET /api/v1/account/keys`). **Endpoint owned by issue #143.**
    pub async fn account_keys(&self, session_token: &str) -> anyhow::Result<AccountKeyBundle> {
        let url = format!("{}/api/v1/account/keys", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(session_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Enroll a device under the authenticated user (`POST /api/v1/devices/enroll`).
    /// Requires a user-session bearer token (from the account SRP flow).
    pub async fn enroll_device(
        &self,
        session_token: &str,
        library_id: Option<&str>,
        device_name: &str,
        encryption_salt_b64: Option<&str>,
        device_public_key: Option<&str>,
    ) -> anyhow::Result<EnrollDeviceResult> {
        let url = format!("{}/api/v1/devices/enroll", self.base_url);
        let mut body = serde_json::json!({ "device_name": device_name });
        if let Some(lib) = library_id {
            body["library_id"] = serde_json::Value::String(lib.to_string());
        }
        if let Some(salt) = encryption_salt_b64 {
            body["encryption_salt"] = serde_json::Value::String(salt.to_string());
        }
        if let Some(pk) = device_public_key {
            body["device_public_key"] = serde_json::Value::String(pk.to_string());
        }
        let resp = self
            .http
            .post(&url)
            .bearer_auth(session_token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// Mint a fresh device-session token for an already-enrolled, active device
    /// (`POST /api/v1/devices/{id}/session`). Requires a user-session bearer.
    pub async fn create_device_session(
        &self,
        session_token: &str,
        device_id: &str,
    ) -> anyhow::Result<DeviceSessionResult> {
        let url = format!("{}/api/v1/devices/{device_id}/session", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(session_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(resp.json().await?)
    }

    /// List the authenticated user's devices (`GET /api/v1/account/devices`).
    pub async fn list_account_devices(
        &self,
        session_token: &str,
    ) -> anyhow::Result<Vec<AccountDeviceInfo>> {
        let url = format!("{}/api/v1/account/devices", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(session_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        let body: AccountDeviceListBody = resp.json().await?;
        Ok(body.devices)
    }

    /// Deregister one of the authenticated user's devices
    /// (`DELETE /api/v1/account/devices/{id}`).
    pub async fn delete_account_device(
        &self,
        session_token: &str,
        device_id: &str,
    ) -> anyhow::Result<()> {
        let url = format!("{}/api/v1/account/devices/{device_id}", self.base_url);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(session_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(extract_error(resp).await);
        }

        Ok(())
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
