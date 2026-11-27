//! Installer-/Updater-Logik (reines Rust, externe Windows-Werkzeuge nur über std::process::Command).
//! Layout wie bei Chrome/Squirrel:  <root>\versions\<ver>\liquidflow.exe  +  <root>\LiquidFlowSetup.exe (Updater/Deinstaller)
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::collections::HashSet;
use crate::release_config;

pub const REPO: &str = release_config::DEFAULT_REPOSITORY;
pub const APP_EXE: &str = "liquidflow.exe";
pub const SETUP_EXE: &str = "LiquidFlowSetup.exe";
const MAX_PACKAGE_BYTES: u64 = 500 * 1024 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct CleanupDir(PathBuf);
impl Drop for CleanupDir {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
struct CleanupFile(PathBuf);
impl Drop for CleanupFile {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Config { pub repo: String, pub updates: bool }

pub struct Opts {
    pub root: PathBuf, pub desktop: bool, pub updates: bool, pub defaults: bool,
    pub repo: String, pub payload: Option<PathBuf>, pub version: Option<String>, pub api: Option<String>,
}

pub struct Release { pub version: String, pub zip: String, pub sha: String }

pub fn default_root() -> PathBuf { dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("LiquidFlow") }
pub fn user_data_dir() -> PathBuf { dirs::config_dir().unwrap_or_default().join("LiquidFlow") }

fn cmd(p: &str) -> Command {
    let mut c = Command::new(p);
    #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
    c
}

pub fn newer(a: &str, b: &str) -> bool {
    release_config::compare_versions(a, b) == Some(std::cmp::Ordering::Greater)
}

pub fn current_version(root: &Path) -> String {
    let version = std::fs::read_to_string(root.join("current.txt")).unwrap_or_default().trim().to_string();
    if release_config::compare_versions(&version, &version).is_some() { version } else { String::new() }
}
pub fn app_path(root: &Path, ver: &str) -> PathBuf { root.join("versions").join(ver).join(APP_EXE) }

fn write_current(root: &Path, version: &str) -> Result<(), String> {
    let id = TEMP_ID.fetch_add(1, AtomicOrdering::Relaxed);
    let current = root.join("current.txt");
    let temp = root.join(format!("current.{}.{}.tmp", std::process::id(), id));
    let backup = root.join(format!("current.{}.{}.bak", std::process::id(), id));
    let _temp_cleanup = CleanupFile(temp.clone());
    let mut file = std::fs::File::create(&temp).map_err(|e| format!("Versionsdatei kann nicht angelegt werden ({e})."))?;
    file.write_all(version.as_bytes()).map_err(|e| format!("Versionsdatei kann nicht geschrieben werden ({e})."))?;
    file.sync_all().map_err(|e| format!("Versionsdatei kann nicht abgeschlossen werden ({e})."))?;
    drop(file);

    let had_current = current.exists();
    if had_current {
        std::fs::rename(&current, &backup).map_err(|e| format!("Vorherige Version kann nicht gesichert werden ({e})."))?;
    }
    if let Err(error) = std::fs::rename(&temp, &current) {
        if had_current { let _ = std::fs::rename(&backup, &current); }
        return Err(format!("Neue Version kann nicht aktiviert werden ({error})."));
    }
    if had_current { let _ = std::fs::remove_file(backup); }
    Ok(())
}

fn request_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, _) => release_config::github_status_error(status),
        ureq::Error::Transport(error) => format!("GitHub-Repository nicht erreichbar ({error})."),
    }
}

pub fn latest(repo: &str, api: Option<&str>) -> Result<Release, String> {
    let repo = if repo.trim().is_empty() { REPO } else { repo.trim() };
    let url = match api {
        Some(url) if url.starts_with("https://") => url.to_string(),
        Some(_) => return Err("Die Release-API muss eine HTTPS-Adresse verwenden.".into()),
        None => release_config::latest_release_api(repo)?,
    };
    let response = ureq::get(&url)
        .set("User-Agent", "LiquidFlowSetup")
        .call()
        .map_err(request_error)?;
    let json: serde_json::Value = response.into_json()
        .map_err(|e| format!("GitHub-Antwort ist ungültig ({e})."))?;
    let checksum_name = format!("{}{}", release_config::APP_ASSET, release_config::SHA256_SUFFIX);
    let assets = release_config::release_assets(&json, release_config::APP_ASSET, &checksum_name)?;
    let version = assets.tag.strip_prefix('v').unwrap_or(&assets.tag).to_string();
    Ok(Release { version, zip: assets.download_url, sha: assets.checksum_url })
}

fn download(url: &str, dest: &Path, prog: &mut dyn FnMut(f32)) -> Result<(), String> {
    let resp = ureq::get(url).set("User-Agent", "LiquidFlowSetup").call().map_err(request_error)?;
    let total: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut r = resp.into_reader();
    let mut f = std::fs::File::create(dest).map_err(|e| format!("Download-Datei kann nicht angelegt werden ({e})."))?;
    let (mut buf, mut got) = (vec![0u8; 64 * 1024], 0u64);
    loop {
        let n = r.read(&mut buf).map_err(|e| format!("Download fehlgeschlagen ({e})."))?;
        if n == 0 { break; }
        got += n as u64;
        if got > MAX_PACKAGE_BYTES { return Err("Das Programmpaket überschreitet die erlaubte Größe.".into()); }
        f.write_all(&buf[..n]).map_err(|e| format!("Download konnte nicht gespeichert werden ({e})."))?;
        if total > 0 { prog((got as f32 / total as f32).min(1.0)); }
    }
    if total > 0 && got != total { return Err("Download unvollständig – Größenprüfung fehlgeschlagen.".into()); }
    f.sync_all().map_err(|e| format!("Download konnte nicht abgeschlossen werden ({e})."))?;
    Ok(())
}

fn sha256_file(p: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 64 * 1024]);
    loop { let n = f.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(format!("{:x}", h.finalize()))
}

fn safe_package_entry(path: &str) -> Option<&str> {
    if path.contains('\\') {
        return None;
    }
    let path = path.trim_end_matches('/');
    if path == "payload" {
        return Some("");
    }
    let path = path.strip_prefix("payload/").unwrap_or(path);
    [APP_EXE, "pt", "pt/lyrebird.exe", "pt/snowflake-client.exe"]
        .contains(&path)
        .then_some(path)
}

/// Paket holen (Download oder lokale Datei), prüfen, nach versions\<ver> entpacken.
fn fetch_and_extract(root: &Path, o: &Opts, prog: &mut dyn FnMut(f32, &str)) -> Result<String, String> {
    let id = TEMP_ID.fetch_add(1, AtomicOrdering::Relaxed);
    let tmp = root.join("tmp").join(format!("setup-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("Ordner kann nicht angelegt werden: {e}"))?;
    let _tmp_cleanup = CleanupDir(tmp.clone());
    let zip = tmp.join("payload.zip");
    let version;
    if let Some(p) = &o.payload {
        std::fs::copy(p, &zip).map_err(|e| e.to_string())?;
        version = o.version.clone().unwrap_or_else(|| "0.0.0".into());
        prog(0.7, "Paket gefunden");
    } else {
        prog(0.02, "Suche aktuelle Version…");
        let rel = latest(&o.repo, o.api.as_deref())?;
        version = rel.version.clone();
        download(&rel.zip, &zip, &mut |p| prog(0.05 + p * 0.65, "Lade Liquid Flow herunter…"))?;
        prog(0.72, "Prüfe Download…");
        let sidecar = ureq::get(&rel.sha).set("User-Agent", "LiquidFlowSetup")
            .call().map_err(request_error)?
            .into_string().map_err(|e| format!("SHA-256-Datei konnte nicht gelesen werden ({e})."))?;
        let want = release_config::parse_sha256(&sidecar)
            .ok_or("Die SHA-256-Datei im Release ist ungültig.")?;
        if !release_config::sha256_matches(&sha256_file(&zip)?, &want) {
            return Err("SHA-256-Prüfung fehlgeschlagen – Installation abgebrochen.".into());
        }
    }
    prog(0.8, "Entpacke…");
    if release_config::compare_versions(&version, &version).is_none() {
        return Err("Das Release enthält eine ungültige Versionsnummer.".into());
    }
    std::fs::create_dir_all(root.join("versions")).map_err(|e| e.to_string())?;
    let stage = root.join("versions").join(format!("{version}.{}.{}.tmp", std::process::id(), id));
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let _stage_cleanup = CleanupDir(stage.clone());
    let mut z = zip::ZipArchive::new(std::fs::File::open(&zip).map_err(|e| e.to_string())?).map_err(|_| "Das Paket ist beschädigt.".to_string())?;
    let mut names = HashSet::new();
    for name in z.file_names() {
        let Some(name) = safe_package_entry(name) else {
            return Err("Das Paket enthält nicht erlaubte Dateien oder Dateipfade.".into());
        };
        if !name.is_empty() && !names.insert(name.to_string()) {
            return Err("Das Paket enthält doppelte Dateien.".into());
        }
    }
    z.extract(&stage).map_err(|e| format!("Entpacken fehlgeschlagen ({e})"))?;
    let payload = stage.join("payload");
    if payload.is_dir() {
        for entry in std::fs::read_dir(&payload).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let dest = stage.join(entry.file_name());
            if dest.exists() {
                return Err("Das Paket enthält doppelte Dateien.".into());
            }
            std::fs::rename(entry.path(), dest).map_err(|e| format!("Paketstruktur kann nicht aktiviert werden ({e})."))?;
        }
        std::fs::remove_dir(&payload).map_err(|e| e.to_string())?;
    }
    if !stage.join(APP_EXE).is_file() { return Err("Das Paket ist unvollständig (liquidflow.exe fehlt).".into()); }
    let dest = root.join("versions").join(&version);
    if dest.exists() {
        let backup = root.join("versions").join(format!("{version}.{}.{}.bak", std::process::id(), id));
        std::fs::rename(&dest, &backup).map_err(|e| format!("Vorhandene Version kann nicht gesichert werden ({e})."))?;
        if let Err(error) = std::fs::rename(&stage, &dest) {
            let _ = std::fs::rename(&backup, &dest);
            return Err(format!("Version kann nicht aktiviert werden ({error})."));
        }
        let _ = std::fs::remove_dir_all(backup);
    } else {
        std::fs::rename(&stage, &dest).map_err(|e| format!("Version kann nicht aktiviert werden ({e})."))?;
    }
    Ok(version)
}

fn ps_quote(s: &str) -> String { format!("'{}'", s.replace('\'', "''")) }

/// Verknüpfungen: Startmenü immer, Desktop je nach Modus ("create" | "existing" | "skip")
fn shortcuts(exe: &Path, desktop: &str) {
    if !cfg!(windows) { return; }
    let (e, d) = (ps_quote(&exe.to_string_lossy()), ps_quote(&exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()));
    let script = format!(
        "$ws=New-Object -ComObject WScript.Shell;\
         function mk($p){{$s=$ws.CreateShortcut($p);$s.TargetPath={e};$s.WorkingDirectory={d};$s.IconLocation=({e}+',0');$s.Description='Liquid Flow';$s.Save()}};\
         mk (Join-Path ([Environment]::GetFolderPath('Programs')) 'Liquid Flow.lnk');\
         $dk=Join-Path ([Environment]::GetFolderPath('Desktop')) 'Liquid Flow.lnk';\
         if('{desktop}' -eq 'create' -or ('{desktop}' -eq 'existing' -and (Test-Path $dk))){{mk $dk}}");
    let _ = cmd("powershell").args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]).output();
}

fn reg(key: &str, name: Option<&str>, kind: &str, val: &str) {
    let mut c = cmd("reg");
    c.args(["add", &format!("HKCU\\{key}")]);
    match name { Some(n) => { c.args(["/v", n]); } None => { c.arg("/ve"); } }
    let _ = c.args(["/t", kind, "/d", val, "/f"]).output();
}

fn register(root: &Path, exe: &Path, version: &str) {
    if !cfg!(windows) { return; }
    let (e, setup) = (exe.to_string_lossy().into_owned(), root.join(SETUP_EXE).to_string_lossy().into_owned());
    let sm = "Software\\Clients\\StartMenuInternet\\LiquidFlow";
    reg(sm, None, "REG_SZ", "Liquid Flow");
    reg(&format!("{sm}\\DefaultIcon"), None, "REG_SZ", &format!("\"{e}\",0"));
    reg(&format!("{sm}\\shell\\open\\command"), None, "REG_SZ", &format!("\"{e}\""));
    reg(&format!("{sm}\\Capabilities"), Some("ApplicationName"), "REG_SZ", "Liquid Flow");
    reg(&format!("{sm}\\Capabilities"), Some("ApplicationDescription"), "REG_SZ", "Schneller, privater Browser in Rust");
    reg(&format!("{sm}\\Capabilities\\URLAssociations"), Some("http"), "REG_SZ", "LiquidFlowURL");
    reg(&format!("{sm}\\Capabilities\\URLAssociations"), Some("https"), "REG_SZ", "LiquidFlowURL");
    reg("Software\\RegisteredApplications", Some("LiquidFlow"), "REG_SZ", &format!("{sm}\\Capabilities"));
    reg("Software\\Classes\\LiquidFlowURL", None, "REG_SZ", "Liquid Flow URL");
    reg("Software\\Classes\\LiquidFlowURL", Some("URL Protocol"), "REG_SZ", "");
    reg("Software\\Classes\\LiquidFlowURL\\shell\\open\\command", None, "REG_SZ", &format!("\"{e}\" \"%1\""));
    let un = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\LiquidFlow";
    reg(un, Some("DisplayName"), "REG_SZ", "Liquid Flow");
    reg(un, Some("DisplayVersion"), "REG_SZ", version);
    reg(un, Some("Publisher"), "REG_SZ", "Liquid Flow");
    reg(un, Some("InstallLocation"), "REG_SZ", &root.to_string_lossy());
    reg(un, Some("DisplayIcon"), "REG_SZ", &e);
    reg(un, Some("UninstallString"), "REG_SZ", &format!("\"{setup}\" --uninstall"));
    reg(un, Some("NoModify"), "REG_DWORD", "1");
    reg(un, Some("NoRepair"), "REG_DWORD", "1");
}

/// Hintergrund-Updater: stündlich per Aufgabenplanung + bei jeder Anmeldung (Run-Schlüssel), ohne Administratorrechte.
fn updater_task(root: &Path, on: bool) {
    if !cfg!(windows) { return; }
    let tr = format!("\"{}\" --update --silent", root.join(SETUP_EXE).display());
    let _ = cmd("schtasks").args(["/Delete", "/TN", "LiquidFlow Update", "/F"]).output();
    let _ = cmd("reg").args(["delete", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "LiquidFlowUpdate", "/f"]).output();
    if on {
        let _ = cmd("schtasks").args(["/Create", "/F", "/SC", "HOURLY", "/MO", "5", "/TN", "LiquidFlow Update", "/TR", &tr]).output();
        reg("Software\\Microsoft\\Windows\\CurrentVersion\\Run", Some("LiquidFlowUpdate"), "REG_SZ", &tr);
    }
}

fn prune_versions(root: &Path, keep: &str) {
    let Ok(rd) = std::fs::read_dir(root.join("versions")) else { return };
    let mut v: Vec<String> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort_by(|a, b| {
        release_config::compare_versions(b, a).unwrap_or(std::cmp::Ordering::Equal)
    });
    for old in v.into_iter().skip(2).filter(|x| x != keep) { let _ = std::fs::remove_dir_all(root.join("versions").join(old)); }
}

fn save_config(root: &Path, c: &Config) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(c).map_err(|e| format!("Installationskonfiguration ist ungültig ({e})."))?;
    std::fs::write(root.join("install.json"), json).map_err(|e| format!("Installationskonfiguration kann nicht gespeichert werden ({e})."))
}
pub fn load_config(root: &Path) -> Config { std::fs::read_to_string(root.join("install.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default() }

pub fn install(o: &Opts, prog: &mut dyn FnMut(f32, &str)) -> Result<String, String> {
    let root = &o.root;
    std::fs::create_dir_all(root).map_err(|e| format!("Der Ordner kann nicht angelegt werden ({e})"))?;
    let version = fetch_and_extract(root, o, prog)?;
    prog(0.88, "Richte Liquid Flow ein…");
    // Setup-Programm als Updater/Deinstaller neben die Versionen legen
    if let Ok(me) = std::env::current_exe() {
        let target = root.join(SETUP_EXE);
        if me != target { let _ = std::fs::remove_file(&target); std::fs::copy(&me, &target).map_err(|e| format!("Updater kann nicht kopiert werden ({e})"))?; }
    }
    save_config(root, &Config { repo: if o.repo.trim().is_empty() { REPO.into() } else { o.repo.trim().into() }, updates: o.updates })?;
    write_current(root, &version)?;
    let exe = app_path(root, &version);
    shortcuts(&exe, if o.desktop { "create" } else { "skip" });
    prog(0.94, "Registriere Browser…");
    register(root, &exe, &version);
    updater_task(root, o.updates);
    prune_versions(root, &version);
    if o.defaults && cfg!(windows) { let _ = cmd("explorer").arg("ms-settings:defaultapps").spawn(); }
    prog(1.0, "Fertig");
    Ok(version)
}

/// Update ohne Oberfläche. Ok(None) = schon aktuell.
pub fn update(root: &Path, repo: &str, api: Option<&str>) -> Result<Option<String>, String> {
    let repo = if repo.trim().is_empty() { REPO } else { repo.trim() };
    let cur = current_version(root);
    let rel = latest(repo, api)?;
    if !newer(&rel.version, &cur) { return Ok(None); }
    let o = Opts { root: root.into(), desktop: false, updates: true, defaults: false, repo: repo.into(), payload: None, version: None, api: api.map(String::from) };
    let version = fetch_and_extract(root, &o, &mut |_, _| {})?;
    let config = load_config(root);
    save_config(root, &Config { repo: repo.to_string(), updates: config.updates })?;
    write_current(root, &version)?;
    let exe = app_path(root, &version);
    shortcuts(&exe, "existing");
    register(root, &exe, &version);
    prune_versions(root, &version);
    Ok(Some(version))
}

pub fn launch(root: &Path) -> bool {
    let exe = app_path(root, &current_version(root));
    exe.exists() && Command::new(exe).spawn().is_ok()
}

pub fn uninstall(root: &Path, delete_data: bool, prog: &mut dyn FnMut(f32, &str)) -> Result<(), String> {
    prog(0.1, "Beende Liquid Flow…");
    if cfg!(windows) {
        let _ = cmd("taskkill").args(["/F", "/IM", APP_EXE]).output();
        prog(0.3, "Entferne Verknüpfungen…");
        let _ = cmd("powershell").args(["-NoProfile", "-NonInteractive", "-Command",
            "Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path ([Environment]::GetFolderPath('Programs')) 'Liquid Flow.lnk'),(Join-Path ([Environment]::GetFolderPath('Desktop')) 'Liquid Flow.lnk')"]).output();
        prog(0.5, "Entferne Einträge…");
        updater_task(root, false);
        for k in ["Software\\Clients\\StartMenuInternet\\LiquidFlow", "Software\\Classes\\LiquidFlowURL", "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\LiquidFlow"] {
            let _ = cmd("reg").args(["delete", &format!("HKCU\\{k}"), "/f"]).output();
        }
        let _ = cmd("reg").args(["delete", "HKCU\\Software\\RegisteredApplications", "/v", "LiquidFlow", "/f"]).output();
    }
    prog(0.8, "Lösche Programmdateien…");
    if delete_data { let _ = std::fs::remove_dir_all(user_data_dir()); }
    // Das laufende Setup liegt selbst im Ordner: Löschen mit kurzer Verzögerung durch eine abgetrennte Shell
    if cfg!(windows) {
        let _ = cmd("cmd").args(["/C", &format!("ping 127.0.0.1 -n 4 >nul & rmdir /S /Q \"{}\"", root.display())]).spawn();
    } else { let _ = std::fs::remove_dir_all(root); }
    prog(1.0, "Fertig");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_versions_without_string_ordering() {
        assert!(newer("v1.0.0", "0.99.99"));
        assert!(newer("1.10.0", "1.9.9"));
        assert!(!newer("1.0", "1.0.0"));
        assert!(!newer("../1.1.0", "1.0.0"));
    }

    #[test]
    fn accepts_only_the_expected_application_package_files() {
        assert_eq!(safe_package_entry("liquidflow.exe"), Some("liquidflow.exe"));
        assert_eq!(safe_package_entry("payload/pt/lyrebird.exe"), Some("pt/lyrebird.exe"));
        assert_eq!(safe_package_entry("payload/"), Some(""));
        assert_eq!(safe_package_entry("../outside.exe"), None);
        assert_eq!(safe_package_entry("payload/other.exe"), None);
        assert_eq!(safe_package_entry("payload\\liquidflow.exe"), None);
    }
}
