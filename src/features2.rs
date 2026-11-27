//! Weitere Funktionen: Sitzung, Panic, Fokus, Speicher, Tools, Leak-Test, Vorschläge, Profile, Gesten, Popups
use crate::shield::host_of;
use crate::{data, net, vault, App, Ev, Tab, START};
use serde_json::{json, Value};
use std::time::Instant;

impl App {
    // ---------- Sitzung ----------
    pub(crate) fn save_session(&self) {
        let tabs: Vec<data::SessTab> = self.tabs.iter().filter(|t| !t.private && t.url.starts_with(|c| c == 'h' || c == 'l'))
            .map(|t| data::SessTab { url: t.url.clone(), title: t.title.clone(), group: t.group }).collect();
        let active = self.tabs.iter().filter(|t| !t.private && t.url.starts_with(|c| c == 'h' || c == 'l')).position(|t| t.id == self.active).unwrap_or(0);
        data::save("session.json", &data::Session { tabs, active });
    }
    pub(crate) fn restore_session(&mut self) -> bool {
        if !self.settings.restore_session { return false; }
        let s: data::Session = data::load("session.json");
        if s.tabs.is_empty() { return false; }
        for st in &s.tabs {
            let id = self.next_id; self.next_id += 1;
            self.tabs.push(Tab { id, url: st.url.clone(), title: st.title.clone(), view: None, last_active: Instant::now(), private: false,
                flags: net::TabFlags::new(), token: String::new(), group: st.group, last_gesture: Instant::now() });
        }
        self.active = self.tabs[s.active.min(self.tabs.len() - 1)].id;
        self.ensure_active_view();
        true
    }

    // ---------- Panic: alles löschen, alles schließen, beenden ----------
    pub(crate) fn panic_exit(&mut self) {
        self.clear_data();
        self.tabs.clear(); self.split = None; self.overlay = None; self.ext_views.clear();
        self.vault = None;
        let _ = std::fs::remove_file(data::home().join("session.json"));
        self.quit = true;
    }

    // ---------- Fokus, Schlafen, Speicher ----------
    pub(crate) fn focus_toggle(&mut self) {
        self.focus = !self.focus;
        let _ = self.bar.set_visible(!self.focus);
        let _ = self.side.set_visible(!self.focus);
        self.relayout();
        self.ensure_active_view();
    }
    pub(crate) fn sleep_others(&mut self) {
        let (a, s) = (self.active, self.split);
        for t in &mut self.tabs { if t.id != a && Some(t.id) != s { t.view = None; } }
    }
    pub(crate) fn measure_mem(&mut self) {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let me = Pid::from_u32(std::process::id());
        let mut family = std::collections::HashSet::from([me]);
        for _ in 0..4 {
            for (pid, p) in sys.processes() { if p.parent().map_or(false, |pp| family.contains(&pp)) { family.insert(*pid); } }
        }
        self.mem_mb = family.iter().filter_map(|p| sys.process(*p)).map(|p| p.memory()).sum::<u64>() / (1024 * 1024);
    }

    // ---------- Popups & Gesten ----------
    pub(crate) fn new_window(&mut self, id: u32, url: String) {
        let allowed = self.tabs.iter().find(|t| t.id == id).map_or(false, |t| t.last_gesture.elapsed().as_millis() < 1500);
        if !url.starts_with("http") { return; }
        if allowed || !self.settings.block_popups { self.new_tab(Some(url), false); }
        else { self.notice = format!("Popup blockiert: {}", host_of(&url)); }
    }
    pub(crate) fn gesture_action(&mut self, a: &str) {
        if matches!(a, "back" | "forward" | "reload" | "new_tab" | "close_tab" | "reopen_tab" | "next_tab" | "prev_tab") { self.action(a); }
    }

    // ---------- Vorschläge ----------
    pub(crate) fn suggest(&mut self, q: &str) {
        let q = q.trim().to_lowercase();
        if q.len() < 2 { self.suggest_close(); return; }
        let mut out: Vec<Value> = vec![];
        let mut seen = std::collections::HashSet::new();
        let bm = self.bookmarks.iter().map(|b| (&b.title, &b.url, "★"));
        let hist = self.history.iter().rev().take(3000).map(|h| (&h.title, &h.url, "↺"));
        for (t, u, k) in bm.chain(hist) {
            if out.len() >= 6 { break; }
            if (t.to_lowercase().contains(&q) || u.to_lowercase().contains(&q)) && seen.insert(u.clone()) { out.push(json!({"t": t, "u": u, "k": k})); }
        }
        self.suggestions = out;
        if self.suggestions.is_empty() { self.suggest_close(); return; }
        if self.overlay_role != "suggest" || self.overlay.is_none() { self.open_overlay("suggest"); let _ = self.bar.focus(); }
    }
    pub(crate) fn suggest_close(&mut self) { if self.overlay_role == "suggest" { self.overlay = None; } }

    // ---------- Leak-Test ----------
    pub(crate) fn leak_run(&mut self) {
        let (p, vpn_on, dns) = (self.px.clone(), self.settings.vpn_enabled, self.settings.dns_mode.clone());
        let flags = net::TabFlags::new();
        flags.adblock.store(false, std::sync::atomic::Ordering::Relaxed);
        let port = self.net.listener(&flags);
        let tor = self.vpn_status.clone();
        self.leak = Some(json!({"busy": true}));
        std::thread::spawn(move || {
            let get = |agent: &ureq::Agent| agent.get("https://api.ipify.org").timeout(std::time::Duration::from_secs(25)).call().ok().and_then(|r| r.into_string().ok());
            let direct = get(&ureq::AgentBuilder::new().build());
            let via = ureq::Proxy::new(format!("socks5://127.0.0.1:{port}")).ok().and_then(|px| get(&ureq::AgentBuilder::new().proxy(px).build()));
            let verdict = match (&direct, &via, vpn_on) {
                (Some(d), Some(v), true) if d == v => "LEAK: Browser-IP = echte IP (VPN wirkt nicht)",
                (_, Some(_), true) => "OK: Browser läuft über Tor",
                (_, Some(_), false) => "VPN aus – Browser nutzt deine normale IP",
                _ => "Test nicht möglich (keine Verbindung)",
            };
            drop(flags);
            let _ = p.send_event(Ev::Leak(json!({"busy": false, "direct": direct, "browser": via, "vpn": vpn_on, "tor": tor, "dns": dns, "verdict": verdict})));
        });
    }

    // ---------- Profile ----------
    pub(crate) fn profile_open(&mut self, name: &str) {
        let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(24).collect();
        if n.is_empty() { return; }
        if let Ok(exe) = std::env::current_exe() { let _ = std::process::Command::new(exe).arg("--profile").arg(&n).spawn(); }
    }

    // ---------- Tab-Vorschau ----------
    pub(crate) fn capture_active(&self) {
        #[cfg(all(windows, feature = "previews"))]
        if let Some(t) = self.tabs.iter().find(|t| t.id == self.active) {
            if let Some(v) = &t.view { let (id, p) = (t.id, self.px.clone()); crate::preview::capture(v, move |png| { let _ = p.send_event(Ev::Thumb(id, png)); }); }
        }
    }
    /// PNG -> Seitenleisten-UI verkleinert es per Canvas und schickt ein kleines JPEG zurück ("thumb")
    pub(crate) fn thumb_in(&mut self, id: u32, png: Vec<u8>) {
        use base64::Engine;
        let b = base64::engine::general_purpose::STANDARD.encode(png);
        let _ = self.side.evaluate_script(&format!("window.__cap&&__cap({id},'data:image/png;base64,{b}')"));
    }
    pub(crate) fn hover(&mut self, id: u32, x: f64, y: f64) {
        if self.preview == Some(id) || !self.thumbs.contains_key(&id) { return; }
        self.preview = Some(id);
        self.preview_xy = (x, y);
        if self.overlay_role != "preview" || self.overlay.is_none() { self.open_overlay("preview"); } else { self.relayout(); }
    }
    pub(crate) fn hover_end(&mut self) { self.preview = None; if self.overlay_role == "preview" { self.overlay = None; } }

    // ---------- Notizen ----------
    pub(crate) fn notes_json(&self) -> Value { json!(std::fs::read_to_string(data::home().join("notes.txt")).unwrap_or_default()) }
    pub(crate) fn extra_json(&self) -> Value {
        json!({"mem": self.mem_mb, "route": self.tabs.iter().find(|t| t.id == self.active).map(|t| t.flags.route.lock().unwrap().clone()).unwrap_or_default(),
               "preview": self.preview.and_then(|id| self.tabs.iter().find(|t| t.id == id).map(|t| json!({"title": t.title, "url": t.url, "thumb": self.thumbs.get(&id)}))), "focus": self.focus, "suggest": self.suggestions, "leak": self.leak, "profile": data::profile(),
               "profiles": data::profiles(), "notes": if self.overlay_role == "tools" { self.notes_json() } else { Value::Null }})
    }
    pub(crate) fn more2(&mut self, t: &str, v: &Value) {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        match t {
            "suggest" => self.suggest(&s("q")),
            "thumb" => { let d = s("d"); if d.len() < 300_000 { self.thumbs.insert(v["id"].as_u64().unwrap_or(0) as u32, d); } }
            "hover" => self.hover(v["id"].as_u64().unwrap_or(0) as u32, v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0)),
            "hover_end" => self.hover_end(),
            "suggest_close" => self.suggest_close(),
            "leak_run" => self.leak_run(),
            "profile_open" => self.profile_open(&s("name")),
            "notes_save" => { let _ = std::fs::write(data::home().join("notes.txt"), s("v")); }
            "bg_pick" => {
                let p = self.px.clone();
                std::thread::spawn(move || { if let Some(r) = rfd::FileDialog::new().add_filter("Bild", &["jpg", "jpeg", "png", "webp"]).pick_file() { let _ = p.send_event(Ev::BgPick(r)); } });
            }
            "bg_clear" => { let _ = std::fs::remove_file(data::home().join("bg.img")); self.notice = "Hintergrund entfernt (beim nächsten neuen Tab sichtbar)".into(); }
            _ => {}
        }
    }
    pub(crate) fn bg_set(&mut self, p: std::path::PathBuf) {
        match std::fs::metadata(&p) {
            Ok(m) if m.len() <= 4 << 20 => { let _ = std::fs::copy(&p, data::home().join("bg.img")); self.notice = "Hintergrund gesetzt (beim nächsten neuen Tab sichtbar)".into(); }
            _ => self.notice = "Bild zu groß (max. 4 MB)".into(),
        }
    }
    pub(crate) fn lock_vault(&mut self) { self.vault = None; let _ = vault::exists(); let _ = START; }
}
