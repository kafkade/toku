//! Domain-separated SRP verifier-input derivation (ADR-010 two-secret auth).
//!
//! ADR-010 requires the SRP-6a verifier to depend on **both** account secrets:
//!
//! ```text
//! verifier_input = hash(domain_sep || Secret Key || account_password)
//! SRP verifier   = srp::generate_verifier(verifier_input)
//! ```
//!
//! Historically the verifier was computed from the raw password only, so the
//! Secret Key hardened only the key-wrapping layer, not authentication: a stolen
//! verifier (server DB breach) was offline-brute-forceable against the password
//! alone (threat-model finding **F1**). This helper folds the Secret Key back
//! into the SRP "password" input so the verifier's strength rests on
//! `password + Secret Key`, restoring the 128 bits ADR-010 promises.
//!
//! The single-secret library/passphrase sync path passes `secret_key = None`; it
//! still routes through the same domain separator for a uniform derivation, but
//! gains no extra entropy (there is no second secret to fold in).
//!
//! The returned 32 bytes are used as the `password` argument to the SRP client's
//! `compute_verifier` / `process_reply`, replacing the raw password bytes at
//! every verifier create/verify site.

use sha2::{Digest, Sha256};

/// Domain-separation tag for the SRP verifier-input derivation.
const SRP_VERIFIER_INPUT_DOMAIN: &[u8] = b"toku/srp/verifier-input/v1";

/// Derive the SRP verifier input from an optional Secret Key and the password.
///
/// Computes `SHA-256(domain_sep || len(secret_key) || secret_key || password)`.
/// The Secret Key is length-prefixed (4-byte big-endian) so the key/password
/// boundary is unambiguous and no `(key, password)` pair can collide with a
/// different split of the same concatenation. When `secret_key` is `None` a
/// zero length is hashed, keeping the single-secret path distinct yet uniform.
///
/// The output is suitable as the `password` bytes passed to
/// `srp::Client::compute_verifier` and `srp::Client::process_reply`.
pub fn srp_verifier_input(secret_key: Option<&[u8]>, password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SRP_VERIFIER_INPUT_DOMAIN);
    match secret_key {
        Some(key) => {
            hasher.update((key.len() as u32).to_be_bytes());
            hasher.update(key);
        }
        None => {
            hasher.update(0u32.to_be_bytes());
        }
    }
    hasher.update(password.as_bytes());

    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic() {
        let key = [0x11u8; 16];
        let a = srp_verifier_input(Some(&key), "correct horse");
        let b = srp_verifier_input(Some(&key), "correct horse");
        assert_eq!(a, b);
    }

    #[test]
    fn secret_key_changes_output() {
        let k1 = [0x11u8; 16];
        let k2 = [0x22u8; 16];
        assert_ne!(
            srp_verifier_input(Some(&k1), "pw"),
            srp_verifier_input(Some(&k2), "pw"),
        );
    }

    #[test]
    fn password_changes_output() {
        let key = [0x11u8; 16];
        assert_ne!(
            srp_verifier_input(Some(&key), "pw-a"),
            srp_verifier_input(Some(&key), "pw-b"),
        );
    }

    #[test]
    fn none_equals_empty_secret_key() {
        // An absent secret key and a zero-length one both hash a zero length
        // prefix, so they coincide. Harmless: real Secret Keys are always
        // 16 bytes, so the single-secret (None) path never collides in practice.
        assert_eq!(
            srp_verifier_input(None, "pw"),
            srp_verifier_input(Some(b""), "pw"),
        );
    }

    #[test]
    fn length_prefix_prevents_boundary_ambiguity() {
        // Without length framing, ("ab", "c") and ("a", "bc") would collide.
        assert_ne!(
            srp_verifier_input(Some(b"ab"), "c"),
            srp_verifier_input(Some(b"a"), "bc"),
        );
    }

    #[test]
    fn stable_test_vector() {
        // Locks the wire/verifier derivation. A change here means every enrolled
        // account's verifier changes — update deliberately, never casually.
        let key = [0xABu8; 16];
        let out = srp_verifier_input(Some(&key), "correct horse battery staple");
        assert_eq!(
            out,
            [
                0x24, 0x76, 0xf0, 0x37, 0x30, 0x02, 0x42, 0x52, 0x87, 0xac, 0x51, 0xf4, 0x1f, 0xec,
                0xd1, 0x71, 0x0f, 0xba, 0x7b, 0x13, 0xeb, 0x75, 0x41, 0x45, 0xb3, 0x24, 0x8e, 0xd9,
                0xe9, 0x3d, 0x58, 0x18,
            ]
        );
    }

    #[test]
    fn stable_test_vector_none() {
        let out = srp_verifier_input(None, "passphrase-only");
        assert_eq!(
            out,
            [
                0x26, 0x80, 0x5c, 0x6d, 0x28, 0x76, 0x27, 0x9b, 0xed, 0x65, 0x68, 0xda, 0x92, 0x8a,
                0xbd, 0x9e, 0xb0, 0x71, 0x1d, 0xad, 0xb2, 0xaf, 0x4d, 0x6d, 0x20, 0x90, 0x85, 0xbb,
                0x0c, 0x3b, 0x0e, 0xc0,
            ]
        );
    }
}
