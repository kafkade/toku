//! Key hierarchy for hosted sync encryption.
//!
//! This module keeps the existing [`super::SyncKey`] as the leaf data-encryption
//! key while introducing an account-level hierarchy:
//!
//! ```text
//! Secret Key + password --Argon2id+SHA-256(2SKD)--> MasterUnlockKey
//! MasterUnlockKey --AES-256-GCM--> WrappedAccountPrivateKey
//! Account public/private keypair (X25519)
//! Account public key --ECIES-like wrap--> WrappedDataKey (contains SyncKey)
//! ```
//!
//! # Versioned serialized key material
//!
//! `AccountKeys` is the versioned on-disk/on-wire container for:
//! - KDF parameters (`AccountKdfParams`)
//! - Account public key
//! - Wrapped account private key
//! - Wrapped data key (`SyncKey`)
//!
//! Backward compatibility strategy:
//! - Existing sync payload encryption (`encrypt_fields` / `decrypt_fields`) is unchanged.
//! - Existing `SyncKey` remains the data-encryption primitive.
//! - Legacy single-passphrase setups can be migrated by unwrapping to a `SyncKey`,
//!   then re-wrapping through `AccountKeys`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::TokuError;

use super::SyncKey;

const ACCOUNT_KEYS_VERSION: u16 = 1;
const KDF_VERSION: u16 = 1;
const WRAP_VERSION: u16 = 1;
const MIN_SECRET_KEY_BYTES: usize = 16;

const MASTER_KDF_ALGORITHM: &str = "argon2id+sha256-2skd";
const PRIVATE_KEY_WRAP_ALGORITHM: &str = "aes-256-gcm";
const DATA_KEY_WRAP_ALGORITHM: &str = "x25519-sha256-kdf+aes-256-gcm";

const MASTER_UNLOCK_INFO: &[u8] = b"toku/master-unlock-key/v1";
const DATA_KEY_WRAP_INFO: &[u8] = b"toku/library-data-key-wrap/v1";
const PRIVATE_KEY_WRAP_AAD: &[u8] = b"v=1,alg=aes-256-gcm,material=account-private-key";

/// Argon2id parameters and salt for deriving [`MasterUnlockKey`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountKdfParams {
    /// KDF parameter schema version.
    pub version: u16,
    /// KDF algorithm identifier.
    pub algorithm: String,
    /// Base64-encoded 16-byte per-account salt.
    pub salt: String,
    /// Argon2 memory cost in KiB.
    pub memory_kib: u32,
    /// Argon2 iterations.
    pub iterations: u32,
    /// Argon2 parallelism.
    pub parallelism: u32,
}

impl AccountKdfParams {
    /// Create v1 KDF params with a fresh random per-account salt.
    pub fn generate() -> Result<Self, TokuError> {
        let salt = random_array::<16>("account salt")?;
        Ok(Self::from_salt(salt))
    }

    /// Create v1 KDF params from a caller-provided salt.
    pub fn from_salt(salt: [u8; 16]) -> Self {
        Self {
            version: KDF_VERSION,
            algorithm: MASTER_KDF_ALGORITHM.to_string(),
            salt: BASE64_STANDARD.encode(salt),
            memory_kib: super::ARGON2_M_COST,
            iterations: super::ARGON2_T_COST,
            parallelism: super::ARGON2_P_COST,
        }
    }

    fn decode_salt(&self) -> Result<[u8; 16], TokuError> {
        decode_b64_fixed(&self.salt, "account kdf salt")
    }

    fn validate(&self) -> Result<(), TokuError> {
        if self.version != KDF_VERSION {
            return Err(TokuError::Crypto(format!(
                "unsupported account kdf version: {}",
                self.version
            )));
        }
        if self.algorithm != MASTER_KDF_ALGORITHM {
            return Err(TokuError::Crypto(format!(
                "unsupported account kdf algorithm: {}",
                self.algorithm
            )));
        }
        Ok(())
    }
}

/// Master unlock key derived from account password + Secret Key.
///
/// Key material is zeroized on drop.
#[derive(Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct MasterUnlockKey([u8; 32]);

impl MasterUnlockKey {
    /// Derive a master unlock key from account password and Secret Key bytes.
    ///
    /// - Password path: Argon2id using [`AccountKdfParams`]
    /// - Two-secret combine step: domain-separated SHA-256 over both secrets
    pub fn derive(
        password: &str,
        secret_key: &[u8],
        params: &AccountKdfParams,
    ) -> Result<Self, TokuError> {
        if password.is_empty() {
            return Err(TokuError::Crypto(
                "account password cannot be empty".to_string(),
            ));
        }
        if secret_key.len() < MIN_SECRET_KEY_BYTES {
            return Err(TokuError::Crypto(format!(
                "secret key must be at least {MIN_SECRET_KEY_BYTES} bytes"
            )));
        }
        params.validate()?;
        let salt = params.decode_salt()?;

        let argon_params = argon2::Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            Some(32),
        )
        .map_err(|e| TokuError::Crypto(format!("argon2 params: {e}")))?;
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon_params,
        );

        let mut password_derived = [0u8; 32];
        argon2
            .hash_password_into(password.as_bytes(), &salt, &mut password_derived)
            .map_err(|e| TokuError::Crypto(format!("password derivation failed: {e}")))?;

        let mut hasher = Sha256::new();
        hasher.update(MASTER_UNLOCK_INFO);
        hasher.update(secret_key);
        hasher.update(password_derived);
        let digest = hasher.finalize();
        let mut unlock = [0u8; 32];
        unlock.copy_from_slice(&digest[..32]);
        password_derived.zeroize();

        Ok(Self(unlock))
    }

    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for MasterUnlockKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterUnlockKey([REDACTED])")
    }
}

/// Wrapped account private key encrypted under [`MasterUnlockKey`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedAccountPrivateKey {
    /// Wrapper schema version.
    pub version: u16,
    /// Algorithm identifier (`aes-256-gcm` for v1).
    pub algorithm: String,
    /// Base64-encoded 96-bit nonce.
    pub nonce: String,
    /// Base64-encoded ciphertext.
    pub ciphertext: String,
}

impl WrappedAccountPrivateKey {
    /// Wrap account private key bytes with a master unlock key.
    pub fn wrap(unlock_key: &MasterUnlockKey, private_key: &[u8; 32]) -> Result<Self, TokuError> {
        let (nonce, ciphertext) = encrypt_with_aes_gcm(
            unlock_key.as_bytes(),
            private_key,
            PRIVATE_KEY_WRAP_AAD,
            "account private key",
        )?;

        Ok(Self {
            version: WRAP_VERSION,
            algorithm: PRIVATE_KEY_WRAP_ALGORITHM.to_string(),
            nonce,
            ciphertext,
        })
    }

    /// Unwrap to raw account private key bytes.
    pub fn unwrap(&self, unlock_key: &MasterUnlockKey) -> Result<[u8; 32], TokuError> {
        self.validate()?;
        let plaintext = decrypt_with_aes_gcm(
            unlock_key.as_bytes(),
            &self.nonce,
            &self.ciphertext,
            PRIVATE_KEY_WRAP_AAD,
            "account private key",
        )?;
        let bytes: [u8; 32] = plaintext.as_slice().try_into().map_err(|_| {
            TokuError::Crypto("wrapped account private key must decode to 32 bytes".to_string())
        })?;
        Ok(bytes)
    }

    fn validate(&self) -> Result<(), TokuError> {
        if self.version != WRAP_VERSION {
            return Err(TokuError::Crypto(format!(
                "unsupported account private key wrap version: {}",
                self.version
            )));
        }
        if self.algorithm != PRIVATE_KEY_WRAP_ALGORITHM {
            return Err(TokuError::Crypto(format!(
                "unsupported account private key wrap algorithm: {}",
                self.algorithm
            )));
        }
        Ok(())
    }
}

/// Wrapped library data key (leaf [`SyncKey`]) encrypted to account public key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedDataKey {
    /// Wrapper schema version.
    pub version: u16,
    /// Algorithm identifier (`x25519-sha256-kdf+aes-256-gcm` for v1).
    pub algorithm: String,
    /// Base64-encoded ephemeral X25519 public key.
    pub ephemeral_public_key: String,
    /// Base64-encoded 16-byte KDF salt.
    pub kdf_salt: String,
    /// Base64-encoded 96-bit AES-GCM nonce.
    pub nonce: String,
    /// Base64-encoded wrapped `SyncKey` bytes.
    pub ciphertext: String,
}

impl WrappedDataKey {
    /// Wrap a data key using the account public key.
    pub fn wrap(data_key: &SyncKey, account_public_key: &[u8; 32]) -> Result<Self, TokuError> {
        let ephemeral_secret_bytes = random_array::<32>("ephemeral private key")?;
        let ephemeral_secret = StaticSecret::from(ephemeral_secret_bytes);
        let ephemeral_public = X25519PublicKey::from(&ephemeral_secret).to_bytes();
        let shared = ephemeral_secret
            .diffie_hellman(&X25519PublicKey::from(*account_public_key))
            .to_bytes();
        let kdf_salt = random_array::<16>("data-key wrap kdf salt")?;
        let mut wrap_key = derive_wrap_key(&shared, &kdf_salt)?;

        let epk_b64 = BASE64_STANDARD.encode(ephemeral_public);
        let kdf_salt_b64 = BASE64_STANDARD.encode(kdf_salt);
        let aad = build_data_key_aad(
            WRAP_VERSION,
            DATA_KEY_WRAP_ALGORITHM,
            &epk_b64,
            &kdf_salt_b64,
        );
        let (nonce, ciphertext) = encrypt_with_aes_gcm(
            &wrap_key,
            data_key.as_exported_bytes(),
            aad.as_bytes(),
            "library data key",
        )?;
        wrap_key.zeroize();

        Ok(Self {
            version: WRAP_VERSION,
            algorithm: DATA_KEY_WRAP_ALGORITHM.to_string(),
            ephemeral_public_key: epk_b64,
            kdf_salt: kdf_salt_b64,
            nonce,
            ciphertext,
        })
    }

    /// Unwrap a data key using the account private key.
    pub fn unwrap(&self, account_private_key: &[u8; 32]) -> Result<SyncKey, TokuError> {
        self.validate()?;
        let epk = decode_b64_fixed::<32>(&self.ephemeral_public_key, "ephemeral public key")?;
        let kdf_salt = decode_b64_fixed::<16>(&self.kdf_salt, "data-key kdf salt")?;
        let private = StaticSecret::from(*account_private_key);
        let shared = private
            .diffie_hellman(&X25519PublicKey::from(epk))
            .to_bytes();
        let mut wrap_key = derive_wrap_key(&shared, &kdf_salt)?;

        let aad = build_data_key_aad(
            self.version,
            &self.algorithm,
            &self.ephemeral_public_key,
            &self.kdf_salt,
        );
        let plaintext = decrypt_with_aes_gcm(
            &wrap_key,
            &self.nonce,
            &self.ciphertext,
            aad.as_bytes(),
            "library data key",
        )?;
        wrap_key.zeroize();

        SyncKey::from_exported_bytes(&plaintext)
    }

    fn validate(&self) -> Result<(), TokuError> {
        if self.version != WRAP_VERSION {
            return Err(TokuError::Crypto(format!(
                "unsupported data-key wrap version: {}",
                self.version
            )));
        }
        if self.algorithm != DATA_KEY_WRAP_ALGORITHM {
            return Err(TokuError::Crypto(format!(
                "unsupported data-key wrap algorithm: {}",
                self.algorithm
            )));
        }
        Ok(())
    }
}

/// Versioned account key material for persisted/encrypted sync state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountKeys {
    /// Schema version for this key-material bundle.
    pub version: u16,
    /// KDF parameters used for deriving the master unlock key.
    pub kdf: AccountKdfParams,
    /// Base64-encoded account X25519 public key.
    pub public_key: String,
    /// Wrapped account private key (under master unlock key).
    pub wrapped_private_key: WrappedAccountPrivateKey,
    /// Wrapped library data key (the leaf [`SyncKey`]).
    pub wrapped_data_key: WrappedDataKey,
}

impl AccountKeys {
    /// Create a fresh account key hierarchy and return the plaintext data key.
    pub fn create(password: &str, secret_key: &[u8]) -> Result<(Self, SyncKey), TokuError> {
        let kdf = AccountKdfParams::generate()?;
        let unlock_key = MasterUnlockKey::derive(password, secret_key, &kdf)?;

        let account_private_key = random_array::<32>("account private key")?;
        let account_private = StaticSecret::from(account_private_key);
        let account_public = X25519PublicKey::from(&account_private).to_bytes();

        let wrapped_private_key =
            WrappedAccountPrivateKey::wrap(&unlock_key, &account_private_key)?;

        let data_key = SyncKey(random_array::<32>("library data key")?);
        let wrapped_data_key = WrappedDataKey::wrap(&data_key, &account_public)?;

        Ok((
            Self {
                version: ACCOUNT_KEYS_VERSION,
                kdf,
                public_key: BASE64_STANDARD.encode(account_public),
                wrapped_private_key,
                wrapped_data_key,
            },
            data_key,
        ))
    }

    /// Derive unlock material and unwrap the leaf data key.
    pub fn unlock_data_key(&self, password: &str, secret_key: &[u8]) -> Result<SyncKey, TokuError> {
        self.validate()?;

        let unlock_key = MasterUnlockKey::derive(password, secret_key, &self.kdf)?;
        let private_key = self.wrapped_private_key.unwrap(&unlock_key)?;
        let private = StaticSecret::from(private_key);
        let expected_public = X25519PublicKey::from(&private).to_bytes();
        let stored_public = decode_b64_fixed::<32>(&self.public_key, "account public key")?;
        if expected_public != stored_public {
            return Err(TokuError::Crypto(
                "account private key does not match stored public key".to_string(),
            ));
        }

        self.wrapped_data_key.unwrap(&private_key)
    }

    /// Re-wrap the account private key under new password/Secret Key material.
    ///
    /// The wrapped data key remains unchanged, enabling password rotation without
    /// re-encrypting all payloads.
    pub fn rotate_unlock_credentials(
        &mut self,
        old_password: &str,
        old_secret_key: &[u8],
        new_password: &str,
        new_secret_key: &[u8],
    ) -> Result<(), TokuError> {
        self.validate()?;

        let old_unlock = MasterUnlockKey::derive(old_password, old_secret_key, &self.kdf)?;
        let private_key = self.wrapped_private_key.unwrap(&old_unlock)?;

        let new_kdf = AccountKdfParams::generate()?;
        let new_unlock = MasterUnlockKey::derive(new_password, new_secret_key, &new_kdf)?;
        let new_wrapped_private = WrappedAccountPrivateKey::wrap(&new_unlock, &private_key)?;

        self.kdf = new_kdf;
        self.wrapped_private_key = new_wrapped_private;
        Ok(())
    }

    fn validate(&self) -> Result<(), TokuError> {
        if self.version != ACCOUNT_KEYS_VERSION {
            return Err(TokuError::Crypto(format!(
                "unsupported account keys version: {}",
                self.version
            )));
        }
        Ok(())
    }
}

fn build_data_key_aad(version: u16, algorithm: &str, epk_b64: &str, kdf_salt_b64: &str) -> String {
    format!("v={version},alg={algorithm},epk={epk_b64},kdf_salt={kdf_salt_b64}")
}

fn derive_wrap_key(shared_secret: &[u8; 32], kdf_salt: &[u8; 16]) -> Result<[u8; 32], TokuError> {
    let mut hasher = Sha256::new();
    hasher.update(DATA_KEY_WRAP_INFO);
    hasher.update(kdf_salt);
    hasher.update(shared_secret);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest[..32]);
    Ok(out)
}

fn encrypt_with_aes_gcm(
    key: &[u8; 32],
    plaintext: &[u8],
    aad: &[u8],
    material_name: &str,
) -> Result<(String, String), TokuError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| TokuError::Crypto(format!("cipher init for {material_name}: {e}")))?;
    let nonce_bytes = random_array::<12>(&format!("{material_name} nonce"))?;
    let nonce = Nonce::from(nonce_bytes);
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    let ciphertext = cipher
        .encrypt(&nonce, payload)
        .map_err(|_| TokuError::Crypto(format!("{material_name} encryption failed")))?;

    Ok((
        BASE64_STANDARD.encode(nonce_bytes),
        BASE64_STANDARD.encode(ciphertext),
    ))
}

fn decrypt_with_aes_gcm(
    key: &[u8; 32],
    nonce_b64: &str,
    ciphertext_b64: &str,
    aad: &[u8],
    material_name: &str,
) -> Result<Vec<u8>, TokuError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| TokuError::Crypto(format!("cipher init for {material_name}: {e}")))?;
    let nonce_bytes = decode_b64_fixed::<12>(nonce_b64, &format!("{material_name} nonce"))?;
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = BASE64_STANDARD
        .decode(ciphertext_b64)
        .map_err(|e| TokuError::Crypto(format!("{material_name} ciphertext decode: {e}")))?;
    let payload = Payload {
        msg: &ciphertext,
        aad,
    };
    cipher
        .decrypt(&nonce, payload)
        .map_err(|_| TokuError::Crypto(format!("{material_name} decryption failed")))
}

fn decode_b64_fixed<const N: usize>(value: &str, label: &str) -> Result<[u8; N], TokuError> {
    let bytes = BASE64_STANDARD
        .decode(value)
        .map_err(|e| TokuError::Crypto(format!("{label} decode: {e}")))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| TokuError::Crypto(format!("{label} must be {N} bytes")))
}

fn random_array<const N: usize>(material_name: &str) -> Result<[u8; N], TokuError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)
        .map_err(|e| TokuError::Crypto(format!("os rng failed for {material_name}: {e}")))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret_key() -> [u8; 16] {
        [42u8; 16]
    }

    #[test]
    fn master_unlock_key_is_stable_for_same_inputs() {
        let params = AccountKdfParams::from_salt([7u8; 16]);
        let key_a = MasterUnlockKey::derive("pw", &secret_key(), &params).unwrap();
        let key_b = MasterUnlockKey::derive("pw", &secret_key(), &params).unwrap();
        assert_eq!(key_a, key_b);
    }

    #[test]
    fn account_keys_create_and_unlock_round_trip() {
        let (account_keys, data_key) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        let unlocked = account_keys.unlock_data_key("pw-1", &secret_key()).unwrap();
        assert_eq!(data_key.as_exported_bytes(), unlocked.as_exported_bytes());
    }

    #[test]
    fn wrong_password_cannot_unlock_data_key() {
        let (account_keys, _) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        assert!(
            account_keys
                .unlock_data_key("wrong", &secret_key())
                .is_err()
        );
    }

    #[test]
    fn wrong_secret_key_cannot_unlock_data_key() {
        let (account_keys, _) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        let wrong_secret = [8u8; 16];
        assert!(account_keys.unlock_data_key("pw-1", &wrong_secret).is_err());
    }

    #[test]
    fn serialized_account_keys_round_trip() {
        let (account_keys, expected_key) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        let encoded = serde_json::to_string(&account_keys).unwrap();
        let decoded: AccountKeys = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, account_keys);

        let unlocked = decoded.unlock_data_key("pw-1", &secret_key()).unwrap();
        assert_eq!(
            unlocked.as_exported_bytes(),
            expected_key.as_exported_bytes()
        );
    }

    #[test]
    fn wrapped_data_key_rejects_unknown_version() {
        let (mut account_keys, _) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        account_keys.wrapped_data_key.version = 99;
        let err = account_keys
            .unlock_data_key("pw-1", &secret_key())
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported data-key wrap version"));
    }

    #[test]
    fn wrapped_private_key_rejects_unknown_algorithm() {
        let (mut account_keys, _) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        account_keys.wrapped_private_key.algorithm = "chacha20".to_string();
        let err = account_keys
            .unlock_data_key("pw-1", &secret_key())
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported account private key wrap algorithm"));
    }

    #[test]
    fn password_rotation_rewraps_private_key_only() {
        let old_secret = [1u8; 16];
        let new_secret = [2u8; 16];

        let (mut account_keys, expected_data_key) =
            AccountKeys::create("old-pw", &old_secret).unwrap();
        let wrapped_data_before = account_keys.wrapped_data_key.clone();

        account_keys
            .rotate_unlock_credentials("old-pw", &old_secret, "new-pw", &new_secret)
            .unwrap();

        assert_eq!(account_keys.wrapped_data_key, wrapped_data_before);
        assert!(account_keys.unlock_data_key("old-pw", &old_secret).is_err());

        let unlocked = account_keys.unlock_data_key("new-pw", &new_secret).unwrap();
        assert_eq!(
            unlocked.as_exported_bytes(),
            expected_data_key.as_exported_bytes()
        );
    }

    #[test]
    fn public_key_mismatch_is_rejected() {
        let (mut account_keys, _) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        let (other_keys, _) = AccountKeys::create("pw-1", &secret_key()).unwrap();
        account_keys.public_key = other_keys.public_key;

        let err = account_keys
            .unlock_data_key("pw-1", &secret_key())
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match stored public key"));
    }
}
