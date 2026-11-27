//! Auto-Update über GitHub Releases. Das Release-Asset wird vor dem Start per SHA-256 geprüft.
use crate::release_config;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

const MAX_INSTALLER_BYTES: u64 = 300 * 1024 * 1024;
static DOWNLOAD_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct Release {
    pub tag: String,
    pub exe: String,
    pub sha: String,
}

fn status_error(status: u16) -> String {
    release_config::github_status_error(status)
}

fn request_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, _) => status_error(status),
        ureq::Error::Transport(error) => format!("GitHub-Repository nicht erreichbar ({error})."),
    }
}

pub fn check(repo: &str) -> Result<Option<Release>, String> {
    let repo = if repo.trim().is_empty() {
        release_config::DEFAULT_REPOSITORY
    } else {
        repo.trim()
    };
    let api = release_config::latest_release_api(repo)?;
    let response = ureq::get(&api)
        .set("User-Agent", "LiquidFlow")
        .call()
        .map_err(request_error)?;
    let json: serde_json::Value = response
        .into_json()
        .map_err(|error| format!("GitHub-Antwort ist ungültig ({error})."))?;
    let checksum_name = format!(
        "{}{}",
        release_config::OFFLINE_SETUP_ASSET,
        release_config::SHA256_SUFFIX
    );
    let assets = release_config::release_assets(
        &json,
        release_config::OFFLINE_SETUP_ASSET,
        &checksum_name,
    )?;
    let current = env!("CARGO_PKG_VERSION");
    match release_config::compare_versions(&assets.tag, current) {
        Some(Ordering::Greater) => Ok(Some(Release {
            tag: assets.tag,
            exe: assets.download_url,
            sha: assets.checksum_url,
        })),
        Some(_) => Ok(None),
        None => Err("Das Release oder die installierte Version hat ein ungültiges Format.".into()),
    }
}

pub fn download(
    release: &Release,
    progress: &mut dyn FnMut(f32),
) -> Result<std::path::PathBuf, String> {
    let checksum = ureq::get(&release.sha)
        .set("User-Agent", "LiquidFlow")
        .call()
        .map_err(request_error)?
        .into_string()
        .map_err(|error| format!("SHA-256-Datei konnte nicht gelesen werden ({error})."))?;
    let expected = release_config::parse_sha256(&checksum)
        .ok_or("Die SHA-256-Datei im Release ist ungültig.")?;

    let response = ureq::get(&release.exe)
        .set("User-Agent", "LiquidFlow")
        .call()
        .map_err(request_error)?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    if total > MAX_INSTALLER_BYTES {
        return Err("Das Update ist größer als die erlaubte Installergröße.".into());
    }

    let id = DOWNLOAD_ID.fetch_add(1, AtomicOrdering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "LiquidFlow-Offline-Setup-{}-{id}.exe",
        std::process::id()
    ));
    let partial = base.with_extension("exe.part");
    let result = (|| {
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&partial)
            .map_err(|error| format!("Temporäre Update-Datei kann nicht angelegt werden ({error})."))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut received = 0u64;
        loop {
            let count = reader
                .read(&mut buf)
                .map_err(|error| format!("Download fehlgeschlagen ({error})."))?;
            if count == 0 {
                break;
            }
            received += count as u64;
            if received > MAX_INSTALLER_BYTES {
                return Err("Das Update ist größer als die erlaubte Installergröße.".into());
            }
            file.write_all(&buf[..count])
                .map_err(|error| format!("Update konnte nicht gespeichert werden ({error})."))?;
            hasher.update(&buf[..count]);
            if total > 0 {
                progress((received as f32 / total as f32).min(1.0));
            }
        }
        file.sync_all()
            .map_err(|error| format!("Update konnte nicht abgeschlossen werden ({error})."))?;
        let actual = format!("{:x}", hasher.finalize());
        if !release_config::sha256_matches(&actual, &expected) {
            return Err("SHA-256-Prüfung fehlgeschlagen – Update abgebrochen.".into());
        }
        std::fs::rename(&partial, &base)
            .map_err(|error| format!("Geprüfter Installer kann nicht aktiviert werden ({error})."))?;
        progress(1.0);
        Ok(base.clone())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
        let _ = std::fs::remove_file(&base);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_only_versions_newer_than_the_running_browser() {
        let current = env!("CARGO_PKG_VERSION");
        assert_eq!(
            release_config::compare_versions("v99.0.0", current),
            Some(Ordering::Greater)
        );
        assert_eq!(
            release_config::compare_versions(current, current),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn a_private_repository_response_is_not_misreported_as_no_update() {
        let error = status_error(404);
        assert!(error.contains("privat"));
        assert!(error.contains("Update-Backend"));
    }
}
