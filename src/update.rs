//! Auto-Update über GitHub Releases. Prüft SHA-256 aus der Release-Datei, bevor der Installer startet.
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Clone)]
pub struct Release { pub tag: String, pub exe: String, pub sha: String }

fn num(v: &str) -> Vec<u32> { v.trim_start_matches('v').split('.').map(|x| x.parse().unwrap_or(0)).collect() }

pub fn check(repo: &str) -> Option<Release> {
    let r: serde_json::Value = ureq::get(&format!("https://api.github.com/repos/{repo}/releases/latest"))
        .set("User-Agent", "LiquidFlow").call().ok()?.into_json().ok()?;
    let tag = r["tag_name"].as_str()?.to_string();
    if num(&tag) <= num(env!("CARGO_PKG_VERSION")) { return None; }
    let find = |n: &str| r["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(n)).and_then(|a| a["browser_download_url"].as_str()).map(String::from);
    Some(Release { tag, exe: find("LiquidFlow-Offline-Setup.exe")?, sha: find("LiquidFlow-Offline-Setup.exe.sha256")? })
}

pub fn download(r: &Release) -> Result<std::path::PathBuf, String> {
    let want = ureq::get(&r.sha).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
    let want = want.split_whitespace().next().unwrap_or("").to_lowercase();
    let mut buf = Vec::new();
    ureq::get(&r.exe).call().map_err(|e| e.to_string())?.into_reader().take(300 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(&buf)) != want { return Err("Prüfsumme stimmt nicht – Update abgebrochen".into()); }
    let p = std::env::temp_dir().join("LiquidFlow-Setup.exe");
    std::fs::write(&p, &buf).map_err(|e| e.to_string())?;
    Ok(p)
}
