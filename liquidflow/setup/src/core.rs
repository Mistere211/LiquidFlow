//! Installer-/Updater-Logik (reines Rust, externe Windows-Werkzeuge nur über std::process::Command).
//! Layout wie bei Chrome/Squirrel:  <root>\versions\<ver>\liquidflow.exe  +  <root>\LiquidFlowSetup.exe (Updater/Deinstaller)
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = match option_env!("LIQUIDFLOW_REPO") { Some(r) => r, None => "" };
pub const ASSET: &str = "liquidflow-app-windows-x86_64.zip";
pub const APP_EXE: &str = "liquidflow.exe";
pub const SETUP_EXE: &str = "LiquidFlowSetup.exe";

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Config { pub repo: String, pub updates: bool }

pub struct Opts {
    pub root: PathBuf, pub desktop: bool, pub updates: bool, pub defaults: bool,
    pub repo: String, pub payload: Option<PathBuf>, pub version: Option<String>, pub api: Option<String>,
}

pub struct Release { pub version: String, pub zip: String, pub sha: Option<String> }

pub fn default_root() -> PathBuf { dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("LiquidFlow") }
pub fn user_data_dir() -> PathBuf { dirs::config_dir().unwrap_or_default().join("LiquidFlow") }

fn cmd(p: &str) -> Command {
    let mut c = Command::new(p);
    #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
    c
}

fn nums(v: &str) -> Vec<u32> { v.trim_start_matches('v').split('.').map(|x| x.parse().unwrap_or(0)).collect() }
pub fn newer(a: &str, b: &str) -> bool { nums(a) > nums(b) }

pub fn current_version(root: &Path) -> String { std::fs::read_to_string(root.join("current.txt")).unwrap_or_default().trim().to_string() }
pub fn app_path(root: &Path, ver: &str) -> PathBuf { root.join("versions").join(ver).join(APP_EXE) }

pub fn latest(repo: &str, api: Option<&str>) -> Result<Release, String> {
    if repo.is_empty() && api.is_none() { return Err("Es ist kein Download-Repository eingestellt.".into()); }
    let url = api.map(String::from).unwrap_or_else(|| format!("https://api.github.com/repos/{repo}/releases/latest"));
    let r: serde_json::Value = ureq::get(&url).set("User-Agent", "LiquidFlowSetup").call()
        .map_err(|e| format!("Server nicht erreichbar ({e})"))?.into_json().map_err(|e| e.to_string())?;
    let tag = r["tag_name"].as_str().ok_or("Es wurde noch kein Release veröffentlicht.")?;
    let find = |n: &str| r["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(n))).and_then(|x| x["browser_download_url"].as_str()).map(String::from);
    Ok(Release { version: tag.trim_start_matches('v').into(), zip: find(ASSET).ok_or("Das Programmpaket fehlt im Release.")?, sha: find(&format!("{ASSET}.sha256")) })
}

fn download(url: &str, dest: &Path, prog: &mut dyn FnMut(f32)) -> Result<(), String> {
    let resp = ureq::get(url).set("User-Agent", "LiquidFlowSetup").call().map_err(|e| format!("Download fehlgeschlagen ({e})"))?;
    let total: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut r = resp.into_reader();
    let mut f = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let (mut buf, mut got) = (vec![0u8; 64 * 1024], 0u64);
    loop {
        let n = r.read(&mut buf).map_err(|e| format!("Download abgebrochen ({e})"))?;
        if n == 0 { break; }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if total > 0 { prog(got as f32 / total as f32); }
    }
    Ok(())
}

fn sha256_file(p: &Path) -> Result<String, String> {
    let mut f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let (mut h, mut buf) = (Sha256::new(), vec![0u8; 64 * 1024]);
    loop { let n = f.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(format!("{:x}", h.finalize()))
}

/// Paket holen (Download oder lokale Datei), prüfen, nach versions\<ver> entpacken.
fn fetch_and_extract(root: &Path, o: &Opts, prog: &mut dyn FnMut(f32, &str)) -> Result<String, String> {
    let tmp = root.join("tmp");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("Ordner kann nicht angelegt werden: {e}"))?;
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
        match &rel.sha {
            Some(u) => {
                let want = ureq::get(u).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
                let want = want.split_whitespace().next().unwrap_or("").to_lowercase();
                if sha256_file(&zip)? != want { return Err("Die Prüfsumme des Downloads stimmt nicht – Installation abgebrochen.".into()); }
            }
            None => return Err("Im Release fehlt die Prüfsumme – Installation abgebrochen.".into()),
        }
    }
    prog(0.8, "Entpacke…");
    let stage = root.join("versions").join(format!("{version}.tmp"));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(std::fs::File::open(&zip).map_err(|e| e.to_string())?).map_err(|_| "Das Paket ist beschädigt.".to_string())?;
    z.extract(&stage).map_err(|e| format!("Entpacken fehlgeschlagen ({e})"))?;
    if !stage.join(APP_EXE).exists() { let _ = std::fs::remove_dir_all(&stage); return Err("Das Paket ist unvollständig (liquidflow.exe fehlt).".into()); }
    let dest = root.join("versions").join(&version);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&stage, &dest).map_err(|e| format!("Version kann nicht aktiviert werden ({e})"))?;
    let _ = std::fs::remove_dir_all(&tmp);
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
    v.sort_by(|a, b| nums(b).cmp(&nums(a)));
    for old in v.into_iter().skip(2).filter(|x| x != keep) { let _ = std::fs::remove_dir_all(root.join("versions").join(old)); }
}

fn save_config(root: &Path, c: &Config) { let _ = std::fs::write(root.join("install.json"), serde_json::to_string_pretty(c).unwrap_or_default()); }
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
    std::fs::write(root.join("current.txt"), &version).map_err(|e| e.to_string())?;
    save_config(root, &Config { repo: if o.repo.is_empty() { REPO.into() } else { o.repo.clone() }, updates: o.updates });
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
    let cur = current_version(root);
    let rel = latest(repo, api)?;
    if !newer(&rel.version, &cur) { return Ok(None); }
    let o = Opts { root: root.into(), desktop: false, updates: true, defaults: false, repo: repo.into(), payload: None, version: None, api: api.map(String::from) };
    let version = fetch_and_extract(root, &o, &mut |_, _| {})?;
    std::fs::write(root.join("current.txt"), &version).map_err(|e| e.to_string())?;
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
