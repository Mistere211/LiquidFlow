//! Sync über einen Ordner deiner Wahl (OneDrive/Dropbox/Syncthing/USB). Alles Ende-zu-Ende verschlüsselt
//! mit dem Master-Passwort; es gibt keinen Server. Dateien: data.lfs (Einstellungen) und vault.lfv (Tresor).
use crate::data::Bookmark;
use crate::settings::{BlockList, Dial, SiteRule};
use crate::vault;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Bundle {
    pub ts: u64,
    pub bookmarks: Vec<Bookmark>,
    pub quick_dials: Vec<Dial>,
    pub site_rules: HashMap<String, SiteRule>,
    pub keymap: HashMap<String, String>,
    pub accent: String,
    pub search_engine: String,
    pub lists: Vec<BlockList>,
}


pub fn read(folder: &str, pass: &str) -> Option<Bundle> {
    let b = std::fs::read(Path::new(folder).join("data.lfs")).ok()?;
    serde_json::from_slice(&vault::open(pass, &b)?).ok()
}
pub fn write(folder: &str, pass: &str, b: &Bundle) -> std::io::Result<()> {
    std::fs::write(Path::new(folder).join("data.lfs"), vault::seal(pass, &serde_json::to_vec(b).unwrap()))
}
