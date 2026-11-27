//! Blocklisten: Hosts-Format, reine Domain-Listen und Adblock-Domainregeln (||domain^).
//! Gecacht als 1 Domain pro Zeile in %APPDATA%\LiquidFlow\blocklists\<id>.txt
use crate::settings::BlockList;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn dir() -> PathBuf { crate::data::home().join("blocklists") }

fn valid(d: &str) -> bool {
    d.contains('.') && d.len() < 254 && d != "localhost"
        && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
}

pub fn parse(text: &str, out: &mut HashSet<String>) {
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with('!') || l.starts_with('[') || l.starts_with("@@") { continue; }
        if l.contains("##") || l.contains("#@#") || l.contains("#?#") { continue; } // Element-Hiding
        let l = l.split('#').next().unwrap_or("").trim();
        let cand: Option<&str> = if let Some(r) = l.strip_prefix("||") {
            match r.find(|c| c == '^' || c == '$' || c == '/') {
                Some(i) if r.as_bytes()[i] == b'/' => None, // Pfadregel: nicht ganze Domain blocken
                Some(i) => Some(&r[..i]),
                None => Some(r),
            }
        } else {
            let mut it = l.split_whitespace();
            match it.next() { Some(x) if x.parse::<std::net::IpAddr>().is_ok() => it.next(), other => other }
        };
        if let Some(d) = cand {
            let d = d.trim_start_matches("*.").trim_end_matches('.').to_ascii_lowercase();
            if valid(&d) { out.insert(d); }
        }
    }
}

fn read_path(p: &Path) -> Result<String, String> {
    let mut all = String::new();
    if p.is_dir() {
        for e in std::fs::read_dir(p).map_err(|e| e.to_string())?.flatten() {
            if e.path().is_file() { all.push_str(&read_path(&e.path()).unwrap_or_default()); all.push('\n'); }
        }
    } else if p.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false) {
        let mut z = zip::ZipArchive::new(std::fs::File::open(p).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| e.to_string())?;
            if f.is_file() { let mut s = String::new(); if f.read_to_string(&mut s).is_ok() { all.push_str(&s); all.push('\n'); } }
        }
    } else {
        all = String::from_utf8_lossy(&std::fs::read(p).map_err(|e| e.to_string())?).into_owned();
    }
    Ok(all)
}

/// Holt/liest die Quelle, parst sie und schreibt den Cache. Läuft im Hintergrund-Thread.
pub fn fetch(l: &BlockList) -> Result<usize, String> {
    let text = if l.source == "url" {
        let resp = ureq::get(&l.location).timeout(std::time::Duration::from_secs(90)).call().map_err(|e| e.to_string())?;
        let mut s = String::new();
        resp.into_reader().take(256 << 20).read_to_string(&mut s).map_err(|e| e.to_string())?;
        s
    } else { read_path(Path::new(&l.location))? };
    let mut set = HashSet::new();
    parse(&text, &mut set);
    if set.is_empty() { return Err("keine Domains gefunden".into()); }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let mut out = String::with_capacity(set.len() * 20);
    for d in &set { out.push_str(d); out.push('\n'); }
    std::fs::write(dir().join(format!("{}.txt", l.id)), out).map_err(|e| e.to_string())?;
    Ok(set.len())
}

pub fn load_enabled(ids: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    for id in ids {
        if let Ok(t) = std::fs::read_to_string(dir().join(format!("{id}.txt"))) {
            for l in t.lines() { if !l.is_empty() { set.insert(l.to_string()); } }
        }
    }
    set
}

pub fn remove_cache(id: &str) { let _ = std::fs::remove_file(dir().join(format!("{id}.txt"))); }
