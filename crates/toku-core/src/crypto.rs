//! Client-side encryption for sync ops.
//!
//! Implements optional AES-256-GCM encryption with Argon2id key derivation,
//! per ADR-006 and ADR-008. When enabled, the `fields` JSON is encrypted
//! before leaving the device; the server stores opaque blobs.
//!
//! # Key derivation
//!
//! Passphrase → Argon2id (m=64MB, t=3, p=1) → 256-bit AES key.
//! Salt is 128-bit random, generated once per library.
//!
//! # Encryption envelope
//!
//! Each op's `fields` JSON is encrypted with AES-256-GCM using a random
//! 96-bit nonce. Additional Authenticated Data (AAD) binds the envelope
//! version, entity type, entity ID, and op type to prevent payload swapping.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::TokuError;
use crate::sync::{EntityType, OpType};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Argon2id memory cost in KiB (64 MB).
const ARGON2_M_COST: u32 = 65536;
/// Argon2id time cost (iterations).
const ARGON2_T_COST: u32 = 3;
/// Argon2id parallelism.
const ARGON2_P_COST: u32 = 1;

/// Current encryption envelope version.
const ENCRYPTION_ENVELOPE_VERSION: u16 = 1;

/// Algorithm identifier for the envelope.
const ALGORITHM: &str = "aes-256-gcm";

// ---------------------------------------------------------------------------
// SyncKey
// ---------------------------------------------------------------------------

/// A 256-bit AES key derived from a user passphrase.
///
/// Key material is zeroed on drop to minimize exposure in memory.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SyncKey([u8; 32]);

impl SyncKey {
    /// Derive a sync key from a passphrase and salt using Argon2id.
    ///
    /// Parameters: m=64MB, t=3, p=1 (per ADR-008).
    pub fn derive(passphrase: &str, salt: &[u8; 16]) -> Result<Self, TokuError> {
        let params = argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32))
            .map_err(|e| TokuError::Crypto(format!("argon2 params: {e}")))?;
        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

        let mut key = [0u8; 32];
        argon2
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .map_err(|e| TokuError::Crypto(format!("key derivation failed: {e}")))?;

        Ok(Self(key))
    }

    /// Generate a random 128-bit salt for key derivation.
    pub fn generate_salt() -> [u8; 16] {
        let mut salt = [0u8; 16];
        getrandom::fill(&mut salt).expect("OS CSPRNG failed");
        salt
    }

    /// Access the raw key bytes (internal use only for AES-GCM).
    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Export the raw key bytes for keychain storage.
    ///
    /// The caller is responsible for securely storing these bytes.
    pub fn as_exported_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Reconstruct a key from raw exported bytes (e.g. loaded from the keychain
    /// or file store). Expects exactly 32 bytes.
    pub fn from_exported_bytes(bytes: &[u8]) -> Result<Self, TokuError> {
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| TokuError::Crypto("sync key must be exactly 32 bytes".to_string()))?;
        Ok(Self(arr))
    }
}

impl std::fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncKey([REDACTED])")
    }
}

// ---------------------------------------------------------------------------
// Encrypted Envelope
// ---------------------------------------------------------------------------

/// An encrypted payload replacing `fields` in a sync op.
///
/// Per ADR-008, the envelope contains the algorithm, nonce, ciphertext,
/// and AAD string that binds the op metadata to the ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedEnvelope {
    /// Encryption envelope version (currently 1).
    pub ev: u16,
    /// Algorithm identifier (`"aes-256-gcm"`).
    pub alg: String,
    /// Base64-encoded 96-bit random nonce.
    pub nonce: String,
    /// Base64-encoded encrypted fields JSON.
    pub ciphertext: String,
    /// Additional Authenticated Data string bound to the ciphertext.
    pub aad: String,
}

// ---------------------------------------------------------------------------
// Encrypt / Decrypt
// ---------------------------------------------------------------------------

/// Build the canonical AAD string for an op.
///
/// Binds envelope version, entity type, entity ID, and op type to the
/// ciphertext. This prevents an attacker from swapping encrypted payloads
/// between ops or changing op metadata without detection.
pub fn build_aad(
    envelope_version: u16,
    entity_type: &EntityType,
    entity_id: &uuid::Uuid,
    op_type: &OpType,
) -> String {
    format!(
        "v={},entity_type={},entity_id={},op_type={}",
        envelope_version,
        entity_type.as_str(),
        entity_id,
        op_type.as_str(),
    )
}

/// Encrypt a `fields` JSON value into an [`EncryptedEnvelope`].
///
/// Generates a random 96-bit nonce via OS CSPRNG. The AAD binds the
/// envelope version, entity type, entity ID, and op type.
pub fn encrypt_fields(
    key: &SyncKey,
    fields: &serde_json::Value,
    entity_type: &EntityType,
    entity_id: &uuid::Uuid,
    op_type: &OpType,
) -> Result<EncryptedEnvelope, TokuError> {
    let plaintext =
        serde_json::to_vec(fields).map_err(|e| TokuError::Crypto(format!("serialize: {e}")))?;

    let aad_str = build_aad(ENCRYPTION_ENVELOPE_VERSION, entity_type, entity_id, op_type);

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| TokuError::Crypto(format!("cipher init: {e}")))?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::fill(&mut nonce_bytes).expect("OS CSPRNG failed");
    let nonce = Nonce::from_slice(&nonce_bytes);

    let payload = Payload {
        msg: &plaintext,
        aad: aad_str.as_bytes(),
    };

    let ciphertext = cipher
        .encrypt(nonce, payload)
        .map_err(|e| TokuError::Crypto(format!("encryption failed: {e}")))?;

    Ok(EncryptedEnvelope {
        ev: ENCRYPTION_ENVELOPE_VERSION,
        alg: ALGORITHM.to_string(),
        nonce: BASE64_STANDARD.encode(nonce_bytes),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
        aad: aad_str,
    })
}

/// Decrypt an [`EncryptedEnvelope`] back into a `fields` JSON value.
///
/// Recomputes the expected AAD from op metadata and verifies it matches
/// the stored AAD before decryption. This ensures the envelope has not
/// been tampered with or swapped between ops.
pub fn decrypt_fields(
    key: &SyncKey,
    envelope: &EncryptedEnvelope,
    entity_type: &EntityType,
    entity_id: &uuid::Uuid,
    op_type: &OpType,
) -> Result<serde_json::Value, TokuError> {
    // Validate envelope
    if envelope.ev != ENCRYPTION_ENVELOPE_VERSION {
        return Err(TokuError::Crypto(format!(
            "unsupported envelope version: {}",
            envelope.ev
        )));
    }
    if envelope.alg != ALGORITHM {
        return Err(TokuError::Crypto(format!(
            "unsupported algorithm: {}",
            envelope.alg
        )));
    }

    // Recompute AAD and verify it matches
    let expected_aad = build_aad(envelope.ev, entity_type, entity_id, op_type);
    if envelope.aad != expected_aad {
        return Err(TokuError::Crypto(
            "AAD mismatch: op metadata may have been tampered with".to_string(),
        ));
    }

    let nonce_bytes = BASE64_STANDARD
        .decode(&envelope.nonce)
        .map_err(|e| TokuError::Crypto(format!("nonce decode: {e}")))?;
    if nonce_bytes.len() != 12 {
        return Err(TokuError::Crypto(format!(
            "invalid nonce length: {} (expected 12)",
            nonce_bytes.len()
        )));
    }
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = BASE64_STANDARD
        .decode(&envelope.ciphertext)
        .map_err(|e| TokuError::Crypto(format!("ciphertext decode: {e}")))?;

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())
        .map_err(|e| TokuError::Crypto(format!("cipher init: {e}")))?;

    let payload = Payload {
        msg: &ciphertext,
        aad: envelope.aad.as_bytes(),
    };

    let plaintext = cipher.decrypt(nonce, payload).map_err(|_| {
        TokuError::Crypto("decryption failed (wrong key or tampered data)".to_string())
    })?;

    serde_json::from_slice(&plaintext)
        .map_err(|e| TokuError::Crypto(format!("deserialize decrypted fields: {e}")))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_entity_type() -> EntityType {
        EntityType::Book
    }

    fn test_entity_id() -> uuid::Uuid {
        uuid::Uuid::parse_str("01972123-abcd-7000-8000-000000000001").unwrap()
    }

    fn test_op_type() -> OpType {
        OpType::Update
    }

    fn test_fields() -> serde_json::Value {
        serde_json::json!({"title": "Dune", "rating": 9})
    }

    fn test_key_and_salt() -> (SyncKey, [u8; 16]) {
        let salt: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let key = SyncKey::derive("test-passphrase", &salt).unwrap();
        (key, salt)
    }

    // --- Key derivation ---

    #[test]
    fn key_derivation_is_stable() {
        let salt: [u8; 16] = [42; 16];
        let key1 = SyncKey::derive("my passphrase", &salt).unwrap();
        let key2 = SyncKey::derive("my passphrase", &salt).unwrap();
        assert_eq!(
            key1.0, key2.0,
            "same passphrase + salt must produce same key"
        );
    }

    #[test]
    fn different_passphrase_different_key() {
        let salt: [u8; 16] = [42; 16];
        let key1 = SyncKey::derive("passphrase-a", &salt).unwrap();
        let key2 = SyncKey::derive("passphrase-b", &salt).unwrap();
        assert_ne!(key1.0, key2.0);
    }

    #[test]
    fn different_salt_different_key() {
        let salt1: [u8; 16] = [1; 16];
        let salt2: [u8; 16] = [2; 16];
        let key1 = SyncKey::derive("same-passphrase", &salt1).unwrap();
        let key2 = SyncKey::derive("same-passphrase", &salt2).unwrap();
        assert_ne!(key1.0, key2.0);
    }

    #[test]
    fn generate_salt_produces_random_bytes() {
        let salt1 = SyncKey::generate_salt();
        let salt2 = SyncKey::generate_salt();
        assert_ne!(salt1, salt2, "two salts should not be equal");
    }

    #[test]
    fn key_debug_does_not_leak() {
        let (key, _) = test_key_and_salt();
        let debug = format!("{:?}", key);
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("0x"));
    }

    // --- Round-trip encryption/decryption ---

    #[test]
    fn encrypt_decrypt_round_trip() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        assert_eq!(envelope.ev, 1);
        assert_eq!(envelope.alg, "aes-256-gcm");
        assert!(!envelope.nonce.is_empty());
        assert!(!envelope.ciphertext.is_empty());

        let decrypted = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        assert_eq!(decrypted, fields);
    }

    #[test]
    fn round_trip_with_complex_fields() {
        let (key, _) = test_key_and_salt();
        let fields = serde_json::json!({
            "title": "Les Misérables",
            "description": "A novel with unicode: 日本語 🎉",
            "page_count": 1488,
            "rating": null,
            "tags": ["classic", "french"],
        });

        let envelope = encrypt_fields(
            &key,
            &fields,
            &EntityType::Book,
            &test_entity_id(),
            &OpType::Create,
        )
        .unwrap();

        let decrypted = decrypt_fields(
            &key,
            &envelope,
            &EntityType::Book,
            &test_entity_id(),
            &OpType::Create,
        )
        .unwrap();

        assert_eq!(decrypted, fields);
    }

    // --- Wrong key fails ---

    #[test]
    fn wrong_key_fails_decryption() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        let wrong_salt: [u8; 16] = [99; 16];
        let wrong_key = SyncKey::derive("wrong-passphrase", &wrong_salt).unwrap();

        let result = decrypt_fields(
            &wrong_key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("decryption failed"),
            "expected decryption error, got: {err}"
        );
    }

    // --- AAD prevents payload swapping ---

    #[test]
    fn aad_mismatch_entity_type_fails() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let envelope = encrypt_fields(
            &key,
            &fields,
            &EntityType::Book,
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        // Try to decrypt as if it were a Session op
        let result = decrypt_fields(
            &key,
            &envelope,
            &EntityType::Session,
            &test_entity_id(),
            &test_op_type(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn aad_mismatch_entity_id_fails() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        let different_id = uuid::Uuid::parse_str("01972123-abcd-7000-8000-999999999999").unwrap();
        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &different_id,
            &test_op_type(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn aad_mismatch_op_type_fails() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &OpType::Update,
        )
        .unwrap();

        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &OpType::Create,
        );
        assert!(result.is_err());
    }

    // --- Nonce uniqueness ---

    #[test]
    fn nonces_are_unique_across_10000_ops() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();
        let mut nonces = HashSet::new();

        for _ in 0..10_000 {
            let envelope = encrypt_fields(
                &key,
                &fields,
                &test_entity_type(),
                &test_entity_id(),
                &test_op_type(),
            )
            .unwrap();
            assert!(
                nonces.insert(envelope.nonce.clone()),
                "nonce collision detected"
            );
        }

        assert_eq!(nonces.len(), 10_000);
    }

    // --- Ciphertext tampering ---

    #[test]
    fn tampered_ciphertext_fails() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let mut envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        // Flip one byte in the ciphertext
        let mut ct_bytes = BASE64_STANDARD.decode(&envelope.ciphertext).unwrap();
        ct_bytes[0] ^= 0xFF;
        envelope.ciphertext = BASE64_STANDARD.encode(&ct_bytes);

        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn tampered_nonce_fails() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let mut envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        // Flip one byte in the nonce
        let mut nonce_bytes = BASE64_STANDARD.decode(&envelope.nonce).unwrap();
        nonce_bytes[0] ^= 0xFF;
        envelope.nonce = BASE64_STANDARD.encode(&nonce_bytes);

        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        );
        assert!(result.is_err());
    }

    // --- Envelope validation ---

    #[test]
    fn unsupported_envelope_version_rejected() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let mut envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        envelope.ev = 99;
        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported envelope version")
        );
    }

    #[test]
    fn unsupported_algorithm_rejected() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let mut envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        envelope.alg = "chacha20".to_string();
        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported algorithm")
        );
    }

    #[test]
    fn invalid_nonce_length_rejected() {
        let (key, _) = test_key_and_salt();
        let fields = test_fields();

        let mut envelope = encrypt_fields(
            &key,
            &fields,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        )
        .unwrap();

        // Set nonce to wrong length (8 bytes instead of 12)
        envelope.nonce = BASE64_STANDARD.encode([0u8; 8]);
        let result = decrypt_fields(
            &key,
            &envelope,
            &test_entity_type(),
            &test_entity_id(),
            &test_op_type(),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid nonce length")
        );
    }

    // --- AAD format ---

    #[test]
    fn aad_format_is_canonical() {
        let aad = build_aad(1, &EntityType::Book, &test_entity_id(), &OpType::Update);
        assert_eq!(
            aad,
            "v=1,entity_type=book,entity_id=01972123-abcd-7000-8000-000000000001,op_type=update"
        );
    }

    // --- All entity types encrypt/decrypt ---

    #[test]
    fn all_entity_types_round_trip() {
        let (key, _) = test_key_and_salt();
        let fields = serde_json::json!({"value": "test"});

        let entity_types = [
            EntityType::Book,
            EntityType::Session,
            EntityType::Progress,
            EntityType::Tag,
            EntityType::Note,
            EntityType::Review,
            EntityType::Setting,
        ];

        for et in &entity_types {
            let envelope =
                encrypt_fields(&key, &fields, et, &test_entity_id(), &OpType::Create).unwrap();

            let decrypted =
                decrypt_fields(&key, &envelope, et, &test_entity_id(), &OpType::Create).unwrap();

            assert_eq!(decrypted, fields, "round trip failed for {et}");
        }
    }
}
