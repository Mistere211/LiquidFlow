#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
macro_rules! wvb {
    ($s:expr) => {{
        let b = wry::WebViewBuilder::new();
        #[cfg(windows)]
        let b = { use wry::WebViewBuilderExtWindows; b.with_additional_browser_args(crate::browser_args($s)) };
        b
    }};
}

mod blocklist;
mod data;
mod ext;
#[cfg(all(windows, feature = "previews"))]
mod preview;
mod features;
mod importer;
mod totp;
mod features2;
mod features3;
mod keys;
mod sync;
mod update;
mod vault;
mod net;
mod settings;
mod shield;
#[path = "../release_config.rs"]
mod release_config;

use settings::{resolve_input, Settings};
use shield::{host_of, Rules, START};
use std::collections::{HashMap, HashSet};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Fullscreen, Window, WindowBuilder};
use wry::{ProxyConfig, ProxyEndpoint, Rect, WebView};

enum Ev {
    Ipc(String), PageIpc(u32, String), DlStart(String, String), DlDone(String, bool), Title(u32, String), Url(u32, String), Navigate(u32, String), Tick, Vpn(String),
    BlPath(std::path::PathBuf), BlRefreshed(String, Result<usize, String>), BlLoaded(usize),
    ExtIpc(String, String, String), ExtInstalled(Result<String, String>), ExtPick(std::path::PathBuf), SyncFolder(std::path::PathBuf),
    Thumb(u32, Vec<u8>),
    Updated(Result<(), String>),
    HttpsProbe(u32, String), ProbeDone(u32, String, bool), AllowHttp(u32, String), Pwned(Vec<(String, Option<u64>)>), Vt(String, String),
    ImportData(Vec<data::Bookmark>, Vec<data::HistEntry>, String), TtsText(String), Notice(String),
    OpenUrl(String), NewWindow(u32, String), Leak(serde_json::Value), BgPick(std::path::PathBuf),
    UpdateAvail(Result<Option<update::Release>, String>), UpdateProgress(f32),
    UpdateReady(Result<std::path::PathBuf, String>), PwCaptured(String, String),
}

fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let h = |n: u64| { let mut x = std::collections::hash_map::RandomState::new().build_hasher(); x.write_u64(n); x.finish() };
    format!("{:016x}{:016x}", h(now()), h(1))
}

fn show_in_folder(p: &str) {
    #[cfg(windows)] { let _ = std::process::Command::new("explorer").arg(format!("/select,{p}")).spawn(); }
    #[cfg(not(windows))] { if let Some(d) = std::path::Path::new(p).parent() { let _ = std::process::Command::new("xdg-open").arg(d).spawn(); } }
}

fn browser_args(s: &Settings) -> String {
    let mut a = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --force-webrtc-ip-handling-policy=disable_non_proxied_udp");
    if s.block_third_party_cookies { a.push_str(" --enable-features=TestThirdPartyCookiePhaseout"); }
    if s.block_autoplay { a.push_str(" --autoplay-policy=document-user-activation-required"); }
    a
}

fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }

const UI: &str = include_str!("ui.html");
const BAR_H: f64 = 92.0;
const STRIP_H: f64 = 44.0;

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { position: LogicalPosition::new(x, y).into(), size: LogicalSize::new(w.max(1.0), h.max(1.0)).into() }
}

struct Tab { id: u32, url: String, title: String, view: Option<WebView>, last_active: Instant, private: bool, flags: Arc<net::TabFlags>, token: String, group: u8, last_gesture: Instant }

struct App {
    window: Window,
    px: EventLoopProxy<Ev>,
    settings: Settings,
    rules: Arc<RwLock<Rules>>,
    tabs: Vec<Tab>,
    active: u32,
    next_id: u32,
    bar: WebView,
    side: WebView,
    overlay: Option<WebView>,
    vpn_status: String,
    net: net::Net,
    refreshing: HashSet<String>,
    bl_total: usize,
    bl_open_done: bool,
    overlay_role: String,
    bookmarks: Vec<data::Bookmark>,
    history: Vec<data::HistEntry>,
    history_dirty: bool,
    closed: Vec<String>,
    downloads: Vec<data::Download>,
    split: Option<u32>,
    exts: Vec<ext::Ext>,
    ext_views: Vec<features::ExtView>,
    pending: HashMap<u64, features::Pending>,
    ext_seq: u64,
    ext_store: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    ext_dirty: bool,
    vault: Option<vault::Vault>,
    vault_err: String,
    vault_last: Instant,
    notice: String,
    release: Option<update::Release>,
    quit: bool,
    tick_n: u32,
    focus: bool,
    mem_mb: u64,
    suggestions: Vec<serde_json::Value>,
    leak: Option<serde_json::Value>,
    thumbs: HashMap<u32, String>,
    preview: Option<u32>,
    preview_xy: (f64, f64),
    gaming: bool,
    tts: Option<std::process::Child>,
    qr: Option<(String, String)>,
    http_ok: HashSet<String>,
    http_nonce: HashMap<String, (u32, String)>,
    pwned: HashMap<String, Option<u64>>,
}

impl App {
    fn size(&self) -> (f64, f64) {
        let s = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        (s.width, s.height)
    }
    fn side_w(&self) -> f64 { if self.focus || self.settings.tabs_on_top { 0.0 } else if self.settings.tabs_wide { 210.0 } else { 64.0 } }
    fn top_h(&self) -> f64 { if self.focus { return 0.0; } BAR_H + if self.settings.tabs_on_top { STRIP_H } else { 0.0 } }
    fn page_rect(&self) -> Rect { let (w, h) = self.size(); rect(self.side_w(), self.top_h(), w - self.side_w(), h - self.top_h()) }

    fn relayout(&self) {
        let (w, h) = self.size();
        let _ = self.bar.set_bounds(rect(0.0, 0.0, w, BAR_H));
        let _ = if self.settings.tabs_on_top { self.side.set_bounds(rect(0.0, BAR_H, w, STRIP_H)) }
                else { self.side.set_bounds(rect(0.0, BAR_H, self.side_w(), h - BAR_H)) };
        for t in &self.tabs { if let Some(v) = &t.view { let _ = v.set_bounds(self.rect_for(t.id)); } }
        if let Some(o) = &self.overlay {
            let r = match self.overlay_role.as_str() {
                "find" => rect(w - 400.0, self.top_h() + 8.0, 390.0, 64.0),
                "suggest" => rect(self.side_w() + 92.0, 46.0, (w - self.side_w() - 220.0).clamp(280.0, 620.0), 250.0),
                "preview" => rect(if self.settings.tabs_on_top { self.preview_xy.0 } else { self.side_w() + 6.0 }, if self.settings.tabs_on_top { self.top_h() + 4.0 } else { (self.top_h() + self.preview_xy.1 - 40.0).max(self.top_h()) }, 250.0, 200.0),
                "tools" => rect(w - 370.0, self.top_h(), 360.0, h - self.top_h()),
                _ => rect(0.0, 0.0, w, h),
            };
            let _ = o.set_bounds(r);
        }
    }

    fn make_view(&self, id: u32, url: &str, private: bool, flags: &Arc<net::TabFlags>, token: &str) -> WebView {
        let (p1, p2, p3, p4, p5) = (self.px.clone(), self.px.clone(), self.px.clone(), self.px.clone(), self.px.clone());
        let rules = self.rules.clone();
        let port = self.net.listener(flags);
        let script = format!("{}\n{}\n{}", shield::script(&self.settings), keys::script(&self.settings, token), ext::tab_script(&self.enabled_exts(), token));
        let b = wvb!(&self.settings)
            .with_custom_protocol("lfext".into(), ext::protocol_handler())
            .with_initialization_script(&script)
            .with_incognito(private)
            .with_devtools(true)
            .with_proxy_config(ProxyConfig::Socks5(ProxyEndpoint { host: "127.0.0.1".into(), port: port.to_string() }))
            .with_document_title_changed_handler(move |t| { let _ = p1.send_event(Ev::Title(id, t)); })
            // Seiten-IPC ist unvertrauenswürdig: nur Nachrichten mit dem geheimen Tab-Token zählen (siehe Ev::PageIpc)
            .with_ipc_handler(move |r| { let _ = p3.send_event(Ev::PageIpc(id, r.body().clone())); })
            .with_download_started_handler(move |src: String, dest: &mut std::path::PathBuf| {
                let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(src);
                *dest = data::unique(dirs::download_dir().unwrap_or_default().join(&name));
                let _ = p4.send_event(Ev::DlStart(name, dest.to_string_lossy().into_owned()));
                true
            })
            .with_download_completed_handler(move |_src: String, path: Option<std::path::PathBuf>, ok: bool| {
                let _ = p5.send_event(Ev::DlDone(path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(), ok));
            })
            .with_new_window_req_handler({ let pn = self.px.clone(); move |u| { let _ = pn.send_event(Ev::NewWindow(id, u)); false } })
            .with_navigation_handler(move |u| {
                if let Some((_, frag)) = u.split_once("#lf-allow-") { let _ = p2.send_event(Ev::AllowHttp(id, frag.to_string())); return false; }
                {
                    let r = rules.read().unwrap();
                    if let Some(n) = shield::rewrite(&u, &r) { let _ = p2.send_event(Ev::Navigate(id, n)); return false; }
                    if shield::needs_probe(&u, &r) { let _ = p2.send_event(Ev::HttpsProbe(id, u)); return false; }
                }
                let _ = p2.send_event(Ev::Url(id, u));
                true
            });
        let b = if url == START { b.with_html(shield::start_html(&self.settings)) } else { b.with_url(url) };
        let v = b.build_as_child(&self.window).expect("webview");
        let _ = v.set_bounds(self.page_rect());
        v
    }

    fn ui_view(&self, role: &str) -> WebView {
        let p = self.px.clone();
        wvb!(&self.settings)
            .with_html(UI)
            .with_initialization_script(&format!("window.ROLE='{role}'"))
            .with_ipc_handler(move |r| { let _ = p.send_event(Ev::Ipc(r.body().clone())); })
            .build_as_child(&self.window).expect("ui")
    }

    fn open_overlay(&mut self, role: &str) {
        self.overlay = Some(self.ui_view(role));
        self.overlay_role = role.to_string();
        self.relayout();
        if role != "suggest" && role != "preview" { if let Some(o) = &self.overlay { let _ = o.focus(); } }
    }

    fn active_view(&self) -> Option<&WebView> { self.tabs.iter().find(|t| t.id == self.active).and_then(|t| t.view.as_ref()) }

    fn active_host(&self) -> String {
        self.tabs.iter().find(|t| t.id == self.active).map(|t| host_of(&t.url)).unwrap_or_default()
    }

    fn new_tab(&mut self, url: Option<String>, private: bool) {
        let url = url.unwrap_or_else(|| START.to_string());
        let id = self.next_id; self.next_id += 1;
        self.tabs.push(Tab { id, url, title: "Neuer Tab".into(), view: None, last_active: Instant::now(), private, flags: net::TabFlags::new(), token: String::new(), group: 0, last_gesture: Instant::now() });
        self.active = id;
        self.ensure_active_view();
    }

    fn switch(&mut self, id: u32) {
        if id == self.active || !self.tabs.iter().any(|t| t.id == id) { return; }
        self.capture_active();
        if self.split == Some(id) { self.split = Some(self.active); }
        self.active = id;
        self.close_popup();
        self.ensure_active_view();
    }

    fn ensure_active_view(&mut self) {
        let want: Vec<u32> = std::iter::once(self.active).chain(self.split).collect();
        for id in want {
            let Some(i) = self.tabs.iter().position(|t| t.id == id) else { continue };
            if self.tabs[i].view.is_none() {
                let (url, private, flags) = (self.tabs[i].url.clone(), self.tabs[i].private, self.tabs[i].flags.clone());
                let token = token();
                let v = self.make_view(id, &url, private, &flags, &token);
                self.tabs[i].view = Some(v);
                self.tabs[i].token = token;
            }
        }
        for t in &mut self.tabs {
            let vis = t.id == self.active || Some(t.id) == self.split;
            if vis { t.last_active = Instant::now(); }
            if let Some(v) = &t.view { let _ = v.set_visible(vis); }
        }
        for t in &self.tabs { if let Some(v) = &t.view { if t.id == self.active || Some(t.id) == self.split { let _ = v.set_bounds(self.rect_for(t.id)); } } }
        self.sync();
        self.apply_zoom();
    }

    fn close_tab(&mut self, id: u32) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return };
        if !self.tabs[i].private && self.tabs[i].url.starts_with("http") {
            self.closed.push(self.tabs[i].url.clone());
            if self.closed.len() > 20 { self.closed.remove(0); }
        }
        self.tabs.remove(i);
        if self.split == Some(id) { self.split = None; }
        if self.tabs.is_empty() { self.new_tab(None, false); }
        else if self.active == id {
            self.active = self.split.take().unwrap_or(self.tabs[i.min(self.tabs.len() - 1)].id);
            self.ensure_active_view();
        } else { self.ensure_active_view(); }
        self.relayout();
    }

    fn navigate(&mut self, url: String) {
        let Some(i) = self.tabs.iter().position(|t| t.id == self.active) else { return };
        self.tabs[i].url = url.clone();
        self.ensure_active_view();
        if let Some(v) = &self.tabs[i].view { let _ = v.load_url(&url); }
    }

    fn rebuild_views(&mut self) { for t in &mut self.tabs { t.view = None; } self.ensure_active_view(); }

    fn rebuild_host(&mut self, host: &str) {
        for t in &mut self.tabs { if host_of(&t.url) == host { t.view = None; } }
        self.ensure_active_view();
    }

    /// Regeln aus den Settings übernehmen und Proxy-Flags (Adblock pro Seite) aktualisieren.
    fn sync(&self) {
        let r = Rules::from(&self.settings);
        for t in &self.tabs { t.flags.adblock.store(r.adblock(&host_of(&t.url)), Ordering::Relaxed); t.flags.isolate.store(self.settings.tor_per_tab, Ordering::Relaxed); }
        let mut r = r;
        r.http_ok = self.http_ok.clone();
        self.net.s.stats_on.store(self.settings.stats_enabled, Ordering::Relaxed);
        *self.rules.write().unwrap() = r;
    }

    fn apply_zoom(&self) {
        let z = self.rules.read().unwrap().zoom(&self.active_host()).unwrap_or(1.0);
        if let Some(v) = self.tabs.iter().find(|t| t.id == self.active).and_then(|t| t.view.as_ref()) { let _ = v.zoom(z); }
    }

    fn site_set(&mut self, host: String, k: &str, val: &serde_json::Value) {
        if host.is_empty() { return; }
        {
            let r = self.settings.site_rules.entry(host.clone()).or_default();
            match k {
                "adblock" => r.adblock = val.as_bool(), "fingerprint" => r.fingerprint = val.as_bool(),
                "webrtc" => r.webrtc = val.as_bool(), "https_only" => r.https_only = val.as_bool(),
                "strip_tracking" => r.strip_tracking = val.as_bool(), "dark" => r.dark = val.as_bool(),
                "zoom" => r.zoom = val.as_f64(), "cookies" => r.cookies = val.as_bool(), "hints" => r.hints = val.as_bool(), "block_cam" => r.block_cam = val.as_bool(),
                "block_mic" => r.block_mic = val.as_bool(), "block_geo" => r.block_geo = val.as_bool(), _ => {}
            }
        }
        self.settings.save();
        self.sync();
        match k {
            "fingerprint" | "webrtc" | "dark" | "cookies" | "hints" | "block_cam" | "block_mic" | "block_geo" => self.rebuild_host(&host),
            "zoom" => self.apply_zoom(),
            _ => self.eval_active("location.reload()"),
        }
    }

    fn eval_active(&self, js: &str) {
        if let Some(v) = self.tabs.iter().find(|t| t.id == self.active).and_then(|t| t.view.as_ref()) { let _ = v.evaluate_script(js); }
    }

    // ---- Blocklisten ----
    fn bl_add(&mut self, source: &str, loc: String) {
        let id = format!("bl{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let name = loc.trim_end_matches(|c| c == '/' || c == '\\').rsplit(|c| c == '/' || c == '\\').next().unwrap_or(&loc).to_string();
        self.settings.blocklists.push(settings::BlockList { id: id.clone(), name, source: source.into(), location: loc, enabled: true, last_updated: 0, count: 0 });
        self.settings.save();
        self.bl_refresh(&id);
    }
    fn bl_refresh(&mut self, id: &str) {
        let Some(l) = self.settings.blocklists.iter().find(|l| l.id == id).cloned() else { return };
        if !self.refreshing.insert(l.id.clone()) { return; }
        let p = self.px.clone();
        std::thread::spawn(move || { let r = blocklist::fetch(&l); let _ = p.send_event(Ev::BlRefreshed(l.id, r)); });
    }
    fn bl_refresh_all(&mut self) {
        let ids: Vec<String> = self.settings.blocklists.iter().filter(|l| l.enabled).map(|l| l.id.clone()).collect();
        for id in ids { self.bl_refresh(&id); }
    }
    fn bl_rebuild(&self) {
        let ids: Vec<String> = self.settings.blocklists.iter().filter(|l| l.enabled).map(|l| l.id.clone()).collect();
        let (net, p) = (self.net.clone(), self.px.clone());
        std::thread::spawn(move || { let set = blocklist::load_enabled(&ids); let n = set.len(); net.set_blocklist(set); let _ = p.send_event(Ev::BlLoaded(n)); });
    }
    fn bl_done(&mut self, id: String, r: Result<usize, String>) {
        self.refreshing.remove(&id);
        if let Ok(n) = r {
            if let Some(l) = self.settings.blocklists.iter_mut().find(|l| l.id == id) { l.count = n; l.last_updated = now(); }
            self.settings.save();
            self.bl_rebuild();
        }
    }
    fn bl_tick(&mut self) {
        let secs = match self.settings.bl_update.as_str() { "interval" => self.settings.bl_interval_hours.max(1) * 3600, "monthly" => 30 * 86400, _ => return };
        let due: Vec<String> = self.settings.blocklists.iter().filter(|l| l.enabled && now().saturating_sub(l.last_updated) >= secs).map(|l| l.id.clone()).collect();
        for id in due { self.bl_refresh(&id); }
    }

    fn apply_settings(&mut self, mut s: Settings, done: bool) {
        s.blocklists = self.settings.blocklists.clone();
        s.site_rules = self.settings.site_rules.clone();
        s.quick_dials = self.settings.quick_dials.clone();
        s.keymap = self.settings.keymap.clone();
        s.ext_disabled = self.settings.ext_disabled.clone();
        s.sync_folder = self.settings.sync_folder.clone();
        s.sync_ts = self.settings.sync_ts;
        s.workspaces = self.settings.workspaces.clone();
        s.active_workspace = self.settings.active_workspace.clone();
        let o = &self.settings;
        let toggled = s.vpn_enabled != o.vpn_enabled || s.kill_switch != o.kill_switch;
        let net_changed = toggled || s.vpn_exit_country != o.vpn_exit_country;
        let script_changed = s.fingerprint_protect != o.fingerprint_protect || s.webrtc_protect != o.webrtc_protect;
        self.settings = s;
        if done { self.settings.onboarded = true; self.overlay = None; }
        self.settings.save();
        self.net.set_dns(&self.settings.dns_mode, &self.settings.dns_custom);
        self.net.set_bridges(&self.settings.bridges);
        if net_changed { self.net.set_vpn(self.settings.vpn_enabled, self.settings.kill_switch, &self.settings.vpn_exit_country); }
        self.sync();
        if toggled || script_changed { self.rebuild_views(); }
        self.relayout();
    }

    fn record_history(&mut self, url: String, title: String) {
        if let Some(l) = self.history.last_mut() { if l.url == url { l.title = title; self.history_dirty = true; return; } }
        self.history.push(data::HistEntry { url, title, t: now() });
        if self.history.len() > 5000 { self.history.drain(..500); }
        self.history_dirty = true;
    }

    fn clear_data(&mut self) {
        if let Some(v) = self.tabs.iter().find_map(|t| t.view.as_ref()) { let _ = v.clear_all_browsing_data(); }
        self.history.clear(); self.downloads.clear(); self.closed.clear();
        data::save("history.json", &self.history);
    }

    fn key(&mut self, c: &str) {
        let c = if c == "Ctrl+Shift++" { "Ctrl+=" } else { c };
        if let Some((id, _)) = keys::effective(&self.settings.keymap).into_iter().find(|(_, k)| k == c) { self.action(&id); }
    }

    fn action(&mut self, id: &str) {
        match id {
            "palette" | "find" | "history" | "bookmarks" | "downloads" | "settings" | "shield" => self.open_overlay(id),
            "new_tab" => self.new_tab(None, false),
            "new_private" => self.new_tab(None, true),
            "close_tab" => { let a = self.active; self.close_tab(a); }
            "reopen_tab" => { if let Some(u) = self.closed.pop() { self.new_tab(Some(u), false); } }
            "next_tab" | "prev_tab" => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == self.active) {
                    let n = self.tabs.len();
                    let j = if id == "next_tab" { (i + 1) % n } else { (i + n - 1) % n };
                    let t = self.tabs[j].id; self.switch(t);
                }
            }
            "focus_url" => { let _ = self.bar.focus(); let _ = self.bar.evaluate_script("window.focusUrl&&focusUrl()"); }
            "reload" => self.eval_active("location.reload()"),
            "back" => self.eval_active("history.back()"),
            "forward" => self.eval_active("history.forward()"),
            "zoom_in" | "zoom_out" | "zoom_reset" => {
                let h = self.active_host();
                if !h.is_empty() {
                    let cur = self.rules.read().unwrap().zoom(&h).unwrap_or(1.0);
                    let z = match id { "zoom_in" => (cur + 0.1).min(3.0), "zoom_out" => (cur - 0.1).max(0.3), _ => 1.0 };
                    self.site_set(h, "zoom", &serde_json::json!(z));
                }
            }
            "bookmark" => {
                let cur = self.tabs.iter().find(|t| t.id == self.active).map(|t| (t.url.clone(), t.title.clone()));
                if let Some((url, title)) = cur {
                    if url.starts_with("http") {
                        if self.bookmarks.iter().any(|b| b.url == url) { self.bookmarks.retain(|b| b.url != url); }
                        else { self.bookmarks.push(data::Bookmark { title, url }); }
                        data::save("bookmarks.json", &self.bookmarks);
                    }
                }
            }
            "dark_toggle" => {
                let h = self.active_host();
                let cur = self.rules.read().unwrap().dark(&h);
                self.site_set(h, "dark", &serde_json::Value::Bool(!cur));
            }
            "dial_add" => {
                if let Some(tab) = self.tabs.iter().find(|x| x.id == self.active) {
                    if tab.url.starts_with("http") { self.settings.quick_dials.push(settings::Dial { name: host_of(&tab.url), url: tab.url.clone() }); self.settings.save(); }
                }
            }
            "fullscreen" => {
                let f = self.window.fullscreen().is_some();
                self.window.set_fullscreen(if f { None } else { Some(Fullscreen::Borderless(None)) });
            }
            "devtools" => { if let Some(v) = self.active_view() { v.open_devtools(); } }
            "print" => { if let Some(v) = self.active_view() { let _ = v.print(); } }
            "reader" => self.eval_active(shield::READER_JS),
            "clear_data" => self.clear_data(),
            "vpn_toggle" => { let mut s = self.settings.clone(); s.vpn_enabled = !s.vpn_enabled; self.apply_settings(s, false); }
            "tor_new" => self.net.new_identity(),
            "split_toggle" => self.split_toggle(),
            "split_swap" => self.split_swap(),
            "tab_group" => self.tab_group(),
            "vault" => self.open_overlay("vault"),
            "pw_fill" => self.pw_fill(),
            "pw_save" => self.pw_save(),
            "sync_now" => self.sync_now(),
            "update_install" => self.update_install(),
            "panic" => self.panic_exit(),
            "gaming_toggle" => self.gaming_toggle(),
            "workspaces" | "privacy" | "import" => self.open_overlay(id),
            "share_phone" => self.share_phone(),
            "totp_fill" => self.totp_fill(),
            "tts_read" => self.tts_read(),
            "tts_stop" => self.tts_stop(),
            "pip" | "speed_up" | "speed_down" | "speed_reset" => self.eval_active(&format!("window.__lfMedia&&__lfMedia('{id}')")),
            "focus_mode" => self.focus_toggle(),
            "sleep_others" => self.sleep_others(),
            "tools" | "leak" | "profiles" => self.open_overlay(id),
            "screenshot" => {
                #[cfg(windows)] { let _ = std::process::Command::new("explorer").arg("ms-screenclip:").spawn(); }
                #[cfg(not(windows))] { self.notice = "Screenshot nur unter Windows (Snip-Tool)".into(); }
            }
            _ => {}
        }
    }

    fn push(&self) {
        let tabs: Vec<_> = self.tabs.iter().map(|t| serde_json::json!({
            "id": t.id, "title": t.title, "url": t.url, "paused": t.view.is_none(), "private": t.private, "group": t.group })).collect();
        let host = self.active_host();
        let cur_url = self.tabs.iter().find(|t| t.id == self.active).map(|t| t.url.clone());
        let n = if self.overlay_role == "history" { 300 } else { 30 };
        let hist: Vec<_> = self.history.iter().rev().take(n).collect();
        let r = self.rules.read().unwrap();
        let site = serde_json::json!({ "host": host, "adblock": r.adblock(&host), "fingerprint": r.fingerprint(&host), "webrtc": r.webrtc(&host),
            "https_only": r.https_only(&host), "strip_tracking": r.strip(&host), "dark": r.dark(&host), "zoom": r.zoom(&host).unwrap_or(1.0),
            "bookmarked": self.bookmarks.iter().any(|b| Some(&b.url) == cur_url.as_ref()),
            "cookies": r.cookies(&host), "hints": r.hints(&host), "block_cam": r.get(&host, "block_cam"), "block_mic": r.get(&host, "block_mic"), "block_geo": r.get(&host, "block_geo") });
        let st = serde_json::json!({ "tabs": tabs, "active": self.active, "settings": self.settings, "vpn": self.vpn_status,
            "blocked": self.net.s.blocked.load(Ordering::Relaxed), "bl_total": self.bl_total, "site": site,
            "actions": keys::state(&self.settings), "bookmarks": self.bookmarks, "history": hist, "downloads": self.downloads,
            "x": self.extra_all(), "split": self.split, "vault": self.vault_json(), "notice": self.notice, "update": self.update_label(),
            "exts": self.exts.iter().map(|e| json!({"id": e.id, "name": e.name(), "popup": e.popup().is_some(), "on": !self.settings.ext_disabled.contains(&e.id)})).collect::<Vec<_>>() });
        let js = format!("window.render&&render({st})");
        for v in [Some(&self.bar), Some(&self.side), self.overlay.as_ref()].into_iter().flatten() { let _ = v.evaluate_script(&js); }
    }

    fn ipc(&mut self, msg: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(msg) else { return };
        let t = v["t"].as_str().unwrap_or("");
        let id = v["id"].as_u64().unwrap_or(0) as u32;
        match t {
            "go" => { let u = resolve_input(v["v"].as_str().unwrap_or(""), &self.settings); self.navigate(u); }
            "search" => { let u = self.settings.search_url(v["v"].as_str().unwrap_or("")); self.navigate(u); }
            "newtab" => self.new_tab(None, v["private"].as_bool().unwrap_or(false)),
            "switch" => self.switch(id),
            "close" => self.close_tab(id),
            "settings" | "onboard_done" => {
                if let Ok(s) = serde_json::from_value::<Settings>(v["s"].clone()) { self.apply_settings(s, t == "onboard_done"); }
            }
            "vpn_toggle" => { let mut s = self.settings.clone(); s.vpn_enabled = !s.vpn_enabled; self.apply_settings(s, false); }
            "tor_new" => self.net.new_identity(),
            "reader" => self.eval_active(shield::READER_JS),
            "site" => { let h = v["host"].as_str().unwrap_or("").to_string(); self.site_set(h, v["k"].as_str().unwrap_or(""), &v["v"]); }
            "site_toggle" => {
                let (h, k) = (self.active_host(), v["k"].as_str().unwrap_or("").to_string());
                let cur = self.rules.read().unwrap().get(&h, &k);
                self.site_set(h, &k, &serde_json::Value::Bool(!cur));
            }
            "dial_add" => {
                if let Some(tab) = self.tabs.iter().find(|x| x.id == self.active) {
                    if tab.url.starts_with("http") { self.settings.quick_dials.push(settings::Dial { name: host_of(&tab.url), url: tab.url.clone() }); self.settings.save(); }
                }
            }
            "bl_add_url" => { let u = v["v"].as_str().unwrap_or("").trim().to_string(); if u.starts_with("http") { self.bl_add("url", u); } }
            "bl_pick" => {
                let dir = v["dir"].as_bool().unwrap_or(false);
                let p = self.px.clone();
                std::thread::spawn(move || {
                    let d = rfd::FileDialog::new();
                    let r = if dir { d.pick_folder() } else { d.add_filter("Blocklist", &["txt", "zip", "hosts", "list"]).pick_file() };
                    if let Some(r) = r { let _ = p.send_event(Ev::BlPath(r)); }
                });
            }
            "bl_refresh" => self.bl_refresh(v["id"].as_str().unwrap_or("")),
            "bl_toggle" => {
                let id = v["id"].as_str().unwrap_or("");
                if let Some(l) = self.settings.blocklists.iter_mut().find(|l| l.id == id) { l.enabled = !l.enabled; }
                self.settings.save(); self.bl_rebuild();
            }
            "bl_remove" => {
                let id = v["id"].as_str().unwrap_or("").to_string();
                self.settings.blocklists.retain(|l| l.id != id);
                blocklist::remove_cache(&id); self.settings.save(); self.bl_rebuild();
            }
            "action" => self.action(v["id"].as_str().unwrap_or("")),
            "key" => self.key(v["c"].as_str().unwrap_or("")),
            "keymap" => {
                let (a, c) = (v["a"].as_str().unwrap_or("").to_string(), v["c"].as_str().unwrap_or("").to_string());
                for (id, cur) in keys::effective(&self.settings.keymap) { if id != a && cur == c && !c.is_empty() { self.settings.keymap.insert(id, String::new()); } }
                self.settings.keymap.insert(a, c);
                self.settings.save(); self.rebuild_views();
            }
            "keymap_reset" => { self.settings.keymap.clear(); self.settings.save(); self.rebuild_views(); }
            "open_url" => { let u = v["url"].as_str().unwrap_or("").to_string(); if u.starts_with("http") { self.navigate(u); } }
            "bm_remove" => { let u = v["url"].as_str().unwrap_or(""); self.bookmarks.retain(|b| b.url != u); data::save("bookmarks.json", &self.bookmarks); }
            "hist_clear" => { self.history.clear(); self.history_dirty = true; }
            "dl_show" => { let p = v["path"].as_str().unwrap_or(""); if self.downloads.iter().any(|d| d.path == p) { show_in_folder(p); } }
            "find" => {
                let q = serde_json::to_string(v["q"].as_str().unwrap_or("")).unwrap_or_default();
                self.eval_active(&format!("window.find({q},false,{},true)", v["back"].as_bool().unwrap_or(false)));
            }
            "open_settings" => self.open_overlay("settings"),
            "open_shield" => self.open_overlay("shield"),
            "open_palette" => self.open_overlay("palette"),
            "close_overlay" => { self.overlay = None; self.close_popup(); }
            _ => { self.ipc_more(t, &v); self.more2(t, &v); self.more3(t, &v); }
        }
        self.push();
    }

    fn event(&mut self, ev: Ev) {
        match ev {
            Ev::Ipc(m) => { self.ipc(&m); return; }
            Ev::PageIpc(id, m) => {
                if m.split_once('|').map_or(false, |(tok, c)| c == "g" && self.tabs.iter().any(|t| t.id == id && t.token == tok)) {
                    if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) { t.last_gesture = Instant::now(); }
                    return;
                }
                let rest = m.split_once('|').filter(|(tok, _)| self.tabs.iter().any(|t| t.id == id && &t.token == tok)).map(|(_, c)| c.to_string());
                match rest { Some(c) if c.starts_with('{') => self.ext_rpc(features::Origin::Tab(id), &c), Some(c) if c.starts_with("act:") => self.gesture_action(&c[4..]), Some(c) => self.key(&c), None => return }
            }
            Ev::DlStart(name, path) => self.downloads.push(data::Download { name, path, done: false, ok: false, vt: String::new() }),
            Ev::DlDone(path, ok) => {
                if let Some(d) = self.downloads.iter_mut().rev().find(|d| d.path == path || (path.is_empty() && !d.done)) { d.done = true; d.ok = ok; }
                if ok { let p2 = path.clone(); self.vt_check(p2); }
            }
            Ev::Title(id, t) => {
                let mut rec = None;
                if let Some(x) = self.tabs.iter_mut().find(|x| x.id == id) {
                    x.title = t.clone();
                    if !x.private && x.url.starts_with("http") { rec = Some((x.url.clone(), t)); }
                }
                if let Some((u, t)) = rec { self.record_history(u, t); }
            }
            Ev::Url(id, u) => {
                if !u.starts_with("http") { return; }
                if !self.bl_open_done && self.settings.bl_update == "on_open" {
                    self.bl_open_done = true;
                    self.bl_refresh_all(); // Hintergrund: Seite lädt parallel mit der gecachten Liste
                }
                if let Some(x) = self.tabs.iter_mut().find(|x| x.id == id) { x.url = u; x.flags.blocked.lock().unwrap().clear(); }
                self.sync();
                if id == self.active { self.apply_zoom(); }
            }
            Ev::Navigate(id, u) => {
                if let Some(x) = self.tabs.iter_mut().find(|x| x.id == id) {
                    x.url = u.clone();
                    if let Some(v) = &x.view { let _ = v.load_url(&u); }
                }
            }
            Ev::Vpn(s) => self.vpn_status = s,
            Ev::BlPath(p) => self.bl_add("path", p.to_string_lossy().into_owned()),
            Ev::BlLoaded(n) => self.bl_total = n,
            Ev::ExtIpc(e, k, m) => self.ext_ipc(e, k, m),
            Ev::ExtInstalled(r) => match r {
                Ok(n) => { self.notice = format!("Installiert: {n}"); self.reload_exts(); }
                Err(e) => self.notice = format!("Fehler: {e}"),
            },
            Ev::ExtPick(p) => { let tx = self.px.clone(); std::thread::spawn(move || { let _ = tx.send_event(Ev::ExtInstalled(ext::install_path(&p))); }); }
            Ev::SyncFolder(p) => { self.settings.sync_folder = p.to_string_lossy().into_owned(); self.settings.save(); self.sync_now(); }
            Ev::UpdateAvail(Ok(Some(r))) => { self.notice = format!("Update {} verfügbar", r.tag); self.release = Some(r); }
            Ev::UpdateAvail(Ok(None)) => { self.release = None; }
            Ev::UpdateAvail(Err(e)) => self.notice = e,
            Ev::UpdateProgress(p) => self.notice = format!("Lade Update… {:.0}%", p * 100.0),
            Ev::UpdateReady(r) => self.update_ready(r),
            Ev::Updated(Ok(())) => self.notice = if self.installed_newer().is_some() { "Update installiert – klicke auf „Neu starten“".into() } else { "Liquid Flow ist aktuell".into() },
            Ev::Updated(Err(e)) => self.notice = e,
            Ev::PwCaptured(h, r) => self.pw_captured(h, r),
            Ev::OpenUrl(u) => { if u.starts_with("http") { self.new_tab(Some(u), false); } self.window.set_minimized(false); self.window.set_focus(); }
            Ev::NewWindow(id, u) => self.new_window(id, u),
            Ev::Thumb(id, png) => self.thumb_in(id, png),
            Ev::Leak(v) => self.leak = Some(v),
            Ev::BgPick(p) => self.bg_set(p),
            Ev::HttpsProbe(id, u) => self.https_probe(id, u),
            Ev::ProbeDone(id, u, ok) => self.probe_done(id, u, ok),
            Ev::AllowHttp(id, n) => self.allow_http(id, n),
            Ev::Pwned(r) => self.pwned_done(r),
            Ev::Vt(path, text) => { if let Some(d) = self.downloads.iter_mut().rev().find(|d| d.path == path) { d.vt = text; } }
            Ev::ImportData(b, h, e) => self.import_done(b, h, e),
            Ev::TtsText(r) => self.tts_speak(r),
            Ev::Notice(n) => self.notice = n,
            Ev::BlRefreshed(id, r) => self.bl_done(id, r),
            Ev::Tick => {
                if !self.gaming { self.bl_tick(); }
                self.tick_n += 1;
                if self.gaming { self.sleep_others(); } else { self.measure_mem(); }
                if !self.gaming && self.tick_n % 4 == 0 { self.capture_active(); }
                self.save_stats();
                self.save_session();
                if self.ext_dirty { self.save_ext_store(); self.ext_dirty = false; }
                if self.vault.is_some() && self.vault_last.elapsed() > Duration::from_secs(900) { self.vault = None; }
                if self.tick_n % 20 == 0 && !self.gaming { self.sync_now(); }
                if self.history_dirty { data::save("history.json", &self.history); self.history_dirty = false; }
                if self.settings.suspend_background_tabs {
                    let limit = Duration::from_secs(self.settings.suspend_secs);
                    for t in &mut self.tabs {
                        if t.id != self.active && Some(t.id) != self.split && t.view.is_some() && t.last_active.elapsed() > limit { t.view = None; }
                    }
                }
            }
        }
        self.push();
    }
}

fn main() {
    // --profile NAME, URL als Argument
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--profile") { if let Some(n) = args.get(i + 1) { data::set_profile(n); } }
    std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", data::home().join("webview"));
    // Einzelinstanz pro Profil: zweite Instanz reicht die URL weiter und beendet sich
    let port = 47000 + (data::profile().bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 900) as u16;
    let single = std::net::TcpListener::bind(("127.0.0.1", port));
    let single = match single {
        Ok(l) => l,
        Err(_) => {
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                use std::io::Write;
                let _ = writeln!(s, "LFOPEN {}", args.iter().skip(1).find(|a| a.starts_with("http")).cloned().unwrap_or_default());
            }
            return;
        }
    };
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let px = event_loop.create_proxy();
    let opener = px.clone();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Read};
        for s in single.incoming().flatten() {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            if BufReader::new(s.take(2048)).read_line(&mut line).is_ok() { if let Some(u) = line.strip_prefix("LFOPEN ") { let _ = opener.send_event(Ev::OpenUrl(u.trim().to_string())); } }
        }
    });
    let ticker = px.clone();
    std::thread::spawn(move || loop { std::thread::sleep(Duration::from_secs(15)); let _ = ticker.send_event(Ev::Tick); });

    let window = WindowBuilder::new().with_title("Liquid Flow").with_inner_size(LogicalSize::new(1280.0, 800.0))
        .build(&event_loop).unwrap();
    let settings = Settings::load();

    let mk = |role: &str| {
        let p = px.clone();
        wvb!(&settings).with_html(UI)
            .with_initialization_script(&format!("window.ROLE='{role}'"))
            .with_ipc_handler(move |r| { let _ = p.send_event(Ev::Ipc(r.body().clone())); })
            .build_as_child(&window).unwrap()
    };
    let (bar, side) = (mk("bar"), mk("side"));
    let vp = px.clone();
    let net = net::Net::start(move |s| { let _ = vp.send_event(Ev::Vpn(s.into())); });
    let rules = Arc::new(RwLock::new(Rules::from(&settings)));
    let mut app = App { window, px: px.clone(), settings, rules, tabs: vec![], active: 0, next_id: 1, bar, side, overlay: None,
        vpn_status: "off".into(), net, refreshing: HashSet::new(), bl_total: 0, bl_open_done: false,
        split: None, exts: ext::load_all(), ext_views: vec![], pending: HashMap::new(), ext_seq: 0, ext_store: data::load("ext_storage.json"), ext_dirty: false,
        vault: None, vault_err: String::new(), vault_last: Instant::now(), notice: String::new(), release: None, quit: false, tick_n: 0, focus: false, mem_mb: 0, suggestions: vec![], leak: None, thumbs: HashMap::new(), preview: None, preview_xy: (0.0, 0.0), gaming: false, tts: None, qr: None, http_ok: HashSet::new(), http_nonce: HashMap::new(), pwned: HashMap::new(),
        overlay_role: String::new(), bookmarks: data::load("bookmarks.json"), history: data::load("history.json"), history_dirty: false, closed: vec![], downloads: vec![] };

    app.net.set_dns(&app.settings.dns_mode, &app.settings.dns_custom);
    app.net.set_bridges(&app.settings.bridges);
    if app.settings.vpn_enabled { app.net.set_vpn(true, app.settings.kill_switch, &app.settings.vpn_exit_country); }
    app.net.load_stats(data::load("stats.json"));
    app.start_all_bg();
    app.check_update();
    app.bl_rebuild(); // gecachte Listen im Hintergrund laden, blockiert den Start nicht
    if app.settings.bl_update == "startup" { app.bl_refresh_all(); }
    let cli = std::env::args().skip(1).find(|a| a.starts_with("http"));
    if !app.restore_session() { app.new_tab(cli.clone().or(None), false); } else if let Some(u) = cli { app.new_tab(Some(u), false); }
    if !app.settings.onboarded { app.open_overlay("onboard"); }
    app.relayout();

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ev) => { app.event(ev); if app.quit { *flow = ControlFlow::Exit; } }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.relayout(),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.save_session(); app.sync_now(); app.save_ext_store();
                if app.settings.clear_on_exit { app.clear_data(); } else { data::save("history.json", &app.history); }
                *flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}
