use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub onboarded: bool,
    pub search_engine: String, // duckduckgo | google | brave
    pub tabs_wide: bool,
    pub tabs_on_top: bool,
    pub suspend_background_tabs: bool,
    pub suspend_secs: u64,
    pub volume: f32,
    pub vpn_enabled: bool,
    pub vpn_exit_country: String,
    pub kill_switch: bool,
    pub dns_mode: String,   // system | quad9 | cloudflare | google | custom
    pub dns_custom: String, // IP, z.B. 9.9.9.9
    pub bl_update: String,  // startup | on_open | interval | monthly | manual
    pub bl_interval_hours: u64,
    pub blocklists: Vec<BlockList>,
    pub ad_block: bool,
    pub fingerprint_protect: bool,
    pub webrtc_protect: bool,
    pub https_only: bool,
    pub strip_tracking: bool,
    pub site_rules: HashMap<String, SiteRule>,
    pub quick_dials: Vec<Dial>,
    pub keymap: HashMap<String, String>,
    pub clear_on_exit: bool,
    pub accent: String,
    pub ext_disabled: Vec<String>,
    pub block_third_party_cookies: bool,
    pub sync_folder: String,
    pub sync_ts: u64,
    pub update_repo: String,
    pub restore_session: bool,
    pub block_autoplay: bool,
    pub block_popups: bool,
    pub cookie_banners: bool,
    pub mouse_gestures: bool,
    pub tor_per_tab: bool,
    pub bridges: String,
    pub theme: String,
    pub weather_city: String,
    pub hints_enabled: bool,
    pub hint_key: String,
    pub hint_chars: String,
    pub tts_voice: String,
    pub tts_rate: i32,
    pub tts_cmd: String,
    pub vt_enabled: bool,
    pub vt_key: String,
    pub hibp_enabled: bool,
    pub https_interstitial: bool,
    pub stats_enabled: bool,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Workspace { pub name: String, pub tabs: Vec<crate::data::SessTab>, pub active: usize }

/// Pro-Seite-Overrides (None = globalen Standard verwenden)
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct SiteRule {
    #[serde(skip_serializing_if = "Option::is_none")] pub adblock: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub fingerprint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub webrtc: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub https_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub strip_tracking: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub dark: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub zoom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub cookies: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub hints: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub block_cam: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub block_mic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")] pub block_geo: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Dial { pub name: String, pub url: String }

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct BlockList {
    pub id: String,
    pub name: String,
    pub source: String,   // url | path
    pub location: String, // Link oder Datei/ZIP/Ordner
    pub enabled: bool,
    pub last_updated: u64,
    pub count: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            onboarded: false,
            search_engine: "duckduckgo".into(),
            tabs_wide: false,
            tabs_on_top: false,
            suspend_background_tabs: true,
            suspend_secs: 120,
            volume: 1.0,
            vpn_enabled: false,
            vpn_exit_country: "de".into(),
            kill_switch: true,
            dns_mode: "system".into(),
            dns_custom: "9.9.9.9".into(),
            bl_update: "startup".into(),
            bl_interval_hours: 24,
            blocklists: vec![],
            ad_block: true,
            fingerprint_protect: true,
            webrtc_protect: true,
            https_only: true,
            strip_tracking: true,
            site_rules: HashMap::new(),
            keymap: HashMap::new(),
            clear_on_exit: false,
            accent: "#5ad1ff".into(),
            ext_disabled: vec![],
            block_third_party_cookies: false,
            sync_folder: String::new(),
            sync_ts: 0,
            update_repo: crate::release_config::DEFAULT_REPOSITORY.into(),
            restore_session: true,
            block_autoplay: true,
            block_popups: true,
            cookie_banners: true,
            mouse_gestures: false,
            tor_per_tab: false,
            bridges: String::new(),
            theme: "liquid".into(),
            weather_city: String::new(),
            hints_enabled: true,
            hint_key: "f".into(),
            hint_chars: "asdfghjklqwer".into(),
            tts_voice: String::new(),
            tts_rate: 0,
            tts_cmd: String::new(),
            vt_enabled: false,
            vt_key: String::new(),
            hibp_enabled: true,
            https_interstitial: true,
            stats_enabled: true,
            workspaces: vec![],
            active_workspace: String::new(),
            quick_dials: ["DuckDuckGo|https://duckduckgo.com", "Wikipedia|https://de.wikipedia.org", "GitHub|https://github.com", "YouTube|https://www.youtube.com"]
                .iter().map(|d| { let (n, u) = d.split_once('|').unwrap(); Dial { name: n.into(), url: u.into() } }).collect(),
        }
    }
}

impl Settings {
    fn path() -> std::path::PathBuf {
        crate::data::home().join("settings.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }
    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(p, serde_json::to_string_pretty(self).unwrap());
    }
    pub fn home_url(&self) -> String {
        match self.search_engine.as_str() {
            "google" => "https://www.google.com".into(),
            "brave" => "https://search.brave.com".into(),
            _ => "https://duckduckgo.com".into(),
        }
    }
    pub fn search_url(&self, q: &str) -> String {
        let q = q.trim().replace(' ', "+");
        match self.search_engine.as_str() {
            "google" => format!("https://www.google.com/search?q={q}"),
            "brave" => format!("https://search.brave.com/search?q={q}"),
            _ => format!("https://duckduckgo.com/?q={q}"),
        }
    }
}

pub fn resolve_input(input: &str, s: &Settings) -> String {
    let t = input.trim();
    if t.starts_with("http://") || t.starts_with("https://") { t.into() }
    else if t.contains('.') && !t.contains(' ') { format!("https://{t}") }
    else { s.search_url(t) }
}
