use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::Engine;
use rand::RngCore;
use rand::rngs::OsRng;

/// 256-bit random token, URL-safe. `prefix` makes leaked tokens recognizable.
pub fn new_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Tokens are high-entropy, so a plain SHA-256 is an adequate at-rest hash.
pub fn hash_token(token: &str) -> String {
    crate::digest::sha256_hex(token.as_bytes())
}

pub fn hash_passphrase(passphrase: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(passphrase.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

pub fn verify_passphrase(passphrase: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|h| Argon2::default().verify_password(passphrase.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

/// Constant-time comparison for secrets of equal length.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrase_round_trip() {
        let h = hash_passphrase("correct horse").unwrap();
        assert!(verify_passphrase("correct horse", &h));
        assert!(!verify_passphrase("wrong", &h));
    }

    #[test]
    fn tokens_are_unique_and_prefixed() {
        let a = new_token("ocs");
        assert!(a.starts_with("ocs_"));
        assert_ne!(a, new_token("ocs"));
        assert!(ct_eq(&a, &a.clone()));
    }
}
