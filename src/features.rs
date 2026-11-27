//! Erweiterungs-Runtime, Passwort-Tresor, Sync, Updates, Split-View, Tab-Gruppen (Teil von App)
use crate::shield::{host_of, norm};
use crate::{data, ext, net, now, sync, update, vault, App, Ev};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Instant;
use wry::{ProxyConfig, ProxyEndpoint, WebView};

pub(crate) struct ExtView { pub ext: String, pub kind: &'static str, pub view: WebView, pub token: String, pub _flags: Arc<net::TabFlags> }
#[derive(Clone, PartialEq)]
pub(crate) enum Origin { Tab(u32), Ext(String, &'static str) }
pub(crate) struct Pending { pub origin: Origin, pub call: u64, pub remaining: usize }

fn kind_of(k: &str) -> &'static str { if k == "popup" { "popup" } else { "bg" } }

impl App {
    // ---------- Layout: Split-View & Gruppen ----------
    pub(crate) fn rect_for(&self, id: u32) -> wry::Rect {
        let (w, h) = self.size();
        let (x, y) = (self.side_w(), self.top_h());
        let (pw, ph) = (w - x, h - y);
        match self.split {
            Some(_) if id == self.active => crate::rect(x, y, pw / 2.0 - 2.0, ph),
            Some(s) if id == s => crate::rect(x + pw / 2.0 + 2.0, y, pw / 2.0 - 2.0, ph),
            _ => crate::rect(x, y, pw, ph),
        }
    }
    pub(crate) fn split_toggle(&mut self) {
        if self.split.is_some() { self.split = None; }
        else if let Some(o) = self.tabs.iter().find(|t| t.id != self.active).map(|t| t.id) { self.split = Some(o); }
        else { let a = self.active; self.new_tab(None, false); self.split = Some(self.active); self.active = a; }
        self.ensure_active_view();
        self.relayout();
    }
    pub(crate) fn split_swap(&mut self) {
        if let Some(s) = self.split { self.split = Some(self.active); self.active = s; self.ensure_active_view(); self.relayout(); }
    }
    pub(crate) fn tab_group(&mut self) {
        let a = self.active;
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == a) { t.group = (t.group + 1) % 6; }
    }

    // ---------- Erweiterungen ----------
    pub(crate) fn enabled_exts(&self) -> Vec<ext::Ext> {
        self.exts.iter().filter(|e| !self.settings.ext_disabled.contains(&e.id)).cloned().collect()
    }
    pub(crate) fn start_all_bg(&mut self) { for e in self.enabled_exts() { self.start_ext_view(&e, "bg"); } }
    pub(crate) fn reload_exts(&mut self) {
        self.exts = ext::load_all();
        self.ext_views.clear();
        self.start_all_bg();
        self.rebuild_views();
    }
    fn start_ext_view(&mut self, e: &ext::Ext, kind: &'static str) {
        let path = if kind == "bg" { if e.m["background"].is_null() { return; } "__bg.html".to_string() } else { match e.popup() { Some(p) => p, None => return } };
        let (token, flags) = (crate::token(), net::TabFlags::new());
        let port = self.net.listener(&flags);
        let (p, id) = (self.px.clone(), e.id.clone());
        let b = wvb!(&self.settings)
            .with_custom_protocol("lfext".into(), ext::protocol_handler())
            .with_initialization_script(&ext::page_script(e, &token, kind))
            .with_proxy_config(ProxyConfig::Socks5(ProxyEndpoint { host: "127.0.0.1".into(), port: port.to_string() }))
            .with_ipc_handler(move |r| { let _ = p.send_event(Ev::ExtIpc(id.clone(), kind.into(), r.body().clone())); })
            .with_url(&ext::url(&e.id, &path));
        if let Ok(v) = b.build_as_child(&self.window) {
            if kind == "bg" { let _ = v.set_visible(false); let _ = v.set_bounds(crate::rect(0.0, 0.0, 1.0, 1.0)); }
            else { let (w, _) = self.size(); let _ = v.set_bounds(crate::rect(w - 400.0, 92.0, 390.0, 560.0)); let _ = v.focus(); }
            self.ext_views.push(ExtView { ext: e.id.clone(), kind, view: v, token, _flags: flags });
        }
    }
    pub(crate) fn open_popup(&mut self, id: &str) {
        self.close_popup();
        match self.enabled_exts().into_iter().find(|e| e.id == id) {
            Some(e) if e.popup().is_some() => self.start_ext_view(&e, "popup"),
            _ => self.notice = "Diese Erweiterung hat kein Popup".into(),
        }
    }
    pub(crate) fn close_popup(&mut self) { self.ext_views.retain(|x| x.kind != "popup"); }

    pub(crate) fn ext_ipc(&mut self, ext: String, kind: String, msg: String) {
        let k = kind_of(&kind);
        let Some((tok, rest)) = msg.split_once('|') else { return };
        if !self.ext_views.iter().any(|x| x.ext == ext && x.kind == k && x.token == tok) { return; }
        self.ext_rpc(Origin::Ext(ext, k), rest);
    }

    fn tab_json(&self, t: &crate::Tab) -> Value { json!({"id": t.id, "url": t.url, "title": t.title, "active": t.id == self.active, "incognito": t.private, "windowId": 1}) }

    pub(crate) fn eval_origin(&self, to: &Origin, js: &str) {
        let v = match to {
            Origin::Tab(t) => self.tabs.iter().find(|x| x.id == *t).and_then(|x| x.view.as_ref()),
            Origin::Ext(e, k) => self.ext_views.iter().find(|x| x.ext == *e && x.kind == *k).map(|x| &x.view),
        };
        if let Some(v) = v { let _ = v.evaluate_script(js); }
    }
    fn resolve(&self, to: &Origin, call: u64, val: &Value) { self.eval_origin(to, &format!("window.__lfResolve&&__lfResolve({call},{val})")); }

    pub(crate) fn ext_rpc(&mut self, from: Origin, raw: &str) {
        let Ok(v) = serde_json::from_str::<Value>(raw) else { return };
        let op = v["op"].as_str().unwrap_or("").to_string();
        let ext = v["ext"].as_str().unwrap_or("").to_string();
        let id = v["id"].as_u64().unwrap_or(0);
        if op == "close_popup" { self.close_popup(); return; }
        if !self.enabled_exts().iter().any(|e| e.id == ext) { return; }
        if let Origin::Ext(own, _) = &from { if *own != ext { return; } }
        let d = v["data"].clone();
        match op.as_str() {
            "storage" => { let r = self.ext_storage(&ext, &d); self.resolve(&from, id, &r); }
            "send" | "tabs.send" => {
                let sender = match &from {
                    Origin::Tab(t) => self.tabs.iter().find(|x| x.id == *t).map(|x| json!({"id": ext, "url": x.url, "tab": self.tab_json(x)})).unwrap_or(json!({"id": ext})),
                    _ => json!({"id": ext}),
                };
                let targets: Vec<Origin> = if op == "send" {
                    self.ext_views.iter().filter(|x| x.ext == ext && Origin::Ext(x.ext.clone(), x.kind) != from).map(|x| Origin::Ext(x.ext.clone(), x.kind)).collect()
                } else { vec![Origin::Tab(d["id"].as_u64().unwrap_or(0) as u32)] };
                if targets.is_empty() { self.resolve(&from, id, &Value::Null); return; }
                self.ext_seq += 1;
                let cid = self.ext_seq;
                self.pending.insert(cid, Pending { origin: from.clone(), call: id, remaining: targets.len() });
                let js = format!("window.__lfDeliver&&__lfDeliver({},{},{},{})", json!(ext), cid, d["msg"], sender);
                for t in targets { self.eval_origin(&t, &js); }
            }
            "resp" => {
                let none = v["none"].as_bool().unwrap_or(false);
                if let Some(p) = self.pending.get_mut(&id) {
                    p.remaining = p.remaining.saturating_sub(1);
                    if !none || p.remaining == 0 {
                        let (o, c) = (p.origin.clone(), p.call);
                        self.pending.remove(&id);
                        let val = if none { Value::Null } else { v["data"].clone() };
                        self.resolve(&o, c, &val);
                    }
                }
            }
            "tabs.query" => {
                let act = d["active"].as_bool();
                let list: Vec<Value> = self.tabs.iter().filter(|t| act.map_or(true, |a| (t.id == self.active) == a)).map(|t| self.tab_json(t)).collect();
                self.resolve(&from, id, &Value::Array(list));
            }
            "tabs.get" => {
                let t = d["id"].as_u64().unwrap_or(0) as u32;
                let r = self.tabs.iter().find(|x| x.id == t).map(|x| self.tab_json(x)).unwrap_or(Value::Null);
                self.resolve(&from, id, &r);
            }
            "tabs.create" => {
                let u = d["url"].as_str().unwrap_or("").to_string();
                if u.starts_with("http") { self.new_tab(Some(u), false); }
                let r = self.tabs.iter().find(|x| x.id == self.active).map(|x| self.tab_json(x)).unwrap_or(Value::Null);
                self.resolve(&from, id, &r);
            }
            "tabs.update" => {
                if let Some(u) = d["props"]["url"].as_str() { if u.starts_with("http") { self.navigate(u.to_string()); } }
                self.resolve(&from, id, &Value::Null);
            }
            "scripting" => {
                let tid = d["tabId"].as_u64().map(|x| x as u32).unwrap_or(self.active);
                let mut js = String::new();
                if let Some(e) = self.exts.iter().find(|e| e.id == ext) {
                    for f in d["files"].as_array().into_iter().flatten().filter_map(|f| f.as_str()).filter(|f| !f.contains("..")) {
                        if let Ok(c) = std::fs::read_to_string(e.dir.join(f.trim_start_matches('/'))) { js.push_str(&format!("(function(){{{c}\n}})();")); }
                    }
                }
                if let Some(f) = d["func"].as_str() { js.push_str(&format!("({f}).apply(null,{});", serde_json::to_string(&d["args"]).unwrap_or("[]".into()))); }
                self.eval_origin(&Origin::Tab(tid), &js);
                self.resolve(&from, id, &json!([]));
            }
            _ => self.resolve(&from, id, &Value::Null),
        }
    }

    fn ext_storage(&mut self, ext: &str, d: &Value) -> Value {
        let m = self.ext_store.entry(format!("{ext}:{}", d["area"].as_str().unwrap_or("local"))).or_default();
        match d["a"].as_str().unwrap_or("") {
            "get" => match &d["k"] {
                Value::Null => Value::Object(m.clone()),
                Value::String(s) => { let mut o = Map::new(); if let Some(x) = m.get(s) { o.insert(s.clone(), x.clone()); } Value::Object(o) }
                Value::Array(a) => { let mut o = Map::new(); for k in a.iter().filter_map(|k| k.as_str()) { if let Some(x) = m.get(k) { o.insert(k.into(), x.clone()); } } Value::Object(o) }
                Value::Object(def) => { let mut o = def.clone(); for k in def.keys() { if let Some(x) = m.get(k) { o.insert(k.clone(), x.clone()); } } Value::Object(o) }
                _ => Value::Object(Map::new()),
            },
            "set" => { if let Some(o) = d["o"].as_object() { for (k, v) in o { m.insert(k.clone(), v.clone()); } } self.ext_dirty = true; Value::Null }
            "remove" => {
                let ks: Vec<String> = match &d["k"] { Value::String(s) => vec![s.clone()], Value::Array(a) => a.iter().filter_map(|k| k.as_str().map(String::from)).collect(), _ => vec![] };
                for k in ks { m.remove(&k); }
                self.ext_dirty = true; Value::Null
            }
            "clear" => { m.clear(); self.ext_dirty = true; Value::Null }
            _ => Value::Null,
        }
    }
    pub(crate) fn save_ext_store(&self) {
        let keep: std::collections::HashMap<_, _> = self.ext_store.iter().filter(|(k, _)| !k.ends_with(":session")).collect();
        data::save("ext_storage.json", &keep);
    }

    // ---------- Passwort-Tresor ----------
    pub(crate) fn vault_unlock(&mut self, pass: &str) {
        self.vault_err.clear();
        if vault::exists() {
            match vault::Vault::unlock(pass) { Some(v) => { self.vault = Some(v); self.vault_last = Instant::now(); self.sync_now(); } None => self.vault_err = "Falsches Passwort".into() }
        } else if pass.chars().count() >= 8 { self.vault = Some(vault::Vault::create(pass)); self.vault_last = Instant::now(); }
        else { self.vault_err = "Mindestens 8 Zeichen".into(); }
    }
    fn fill_js(u: &str, p: &str) -> String {
        format!(r#"(function(u,p){{const vis=e=>e.offsetParent!==null;const pw=[...document.querySelectorAll('input[type=password]')].find(vis);if(!pw)return;
const set=(el,v)=>{{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(el,v);el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));}};
const ins=[...document.querySelectorAll('input')].filter(vis);let un=null;for(let k=ins.indexOf(pw)-1;k>=0;k--){{if(['text','email','tel'].includes((ins[k].type||'text').toLowerCase())){{un=ins[k];break;}}}}
if(un)set(un,u);set(pw,p);}})({},{})"#, serde_json::to_string(u).unwrap(), serde_json::to_string(p).unwrap())
    }
    /// Nur auf Nutzeraktion, nur HTTPS, nur wenn die Seite zum Eintrag passt (Schutz gegen Phishing/stilles Auslesen).
    fn fill_entry(&mut self, pick: Option<&str>) {
        let Some(t) = self.tabs.iter().find(|t| t.id == self.active) else { return };
        let (host, secure) = (host_of(&t.url), t.url.starts_with("https://") || host_of(&t.url) == "localhost");
        let Some(v) = &self.vault else { self.open_overlay("vault"); return };
        if !secure { self.notice = "Autofill nur auf HTTPS-Seiten".into(); return; }
        let hit = v.entries.iter().find(|e| norm(&e.host) == host && pick.map_or(true, |p| p == e.id)).cloned();
        match hit {
            Some(e) => { self.eval_active(&Self::fill_js(&e.user, &e.pass)); self.vault_last = Instant::now(); }
            None => self.notice = format!("Kein passender Login für {host}"),
        }
    }
    pub(crate) fn pw_fill(&mut self) { self.fill_entry(None); }
    pub(crate) fn pw_save(&mut self) {
        let Some(t) = self.tabs.iter().find(|t| t.id == self.active) else { return };
        let host = host_of(&t.url);
        let p = self.px.clone();
        let js = "(function(){const vis=e=>e.offsetParent!==null;const pw=[...document.querySelectorAll('input[type=password]')].find(e=>vis(e)&&e.value);if(!pw)return null;const ins=[...document.querySelectorAll('input')].filter(vis);let u='';for(let k=ins.indexOf(pw)-1;k>=0;k--){if(['text','email','tel'].includes((ins[k].type||'text').toLowerCase())){u=ins[k].value;break;}}return {u,p:pw.value};})()";
        if let Some(v) = &t.view { let _ = v.evaluate_script_with_callback(js, move |r| { let _ = p.send_event(Ev::PwCaptured(host.clone(), r)); }); }
    }
    pub(crate) fn pw_captured(&mut self, host: String, res: String) {
        let v: Value = serde_json::from_str(&res).unwrap_or(Value::Null);
        let (Some(u), Some(p)) = (v["u"].as_str(), v["p"].as_str()) else { self.notice = "Kein ausgefülltes Passwortfeld gefunden".into(); return };
        let Some(vt) = &mut self.vault else { self.notice = "Tresor zuerst entsperren".into(); self.open_overlay("vault"); return };
        vt.entries.retain(|e| !(norm(&e.host) == host && e.user == u));
        vt.entries.push(vault::Entry { id: vault::new_id(), host: host.clone(), user: u.into(), pass: p.into(), totp: String::new() });
        vt.save();
        self.notice = format!("Login für {host} gespeichert");
    }

    // ---------- Sync ----------
    fn make_bundle(&self, ts: u64) -> sync::Bundle {
        sync::Bundle { ts, bookmarks: self.bookmarks.clone(), quick_dials: self.settings.quick_dials.clone(), site_rules: self.settings.site_rules.clone(),
            keymap: self.settings.keymap.clone(), accent: self.settings.accent.clone(), search_engine: self.settings.search_engine.clone(),
            lists: self.settings.blocklists.iter().filter(|l| l.source == "url").cloned().collect() }
    }
    fn apply_bundle(&mut self, b: sync::Bundle) {
        for bm in b.bookmarks { if !self.bookmarks.iter().any(|x| x.url == bm.url) { self.bookmarks.push(bm); } }
        data::save("bookmarks.json", &self.bookmarks);
        for d in b.quick_dials { if !self.settings.quick_dials.iter().any(|x| x.url == d.url) { self.settings.quick_dials.push(d); } }
        self.settings.site_rules.extend(b.site_rules);
        self.settings.keymap.extend(b.keymap);
        if !b.accent.is_empty() { self.settings.accent = b.accent; }
        if !b.search_engine.is_empty() { self.settings.search_engine = b.search_engine; }
        let mut new = vec![];
        for l in b.lists { if !self.settings.blocklists.iter().any(|x| x.location == l.location) { new.push(settings_list(l)); } }
        self.settings.blocklists.extend(new);
        self.settings.save();
        self.sync();
        self.bl_refresh_all();
    }
    pub(crate) fn sync_now(&mut self) {
        let folder = self.settings.sync_folder.clone();
        if folder.is_empty() { return; }
        let Some(pass) = self.vault.as_ref().map(|v| v.pass.to_string()) else { return };
        if let Some(b) = sync::read(&folder, &pass) { if b.ts > self.settings.sync_ts { self.apply_bundle(b); } }
        let ts = now();
        if sync::write(&folder, &pass, &self.make_bundle(ts)).is_ok() { self.settings.sync_ts = ts; self.settings.save(); }
        let (lp, rp) = (vault::path(), std::path::Path::new(&folder).join("vault.lfv"));
        let mt = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if mt(&rp) > mt(&lp) {
            if std::fs::read(&rp).ok().and_then(|b| vault::open(&pass, &b)).is_some() && std::fs::copy(&rp, &lp).is_ok() {
                if let Some(nv) = vault::Vault::unlock(&pass) { self.vault = Some(nv); }
            }
        } else if mt(&lp) > mt(&rp) { let _ = std::fs::copy(&lp, &rp); }
    }

    // ---------- Updates ----------
    /// Installationsordner des Liquid-Flow-Setups (…\LiquidFlow\versions\<ver>\liquidflow.exe -> …\LiquidFlow)
    fn install_root() -> Option<std::path::PathBuf> {
        let r = std::env::current_exe().ok()?.parent()?.parent()?.parent()?.to_path_buf();
        r.join("LiquidFlowSetup.exe").exists().then_some(r)
    }
    fn ver_nums(v: &str) -> Vec<u32> { v.trim_start_matches('v').split('.').map(|x| x.parse().unwrap_or(0)).collect() }
    /// Hat der Hintergrund-Updater schon eine neuere Version installiert, als gerade läuft?
    pub(crate) fn installed_newer(&self) -> Option<String> {
        let cur = std::fs::read_to_string(Self::install_root()?.join("current.txt")).ok()?.trim().to_string();
        (Self::ver_nums(&cur) > Self::ver_nums(env!("CARGO_PKG_VERSION"))).then_some(cur)
    }
    pub(crate) fn update_label(&self) -> Option<String> {
        if self.installed_newer().is_some() { Some("restart".into()) } else { self.release.as_ref().map(|r| r.tag.clone()) }
    }
    pub(crate) fn restart(&mut self) {
        if let Some(r) = Self::install_root() {
            let v = std::fs::read_to_string(r.join("current.txt")).unwrap_or_default();
            let exe = r.join("versions").join(v.trim()).join("liquidflow.exe");
            #[cfg(windows)]
            { use std::os::windows::process::CommandExt;
              let _ = std::process::Command::new("cmd").args(["/C", &format!("ping 127.0.0.1 -n 3 >nul & start \"\" \"{}\"", exe.display())]).creation_flags(0x0800_0000).spawn(); }
            #[cfg(not(windows))] { let _ = exe; }
        }
        self.save_session(); self.save_ext_store();
        self.quit = true;
    }
    pub(crate) fn check_update(&self) {
        // Neuer Installer: Updater-Programm fragen (lädt, prüft und installiert still im Hintergrund)
        if let Some(root) = Self::install_root() {
            let p = self.px.clone();
            std::thread::spawn(move || {
                let ok = std::process::Command::new(root.join("LiquidFlowSetup.exe")).arg("--update").status().map(|s| s.success()).unwrap_or(false);
                let _ = p.send_event(Ev::Updated(ok));
            });
            return;
        }
        let repo = self.settings.update_repo.trim().to_string();
        if repo.is_empty() { return; }
        let p = self.px.clone();
        std::thread::spawn(move || { if let Some(r) = update::check(&repo) { let _ = p.send_event(Ev::UpdateAvail(r)); } });
    }
    pub(crate) fn update_install(&mut self) {
        if self.installed_newer().is_some() { self.restart(); return; }
        if Self::install_root().is_some() { self.notice = "Suche Update…".into(); self.check_update(); return; }
        let Some(r) = self.release.clone() else { self.notice = "Kein Update verfügbar".into(); return };
        self.notice = "Lade Update…".into();
        let p = self.px.clone();
        std::thread::spawn(move || { let _ = p.send_event(Ev::UpdateReady(update::download(&r))); });
    }
    pub(crate) fn update_ready(&mut self, r: Result<std::path::PathBuf, String>) {
        match r {
            Ok(p) => {
                if std::process::Command::new(p).args(["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"]).spawn().is_ok() { self.quit = true; }
                else { self.notice = "Installer konnte nicht gestartet werden".into(); }
            }
            Err(e) => self.notice = e,
        }
    }

    // ---------- weitere IPC-Nachrichten der Oberfläche ----------
    pub(crate) fn ipc_more(&mut self, t: &str, v: &Value) {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        match t {
            "ext_install" => {
                let id = s("v").split(|c: char| !c.is_ascii_lowercase()).find(|x| x.len() == 32 && x.bytes().all(|c| c <= b'p')).map(String::from);
                match id {
                    Some(id) => { self.notice = "Installiere…".into(); let p = self.px.clone(); std::thread::spawn(move || { let _ = p.send_event(Ev::ExtInstalled(ext::install_store(&id))); }); }
                    None => self.notice = "Keine gültige Erweiterungs-ID gefunden".into(),
                }
            }
            "ext_pick" | "ext_pick_dir" => {
                let (p, dir) = (self.px.clone(), t == "ext_pick_dir");
                std::thread::spawn(move || {
                    let d = rfd::FileDialog::new();
                    let r = if dir { d.pick_folder() } else { d.add_filter("Erweiterung", &["crx", "zip"]).pick_file() };
                    if let Some(r) = r { let _ = p.send_event(Ev::ExtPick(r)); }
                });
            }
            "ext_toggle" => {
                let id = s("id");
                if self.settings.ext_disabled.contains(&id) { self.settings.ext_disabled.retain(|x| *x != id); } else { self.settings.ext_disabled.push(id); }
                self.settings.save(); self.reload_exts();
            }
            "ext_remove" => { let id = s("id"); ext::remove(&id); self.settings.ext_disabled.retain(|x| *x != id); self.settings.save(); self.reload_exts(); }
            "ext_popup" => self.open_popup(&s("id")),
            "vault_unlock" => self.vault_unlock(&s("p")),
            "vault_lock" => self.vault = None,
            "vault_add" => {
                let raw = s("host");
                let host = if raw.contains("://") { host_of(&raw) } else { norm(raw.trim()) };
                let totp_in = s("totp");
                if !totp_in.trim().is_empty() && crate::totp::parse(&totp_in).is_none() { self.notice = "2FA-Secret ungültig (Base32-Schlüssel oder otpauth://-Link)".into(); return; }
                if let Some(vt) = &mut self.vault {
                    if !host.is_empty() {
                        vt.entries.retain(|e| !(e.host == host && e.user == s("user")));
                        vt.entries.push(vault::Entry { id: vault::new_id(), host, user: s("user"), pass: s("pass"), totp: s("totp").trim().to_string() });
                        vt.save(); self.vault_last = Instant::now();
                    }
                }
            }
            "vault_del" => { let id = s("id"); if let Some(vt) = &mut self.vault { vt.entries.retain(|e| e.id != id); vt.save(); } }
            "pw_fill_id" => { let id = s("id"); self.fill_entry(Some(&id)); self.overlay = None; }
            "sync_pick" => { let p = self.px.clone(); std::thread::spawn(move || { if let Some(r) = rfd::FileDialog::new().pick_folder() { let _ = p.send_event(Ev::SyncFolder(r)); } }); }
            "notice_clear" => self.notice.clear(),
            "update_check" => self.check_update(),
            _ => {}
        }
    }
}

fn settings_list(mut l: crate::settings::BlockList) -> crate::settings::BlockList { l.count = 0; l.last_updated = 0; l.enabled = true; l }
