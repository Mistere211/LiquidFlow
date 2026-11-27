//! Lesezeichen, Verlauf, Downloads – lokal in %APPDATA%\LiquidFlow\
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)] pub struct Bookmark { pub title: String, pub url: String }
#[derive(Serialize, Deserialize, Clone)] pub struct HistEntry { pub title: String, pub url: String, pub t: u64 }
#[derive(Serialize, Clone)] pub struct Download { pub name: String, pub path: String, pub done: bool, pub ok: bool, pub vt: String }

static PROFILE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
pub fn set_profile(p: &str) { let _ = PROFILE.set(p.to_string()); }
pub fn profile() -> String { PROFILE.get().cloned().filter(|p| !p.is_empty()).unwrap_or_else(|| "default".into()) }
/// Datenordner des aktuellen Profils (%APPDATA%\LiquidFlow oder ...\profiles\<name>)
pub fn home() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_default().join("LiquidFlow");
    match profile().as_str() { "default" => base, p => base.join("profiles").join(p) }
}
pub fn profiles() -> Vec<String> {
    let mut v = vec!["default".to_string()];
    if let Ok(rd) = std::fs::read_dir(dirs::config_dir().unwrap_or_default().join("LiquidFlow/profiles")) {
        for e in rd.flatten() { if e.path().is_dir() { v.push(e.file_name().to_string_lossy().into_owned()); } }
    }
    v
}
fn dir() -> PathBuf { home() }

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct SessTab { pub url: String, pub title: String, pub group: u8 }
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Session { pub tabs: Vec<SessTab>, pub active: usize }
pub fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    std::fs::read_to_string(dir().join(name)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
pub fn save<T: Serialize>(name: &str, v: &T) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(dir().join(name), serde_json::to_string(v).unwrap_or_default());
}
/// "datei.zip" -> "datei (1).zip", falls schon vorhanden
pub fn unique(p: PathBuf) -> PathBuf {
    if !p.exists() { return p; }
    let (stem, ext) = (p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default());
    (1..).map(|i| p.with_file_name(format!("{stem} ({i}){ext}"))).find(|c| !c.exists()).unwrap()
}
