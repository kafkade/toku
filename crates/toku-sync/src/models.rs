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
    /// Zero-knowledge: in hosted mode this is always an encrypted envelope
    /// (`{ev, alg, nonce, ciphertext, aad}`) or `null` for content-free ops.
    /// The server rejects plaintext payloads (issue #121).
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
    /// Zero-knowledge: a serialized encrypted envelope over the
    /// `LibrarySnapshot` JSON. The server rejects plaintext snapshots (#121).
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
    /// Wire protocol this server speaks.
    pub protocol_version: i64,
    /// Lowest client protocol the server currently accepts.
    pub min_protocol: i64,
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
    /// Opaque wrapped library data key (the leaf `SyncKey`, wrapped to the
    /// account public key). Stored as-is; never derivable server-side (#143).
    #[serde(default)]
    pub wrapped_data_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SignupResponse {
    pub user_id: String,
    pub email: String,
    pub role: String,
    /// Relay libraries adopted under this account during first-admin bootstrap
    /// (#126). Zero for every signup except the migrating admin.
    #[serde(default)]
    pub adopted_libraries: i64,
    /// Relay devices adopted under this account during first-admin bootstrap.
    #[serde(default)]
    pub adopted_devices: i64,
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

/// `GET/PUT /api/v1/admin/device-approvals` — read/toggle the device-approval gate.
#[derive(Debug, Serialize)]
pub struct DeviceApprovalsConfigResponse {
    pub device_approvals_required: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetDeviceApprovalsRequest {
    pub required: bool,
}

// ── Authenticated device enrollment (issue #120) ─────────────────────────────

/// `POST /api/v1/devices/enroll` — enroll a device under the authenticated user.
///
/// The caller must present a user-session bearer (obtained via the account SRP
/// flow, which already proves possession of password + Secret Key). The device
/// is bound to the user's library; no library is auto-created for an
/// unauthenticated caller.
#[derive(Debug, Deserialize)]
pub struct EnrollDeviceRequest {
    /// Target library. Omit to create a fresh library owned by the user; when
    /// supplied it must be a library the authenticated user owns (or one that
    /// does not exist yet, in which case it is created owned by the user).
    #[serde(default)]
    pub library_id: Option<String>,
    pub device_name: String,
    /// Optional base64-encoded library key-derivation salt; first writer wins.
    #[serde(default)]
    pub encryption_salt: Option<String>,
    /// Optional hex/base64 device public key (X25519); stored opaquely.
    #[serde(default)]
    pub device_public_key: Option<String>,
}

/// Response to `POST /api/v1/devices/enroll`.
///
/// When the device is immediately `active`, `session_token` + `expires_at` are
/// populated and the device can sync right away. When `status` is `pending`
/// (approval flow), both are `None` until an existing trusted device approves
/// it, after which the device mints a token via `POST /devices/{id}/session`.
#[derive(Debug, Serialize)]
pub struct EnrollDeviceResponse {
    pub device_id: String,
    pub library_id: String,
    pub status: String,
    pub session_token: Option<String>,
    pub expires_at: Option<String>,
}

/// `POST /api/v1/devices/{id}/approval` — approve or reject a pending device.
#[derive(Debug, Deserialize)]
pub struct DeviceApprovalRequest {
    /// `"approve"` or `"reject"`.
    pub decision: String,
}

/// Response to `POST /api/v1/devices/{id}/session` — a freshly minted device
/// session token for an approved (active) device.
#[derive(Debug, Serialize)]
pub struct DeviceSessionResponse {
    pub device_id: String,
    pub library_id: String,
    pub session_token: String,
    pub expires_at: String,
}

/// A device as exposed to its owning account. Never includes token material.
#[derive(Debug, Serialize)]
pub struct AccountDeviceSummary {
    pub device_id: String,
    pub library_id: String,
    pub device_name: String,
    pub status: String,
    pub last_seen: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct AccountDeviceListResponse {
    pub devices: Vec<AccountDeviceSummary>,
}

/// `GET /api/v1/account/keys` — the account key bundle a new device needs to
/// unlock the shared library data key the zero-knowledge way (issue #143).
///
/// All four fields are opaque strings persisted verbatim at signup. The server
/// stores and returns only ciphertext plus the account public key; it can never
/// derive or read the plaintext data key. Fields are required (non-null) on the
/// 200 path — an account missing any of them yields `409 Conflict` instead.
#[derive(Debug, Serialize)]
pub struct AccountKeysResponse {
    /// JSON-serialized `toku_core::AccountKdfParams`.
    pub kdf_params: String,
    /// Base64-encoded account X25519 public key.
    pub account_public_key: String,
    /// JSON-serialized `toku_core::WrappedAccountPrivateKey`.
    pub wrapped_private_key: String,
    /// JSON-serialized `toku_core::WrappedDataKey`.
    pub wrapped_data_key: String,
}
