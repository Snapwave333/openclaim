//! Authenticated encryption for the local evidence vault and sensitive fields.
//!
//! The 256-bit vault key is generated once. On Windows it is stored wrapped by
//! DPAPI (bound to the current user account); elsewhere it is stored in a file
//! readable only by the owner. Recovery limitation: losing the OS account (or
//! the key file) makes encrypted evidence unrecoverable; use `data backup`,
//! which exports decrypted copies under the user's control.

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use rand::rngs::OsRng;
use std::path::Path;

const NONCE_LEN: usize = 24;
const MAGIC: &[u8; 4] = b"OCV1";

pub struct VaultKey {
    cipher: XChaCha20Poly1305,
}

#[derive(Debug)]
pub struct VaultError(pub String);

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl VaultKey {
    pub fn from_bytes(key: [u8; 32]) -> Self {
        VaultKey { cipher: XChaCha20Poly1305::new((&key).into()) }
    }

    /// Loads the key from `dir`, creating and protecting a new one on first run.
    pub fn load_or_create(dir: &Path) -> Result<Self, VaultError> {
        std::fs::create_dir_all(dir).map_err(|e| VaultError(format!("create {}: {e}", dir.display())))?;
        let path = dir.join(KEY_FILE);
        if path.exists() {
            let wrapped = std::fs::read(&path).map_err(|e| VaultError(format!("read vault key: {e}")))?;
            let raw = unprotect(&wrapped)?;
            let key: [u8; 32] = raw.try_into().map_err(|_| VaultError("vault key has the wrong length".into()))?;
            return Ok(Self::from_bytes(key));
        }
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        let wrapped = protect(&key)?;
        write_private(&path, &wrapped)?;
        Ok(Self::from_bytes(key))
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let ct = self.cipher.encrypt(XNonce::from_slice(&nonce), plaintext).expect("encryption cannot fail");
        let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    pub fn decrypt(&self, blob: &[u8]) -> Result<Vec<u8>, VaultError> {
        if blob.len() < 4 + NONCE_LEN || &blob[..4] != MAGIC {
            return Err(VaultError("not an OpenClaim vault blob".into()));
        }
        let nonce = XNonce::from_slice(&blob[4..4 + NONCE_LEN]);
        self.cipher
            .decrypt(nonce, &blob[4 + NONCE_LEN..])
            .map_err(|_| VaultError("vault blob failed authentication (tampered or wrong key)".into()))
    }

    pub fn encrypt_str(&self, s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.encrypt(s.as_bytes()))
    }

    pub fn decrypt_str(&self, s: &str) -> Result<String, VaultError> {
        let blob = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|_| VaultError("encrypted field is not base64".into()))?;
        String::from_utf8(self.decrypt(&blob)?).map_err(|_| VaultError("decrypted field is not UTF-8".into()))
    }
}

#[cfg(windows)]
const KEY_FILE: &str = "vault.key.dpapi";
#[cfg(not(windows))]
const KEY_FILE: &str = "vault.key";

/// Writes a file that only the current user can read.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| VaultError(format!("write {}: {e}", path.display())))?;
        f.write_all(bytes).map_err(|e| VaultError(e.to_string()))
    }
    #[cfg(not(unix))]
    {
        // The data directory lives under the user's profile (%LOCALAPPDATA%), whose ACL already
        // excludes other users; the key itself is additionally DPAPI-wrapped.
        std::fs::write(path, bytes).map_err(|e| VaultError(format!("write {}: {e}", path.display())))
    }
}

#[cfg(windows)]
fn protect(data: &[u8]) -> Result<Vec<u8>, VaultError> {
    dpapi::protect(data)
}

#[cfg(windows)]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, VaultError> {
    dpapi::unprotect(data)
}

#[cfg(not(windows))]
fn protect(data: &[u8]) -> Result<Vec<u8>, VaultError> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> Result<Vec<u8>, VaultError> {
    Ok(data.to_vec())
}

#[cfg(windows)]
mod dpapi {
    use super::VaultError;
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    fn call(data: &[u8], protect: bool) -> Result<Vec<u8>, VaultError> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        // SAFETY: input points to `data` for the duration of the call; output is allocated by
        // the OS with LocalAlloc and freed below after copying.
        let ok = unsafe {
            if protect {
                CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            } else {
                CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            }
        };
        if ok == 0 {
            return Err(VaultError(format!(
                "DPAPI {} failed: {}",
                if protect { "protect" } else { "unprotect" },
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: on success output.pbData points to output.cbData bytes owned by us.
        let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
        unsafe { LocalFree(output.pbData as HLOCAL) };
        Ok(bytes)
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, VaultError> {
        call(data, true)
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, VaultError> {
        call(data, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper_detection() {
        let dir = tempfile::tempdir().unwrap();
        let k = VaultKey::load_or_create(dir.path()).unwrap();
        let blob = k.encrypt(b"receipt bytes");
        assert_eq!(k.decrypt(&blob).unwrap(), b"receipt bytes");
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(k.decrypt(&bad).is_err());
        let k2 = VaultKey::load_or_create(dir.path()).unwrap();
        assert_eq!(k2.decrypt(&blob).unwrap(), b"receipt bytes");
        assert_eq!(k2.decrypt_str(&k.encrypt_str("hi")).unwrap(), "hi");
    }
}
