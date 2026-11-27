//! Passwort-Tresor: Argon2id (64 MiB) -> ChaCha20-Poly1305. Datei: %APPDATA%\LiquidFlow\vault.lfv
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{rand_core::RngCore, Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry { pub id: String, pub host: String, pub user: String, pub pass: String, #[serde(default)] pub totp: String }

pub fn path() -> PathBuf { crate::data::home().join("vault.lfv") }
pub fn exists() -> bool { path().exists() }
pub fn new_id() -> String { format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)) }

fn derive(pass: &str, salt: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut k = Zeroizing::new([0u8; 32]);
    let p = Params::new(65536, 3, 1, Some(32)).unwrap();
    Argon2::new(Algorithm::Argon2id, Version::V0x13, p).hash_password_into(pass.as_bytes(), salt, &mut *k).unwrap();
    k
}

/// Format: "LFV1" | salt(16) | nonce(12) | ciphertext
pub fn seal(pass: &str, data: &[u8]) -> Vec<u8> {
    let (mut salt, mut nonce) = ([0u8; 16], [0u8; 12]);
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let k = derive(pass, &salt);
    let ct = ChaCha20Poly1305::new(Key::from_slice(&*k)).encrypt(Nonce::from_slice(&nonce), data).unwrap();
    let mut out = b"LFV1".to_vec();
    out.extend(salt); out.extend(nonce); out.extend(ct);
    out
}

pub fn open(pass: &str, blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < 48 || &blob[..4] != b"LFV1" { return None; }
    let (salt, rest) = blob[4..].split_at(16);
    let (nonce, ct) = rest.split_at(12);
    let k = derive(pass, salt);
    ChaCha20Poly1305::new(Key::from_slice(&*k)).decrypt(Nonce::from_slice(nonce), ct).ok()
}

pub struct Vault { pub pass: Zeroizing<String>, pub entries: Vec<Entry> }

impl Vault {
    pub fn unlock(pass: &str) -> Option<Vault> {
        let d = open(pass, &std::fs::read(path()).ok()?)?;
        Some(Vault { pass: Zeroizing::new(pass.into()), entries: serde_json::from_slice(&d).ok()? })
    }
    pub fn create(pass: &str) -> Vault { let v = Vault { pass: Zeroizing::new(pass.into()), entries: vec![] }; v.save(); v }
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(path().parent().unwrap());
        let _ = std::fs::write(path(), seal(&self.pass, &serde_json::to_vec(&self.entries).unwrap()));
    }
}
