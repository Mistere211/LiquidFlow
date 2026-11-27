use std::cmp::Ordering;
use sha2::{Digest, Sha256};

pub const DEFAULT_REPOSITORY: &str = "Mistere211/LiquidFlow";
pub const REPOSITORY_URL: &str = "https://github.com/Mistere211/LiquidFlow";
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/Mistere211/LiquidFlow/releases/latest";

pub const APP_ASSET: &str = "liquidflow-app-windows-x86_64.zip";
pub const OFFLINE_SETUP_ASSET: &str = "LiquidFlow-Offline-Setup.exe";
pub const SHA256_SUFFIX: &str = ".sha256";

pub struct ReleaseAssets {
    pub tag: String,
    pub download_url: String,
    pub checksum_url: String,
}

pub fn latest_release_api(repository: &str) -> Result<String, String> {
    let repository = repository.trim();
    if repository.is_empty() {
        return Err("Es ist kein Download-Repository eingestellt.".into());
    }

    let mut parts = repository.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err("Das Repository muss im Format owner/name angegeben werden.".into());
    };
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if !valid(owner) || !valid(name) {
        return Err("Das Repository muss im Format owner/name angegeben werden.".into());
    }

    if repository.eq_ignore_ascii_case(DEFAULT_REPOSITORY) {
        Ok(LATEST_RELEASE_API.to_string())
    } else {
        Ok(format!(
            "https://api.github.com/repos/{owner}/{name}/releases/latest"
        ))
    }
}

pub fn github_status_error(status: u16) -> String {
    match status {
        404 => "GitHub hat das Repository oder Release nicht gefunden. Es ist möglicherweise privat; private Releases benötigen ein Update-Backend.".into(),
        401 => "GitHub verlangt eine Authentifizierung. Private Releases benötigen ein Update-Backend; Zugangstokens werden nicht in Liquid Flow gespeichert.".into(),
        403 => "GitHub hat den Zugriff verweigert (Berechtigung oder API-Limit). Private Releases benötigen ein Update-Backend; Client-Tokens werden nicht eingebettet.".into(),
        _ => format!("GitHub-Release-Abfrage fehlgeschlagen (HTTP {status})."),
    }
}

pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    struct Version {
        core: Vec<u64>,
        prerelease: Option<Vec<String>>,
    }

    fn parse(version: &str) -> Option<Version> {
        let version = version.trim().strip_prefix('v').unwrap_or(version.trim());
        let (version, build) = match version.split_once('+') {
            Some((version, build)) => (version, Some(build)),
            None => (version, None),
        };
        if version.is_empty()
            || version.contains('+')
            || build.map_or(false, |build| {
                build.is_empty()
                    || !build.split('.').all(|id| {
                        !id.is_empty()
                            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
            })
        {
            return None;
        }
        let (core, prerelease) = match version.split_once('-') {
            Some((core, prerelease)) if !prerelease.is_empty() => (core, Some(prerelease)),
            Some(_) => return None,
            None => (version, None),
        };
        let core_parts = core.split('.').collect::<Vec<_>>();
        if core_parts
            .iter()
            .any(|part| part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        let mut core = core_parts
            .into_iter()
            .map(|component| {
                if component.is_empty() || !component.bytes().all(|b| b.is_ascii_digit()) {
                    None
                } else {
                    component.parse::<u64>().ok()
                }
            })
            .collect::<Option<Vec<_>>>()?;
        while core.len() > 1 && core.last() == Some(&0) {
            core.pop();
        }
        let prerelease = if let Some(prerelease) = prerelease {
            let ids = prerelease.split('.').map(str::to_string).collect::<Vec<_>>();
            if ids.iter().any(|id| {
                id.is_empty()
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || (id.len() > 1
                        && id.bytes().all(|b| b.is_ascii_digit())
                        && id.starts_with('0'))
            }) {
                return None;
            }
            Some(ids)
        } else {
            None
        };
        Some(Version { core, prerelease })
    }

    let left = parse(left)?;
    let right = parse(right)?;
    for index in 0..left.core.len().max(right.core.len()) {
        match left.core.get(index).copied().unwrap_or(0).cmp(&right.core.get(index).copied().unwrap_or(0)) {
            Ordering::Equal => {}
            ordering => return Some(ordering),
        }
    }
    match (left.prerelease, right.prerelease) {
        (None, None) => Some(Ordering::Equal),
        (None, Some(_)) => Some(Ordering::Greater),
        (Some(_), None) => Some(Ordering::Less),
        (Some(left), Some(right)) => {
            for (left, right) in left.iter().zip(&right) {
                let ordering = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left_num), Ok(right_num)) => left_num.cmp(&right_num),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => left.cmp(right),
                };
                if ordering != Ordering::Equal {
                    return Some(ordering);
                }
            }
            Some(left.len().cmp(&right.len()))
        }
    }
}

pub fn release_assets(
    release: &serde_json::Value,
    asset_name: &str,
    checksum_name: &str,
) -> Result<ReleaseAssets, String> {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("GitHub hat keine gültige Release-Version zurückgegeben.")?;
    if compare_versions(tag, tag).is_none() {
        return Err("Das Release enthält eine ungültige Versionsnummer.".into());
    }
    let find = |name: &str| {
        release["assets"]
            .as_array()?
            .iter()
            .find(|asset| asset["name"].as_str() == Some(name))
            .and_then(|asset| asset["browser_download_url"].as_str())
            .filter(|url| url.starts_with("https://github.com/") && url.contains("/releases/download/"))
            .map(str::to_string)
    };
    let download_url = find(asset_name)
        .ok_or("Kein passendes Windows-Asset gefunden (Release-Datei fehlt).")?;
    let checksum_url = find(checksum_name)
        .ok_or("Im Release fehlt die SHA-256-Prüfsummendatei.")?;
    Ok(ReleaseAssets {
        tag: tag.to_string(),
        download_url,
        checksum_url,
    })
}

pub fn parse_sha256(text: &str) -> Option<String> {
    text.split_whitespace()
        .next()
        .map(|token| token.trim_start_matches('*'))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
}

pub fn sha256_matches(actual: &str, expected: &str) -> bool {
    expected.len() == 64
        && expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        && actual.eq_ignore_ascii_case(expected)
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_canonical_repository_api_and_validates_overrides() {
        assert_eq!(REPOSITORY_URL, "https://github.com/Mistere211/LiquidFlow");
        assert_eq!(latest_release_api(DEFAULT_REPOSITORY).unwrap(), LATEST_RELEASE_API);
        assert_eq!(
            latest_release_api("example/project").unwrap(),
            "https://api.github.com/repos/example/project/releases/latest"
        );
        assert!(latest_release_api("https://example/project").is_err());
        assert!(latest_release_api("../project").is_err());
    }

    #[test]
    fn identifies_private_repository_access_without_suggesting_embedded_credentials() {
        let message = github_status_error(404);
        assert!(message.contains("privat"));
        assert!(message.contains("Update-Backend"));
        assert!(!message.contains("Token"));
    }

    #[test]
    fn compares_numeric_versions_and_ignores_trailing_zero_components() {
        assert_eq!(compare_versions("v1.0.0", "0.99.99"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.0", "v1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.10.0", "1.9.9"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.0.0-beta.2", "1.0.0-beta.11"), Some(Ordering::Less));
        assert_eq!(compare_versions("1.0.0", "1.0.0-beta"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.x", "1.0.0"), None);
    }

    #[test]
    fn parses_only_well_formed_sha256_sidecars() {
        let hash = "a".repeat(64);
        assert_eq!(parse_sha256(&format!("{hash}  archive.zip")), Some(hash.clone()));
        assert_eq!(parse_sha256("not-a-hash"), None);
        assert_eq!(parse_sha256(&"g".repeat(64)), None);
        let expected = "d82be6d23bc8b5f3bc7fc5609b77c05e2c9bcad3a53da42bcdef5223b05ac4ed";
        assert_eq!(sha256(b"LiquidFlow"), expected);
        assert!(sha256_matches(expected, expected));
        assert!(!sha256_matches(&"0".repeat(64), expected));
    }

    #[test]
    fn matches_only_the_requested_release_asset_names() {
        let release = serde_json::json!({
            "tag_name": "v1.0.0",
            "assets": [
                {"name": "other.zip", "browser_download_url": "https://example.com/other.zip"},
                {"name": APP_ASSET, "browser_download_url": "https://github.com/Mistere211/LiquidFlow/releases/download/v1.0.0/file.zip"},
                {"name": "liquidflow-app-windows-x86_64.zip.sha256", "browser_download_url": "https://github.com/Mistere211/LiquidFlow/releases/download/v1.0.0/file.zip.sha256"}
            ]
        });
        let assets = release_assets(
            &release,
            APP_ASSET,
            "liquidflow-app-windows-x86_64.zip.sha256",
        )
        .unwrap();
        assert_eq!(assets.tag, "v1.0.0");
        assert!(assets.download_url.ends_with("/file.zip"));
        assert!(release_assets(&release, "unexpected.exe", "sum.sha256").is_err());
    }
}
