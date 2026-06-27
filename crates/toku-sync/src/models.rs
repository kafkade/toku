use serde::{Deserialize, Serialize};

// ── Requests ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub library_id: String,
    pub device_name: String,
    /// Optional base64-encoded key-derivation salt. The first device to
    /// register a library with encryption enabled establishes the salt for
    /// the whole library; later devices fetch it via `GET /salt`.
    #[serde(default)]
    pub salt: Option<String>,
}

/// Enroll the first device for a library using SRP-6a.
/// The client uploads only the verifier and salt — never the password.
#[derive(Debug, Deserialize)]
pub struct EnrollRequest {
    pub library_id: String,
    pub device_name: String,
    /// Hex-encoded 16-byte random salt used to compute the SRP verifier.
    pub srp_salt: String,
    /// Hex-encoded SRP verifier `v = g^x mod N` (up to 256 bytes for G_2048).
    pub srp_verifier: String,
    /// Optional base64-encoded 16-byte salt for client-side encryption key
    /// derivation. Stored in `libraries.salt`; first writer wins.
    #[serde(default)]
    pub encryption_salt: Option<String>,
}

/// Start an SRP-6a login: client sends its public ephemeral A.
#[derive(Debug, Deserialize)]
pub struct SrpChallengeRequest {
    pub library_id: String,
    /// Hex-encoded client public ephemeral A (`g^a mod N`).
    pub client_public_a: String,
}

/// Complete an SRP-6a login: client sends proof M1.
#[derive(Debug, Deserialize)]
pub struct SrpVerifyRequest {
    pub challenge_id: String,
    /// Hex-encoded client proof M1.
    pub client_proof_m1: String,
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

#[derive(Debug, Deserialize)]
pub struct RekeyRequest {
    pub new_salt: String,
    pub ops: Vec<OpPayload>,
}

#[derive(Debug, Deserialize)]
pub struct UploadSnapshotRequest {
    pub snapshot_json: String,
    pub hlc_at_snapshot: String,
}

// ── Responses ───────────────────────────────────────────────────────────────

/// Response to `POST /api/v1/auth/enroll`.
#[derive(Debug, Serialize)]
pub struct EnrollResponse {
    pub device_id: String,
    pub library_id: String,
}

/// Response to `POST /api/v1/auth/challenge`.
#[derive(Debug, Serialize)]
pub struct SrpChallengeResponse {
    pub challenge_id: String,
    /// Hex-encoded server public ephemeral B.
    pub server_public_b: String,
    /// Hex-encoded SRP salt stored at enrollment. The client needs this to
    /// recompute `x = H(salt || H(library_id || ":" || password))`.
    pub srp_salt: String,
}

/// Response to `POST /api/v1/auth/verify`.
#[derive(Debug, Serialize)]
pub struct SrpVerifyResponse {
    /// The session bearer token — store in the OS keychain, use for all subsequent API calls.
    pub session_token: String,
    /// Hex-encoded server proof M2. Client MUST verify this to confirm the server knows the verifier.
    pub server_proof_m2: String,
    pub expires_at: String,
    pub device_id: String,
    pub library_id: String,
}

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

#[derive(Debug, Serialize)]
pub struct RekeyResponse {
    pub ops_replaced: usize,
    pub new_salt: String,
}

#[derive(Debug, Serialize)]
pub struct UploadSnapshotResponse {
    pub ops_pruned: usize,
    pub hlc_at_snapshot: String,
}

#[derive(Debug, Serialize)]
pub struct DownloadSnapshotResponse {
    pub snapshot_json: String,
    pub hlc_at_snapshot: String,
    pub created_at: String,
    pub created_by_device: String,
}

// ── User accounts & admin (issue #119) ───────────────────────────────────────

/// `POST /api/v1/account/signup` — create a user account.
///
/// The client computes the SRP verifier locally from `(Secret Key + password)`
/// and uploads only the verifier + salt (never the secrets). Wrapped key
/// material from the key hierarchy (#116) is stored opaquely.
#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub email: String,
    /// Hex-encoded SRP salt used to compute the verifier.
    pub srp_salt: String,
    /// Hex-encoded SRP verifier `v = g^x mod N`.
    pub srp_verifier: String,
    /// Opaque wrapped account private key (AES-256-GCM under the unlock key).
    #[serde(default)]
    pub wrapped_private_key: Option<String>,
    /// Account public key (X25519).
    #[serde(default)]
    pub account_public_key: Option<String>,
    /// Versioned KDF parameter blob.
    #[serde(default)]
    pub kdf_params: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SignupResponse {
    pub user_id: String,
    pub email: String,
    pub role: String,
}

/// `POST /api/v1/account/challenge` — start a user SRP login.
#[derive(Debug, Deserialize)]
pub struct AccountChallengeRequest {
    pub email: String,
    /// Hex-encoded client public ephemeral A (`g^a mod N`).
    pub client_public_a: String,
}

#[derive(Debug, Serialize)]
pub struct AccountChallengeResponse {
    pub challenge_id: String,
    /// Hex-encoded server public ephemeral B.
    pub server_public_b: String,
    /// Hex-encoded SRP salt stored at signup.
    pub srp_salt: String,
}

/// `POST /api/v1/account/verify` — complete a user SRP login.
#[derive(Debug, Deserialize)]
pub struct AccountVerifyRequest {
    pub challenge_id: String,
    /// Hex-encoded client proof M1.
    pub client_proof_m1: String,
}

#[derive(Debug, Serialize)]
pub struct AccountVerifyResponse {
    /// User session bearer token — store securely, use for account/admin calls.
    pub session_token: String,
    /// Hex-encoded server proof M2. The client MUST verify this.
    pub server_proof_m2: String,
    pub expires_at: String,
    pub user_id: String,
    pub role: String,
}

/// A single user, as exposed to admins. Never includes verifier/key material.
#[derive(Debug, Serialize)]
pub struct UserSummary {
    pub id: String,
    pub email: String,
    pub role: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub users: Vec<UserSummary>,
}

/// `POST /api/v1/admin/users/{id}/status` — enable/disable a user.
#[derive(Debug, Deserialize)]
pub struct SetUserStatusRequest {
    /// `"active"` or `"disabled"`.
    pub status: String,
}

/// `GET/PUT /api/v1/admin/registration` — read/toggle open registration.
#[derive(Debug, Serialize)]
pub struct RegistrationConfigResponse {
    pub registration_open: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetRegistrationRequest {
    pub open: bool,
}
