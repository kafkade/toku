//! Account Secret Key: generation, formatting, and checksum-validated parsing.
//!
//! The Secret Key is the high-entropy second factor in Toku's 1Password-style
//! two-secret auth model (see ADR-010). It is generated **on the device** at
//! account creation, surfaced exactly once via the Emergency Kit, and combined
//! with the account password to derive the master unlock key
//! ([`super::AccountKeys`]). The server never sees it and there is **no**
//! server-side escrow.
//!
//! # Format
//!
//! ```text
//! TK-XXXXXX-XXXXX-XXXXX-XXXXX-XXXXX-CC
//! └┬┘ └────────── 26 base32 chars ──────────┘ └┬┘
//!  version           128-bit entropy          checksum
//! ```
//!
//! - `TK` is a human-readable version prefix.
//! - 16 random bytes (128-bit entropy) are encoded as 26 RFC 4648 base32
//!   characters (`A–Z`, `2–7`) and grouped `6-5-5-5-5` for readability.
//! - The final `CC` group is a 10-bit checksum (2 base32 chars) derived from the
//!   entropy. Parsing recomputes it so single-character typos and adjacent
//!   transpositions are caught before the key is used.
//!
//! The decoded 16 entropy bytes are what feed [`super::AccountKeys::create`] and
//! [`super::AccountKeys::unlock_data_key`].

use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::TokuError;

/// Number of entropy bytes (128-bit).
const SECRET_KEY_BYTES: usize = 16;
/// Human-readable version prefix.
const VERSION_PREFIX: &str = "TK";
/// RFC 4648 base32 alphabet (no padding), uppercase and unambiguous.
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
/// Base32 characters needed to encode 16 bytes (`ceil(128 / 5)`).
const ENTROPY_CHARS: usize = 26;
/// Base32 characters in the checksum group (10 bits).
const CHECKSUM_CHARS: usize = 2;
/// Domain separation tag for the checksum hash.
const CHECKSUM_DOMAIN: &[u8] = b"toku/secret-key/checksum/v1";

/// A 128-bit account Secret Key.
///
/// The raw entropy is zeroized on drop. Use [`SecretKey::format`] (or the
/// [`std::fmt::Display`] impl) to render the transcribable string, and
/// [`SecretKey::parse`] to validate user-entered input.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct SecretKey([u8; SECRET_KEY_BYTES]);

impl SecretKey {
    /// Generate a fresh Secret Key from the operating system CSPRNG.
    pub fn generate() -> Result<Self, TokuError> {
        let mut bytes = [0u8; SECRET_KEY_BYTES];
        getrandom::fill(&mut bytes)
            .map_err(|e| TokuError::Crypto(format!("os rng failed for secret key: {e}")))?;
        Ok(Self(bytes))
    }

    /// Reconstruct a Secret Key from raw 16-byte entropy.
    pub fn from_bytes(bytes: [u8; SECRET_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// The raw 16 entropy bytes, suitable for [`super::AccountKeys::create`].
    pub fn as_bytes(&self) -> &[u8; SECRET_KEY_BYTES] {
        &self.0
    }

    /// Render the formatted, transcribable Secret Key string
    /// (e.g. `TK-ABCDEF-GHIJK-LMNOP-QRSTU-VWXYZ-23`).
    pub fn format(&self) -> String {
        let entropy = encode_base32(&self.0);
        let checksum = checksum_chars(&self.0);

        // Grouping: 6-5-5-5-5 over the 26 entropy chars, then the checksum.
        let groups = [
            &entropy[0..6],
            &entropy[6..11],
            &entropy[11..16],
            &entropy[16..21],
            &entropy[21..26],
        ];
        format!("{VERSION_PREFIX}-{}-{checksum}", groups.join("-"))
    }

    /// Parse and validate a user-entered Secret Key string.
    ///
    /// Input is normalized (uppercased, whitespace and hyphens removed), then
    /// the version prefix, length, alphabet, canonical base32 encoding, and
    /// checksum are all validated. A checksum mismatch (typo) is rejected.
    pub fn parse(input: &str) -> Result<Self, TokuError> {
        let normalized: String = input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| c.to_ascii_uppercase())
            .collect();

        let body = normalized.strip_prefix(VERSION_PREFIX).ok_or_else(|| {
            TokuError::Crypto(format!("secret key must start with '{VERSION_PREFIX}-'"))
        })?;

        let expected_len = ENTROPY_CHARS + CHECKSUM_CHARS;
        if body.len() != expected_len {
            return Err(TokuError::Crypto(format!(
                "secret key has {} characters, expected {expected_len}",
                body.len()
            )));
        }

        if let Some(bad) = body.bytes().find(|b| !BASE32_ALPHABET.contains(b)) {
            return Err(TokuError::Crypto(format!(
                "secret key contains invalid character '{}'",
                bad as char
            )));
        }

        let (entropy_chars, checksum_chars) = body.split_at(ENTROPY_CHARS);
        let bytes = decode_base32(entropy_chars)?;

        if checksum_chars != checksum_chars_str(&bytes) {
            return Err(TokuError::Crypto(
                "secret key checksum mismatch (check for typos)".to_string(),
            ));
        }

        Ok(Self(bytes))
    }
}

impl std::fmt::Display for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.format())
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey([REDACTED])")
    }
}

/// Encode bytes as RFC 4648 base32 without padding (uppercase).
fn encode_base32(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in input {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(BASE32_ALPHABET[idx] as char);
    }
    out
}

/// Decode canonical RFC 4648 base32 (no padding) into 16 bytes.
///
/// Rejects non-canonical encodings (non-zero trailing bits), which catches a
/// further class of transcription errors.
fn decode_base32(input: &str) -> Result<[u8; SECRET_KEY_BYTES], TokuError> {
    let mut out = Vec::with_capacity(SECRET_KEY_BYTES);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for ch in input.bytes() {
        let value = BASE32_ALPHABET
            .iter()
            .position(|&a| a == ch)
            .ok_or_else(|| TokuError::Crypto("secret key contains invalid character".to_string()))?
            as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    if bits > 0 && (buffer & ((1 << bits) - 1)) != 0 {
        return Err(TokuError::Crypto(
            "secret key is not canonically encoded".to_string(),
        ));
    }
    out.try_into()
        .map_err(|_| TokuError::Crypto("secret key has wrong decoded length".to_string()))
}

/// Compute the 10-bit checksum for the entropy and return it as two base32 chars.
fn checksum_chars(entropy: &[u8; SECRET_KEY_BYTES]) -> String {
    checksum_chars_str(entropy)
}

fn checksum_chars_str(entropy: &[u8; SECRET_KEY_BYTES]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CHECKSUM_DOMAIN);
    hasher.update(entropy);
    let digest = hasher.finalize();
    // Top 10 bits of the digest -> two 5-bit base32 symbols.
    let value = (u16::from(digest[0]) << 8 | u16::from(digest[1])) >> 6;
    let c1 = BASE32_ALPHABET[((value >> 5) & 0x1f) as usize] as char;
    let c2 = BASE32_ALPHABET[(value & 0x1f) as usize] as char;
    format!("{c1}{c2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::AccountKeys;

    #[test]
    fn generate_produces_valid_parseable_key() {
        let key = SecretKey::generate().unwrap();
        let formatted = key.format();
        let parsed = SecretKey::parse(&formatted).unwrap();
        assert_eq!(key.as_bytes(), parsed.as_bytes());
    }

    #[test]
    fn formatted_key_has_expected_shape() {
        let key = SecretKey::from_bytes([0u8; 16]);
        let formatted = key.format();
        assert!(formatted.starts_with("TK-"));
        let groups: Vec<&str> = formatted.split('-').collect();
        // TK + 5 entropy groups + 1 checksum group
        assert_eq!(groups.len(), 7);
        assert_eq!(groups[0], "TK");
        assert_eq!(groups[1].len(), 6);
        for g in &groups[2..6] {
            assert_eq!(g.len(), 5);
        }
        assert_eq!(groups[6].len(), 2);
    }

    #[test]
    fn parse_is_case_and_whitespace_insensitive() {
        let key = SecretKey::generate().unwrap();
        let formatted = key.format();
        let messy = format!("  {}  ", formatted.to_lowercase().replace('-', " "));
        let parsed = SecretKey::parse(&messy).unwrap();
        assert_eq!(key.as_bytes(), parsed.as_bytes());
    }

    #[test]
    fn as_bytes_is_16_bytes() {
        let key = SecretKey::generate().unwrap();
        assert_eq!(key.as_bytes().len(), 16);
    }

    #[test]
    fn checksum_rejects_single_char_typo() {
        let key = SecretKey::from_bytes([7u8; 16]);
        let formatted = key.format();
        // Flip one character in the entropy section.
        let mut chars: Vec<char> = formatted.chars().collect();
        let pos = 4; // inside the first entropy group
        chars[pos] = if chars[pos] == 'A' { 'B' } else { 'A' };
        let typo: String = chars.into_iter().collect();
        assert!(SecretKey::parse(&typo).is_err());
    }

    #[test]
    fn checksum_rejects_adjacent_transposition() {
        // Find a key whose first two entropy chars differ, then swap them.
        let mut formatted = String::new();
        for seed in 0u8..255 {
            let f = SecretKey::from_bytes([seed; 16]).format();
            let body: Vec<char> = f.chars().collect();
            // index 3,4 are the first two entropy chars (after "TK-")
            if body[3] != body[4] {
                formatted = f;
                break;
            }
        }
        assert!(!formatted.is_empty());
        let mut chars: Vec<char> = formatted.chars().collect();
        chars.swap(3, 4);
        let swapped: String = chars.into_iter().collect();
        assert!(SecretKey::parse(&swapped).is_err());
    }

    #[test]
    fn parse_rejects_missing_prefix() {
        let key = SecretKey::generate().unwrap();
        let without_prefix = key.format().replacen("TK-", "", 1);
        assert!(SecretKey::parse(&without_prefix).is_err());
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(SecretKey::parse("TK-AAAAA").is_err());
        assert!(SecretKey::parse("TK-AAAAAA-BBBBB-CCCCC-DDDDD-EEEEE-FF-EXTRA").is_err());
    }

    #[test]
    fn parse_rejects_invalid_character() {
        // '0', '1', '8', '9' are not in the base32 alphabet.
        let key = SecretKey::generate().unwrap();
        let bad = key.format().replacen(|c: char| c.is_alphabetic(), "0", 1);
        assert!(SecretKey::parse(&bad).is_err());
    }

    #[test]
    fn secret_key_unlocks_account_keys_end_to_end() {
        let secret = SecretKey::generate().unwrap();
        let (account_keys, data_key) =
            AccountKeys::create("correct horse battery staple", secret.as_bytes()).unwrap();

        // Re-parse the formatted key (simulating a new device) and unlock.
        let reparsed = SecretKey::parse(&secret.format()).unwrap();
        let unlocked = account_keys
            .unlock_data_key("correct horse battery staple", reparsed.as_bytes())
            .unwrap();
        assert_eq!(
            data_key.as_exported_bytes(),
            unlocked.as_exported_bytes(),
            "re-parsed secret key must unlock the same data key"
        );
    }

    #[test]
    fn debug_is_redacted() {
        let key = SecretKey::generate().unwrap();
        assert_eq!(format!("{key:?}"), "SecretKey([REDACTED])");
    }
}
