//! Gaming-Modus, Arbeitsbereiche, Vorlesen, QR, Datenschutz-Bericht, HTTPS-Warnseite, Passwort-Check, 2FA, VirusTotal, Import
use crate::shield::{host_of, norm};
use crate::{data, importer, net, now, totp, vault, App, Ev, Tab};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn esc(x: &str) -> String { x.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }

impl App {
    /// HTTP-Client, der denselben Weg nimmt wie der Browser (Tor/DNS/Kill-Switch).
    pub(crate) fn browser_agent(&self) -> (ureq::Agent, Arc<net::TabFlags>) {
        let flags = net::TabFlags::new();
        flags.adblock.store(false, std::sync::atomic::Ordering::Relaxed);
        let port = self.net.listener(&flags);
        let mut b = ureq::AgentBuilder::new().timeout(Duration::from_secs(25));
        if let Ok(p) = ureq::Proxy::new(format!("socks5://127.0.0.1:{port}")) { b = b.proxy(p); }
        (b.build(), flags)
    }

    // ---------- Gaming-Modus ----------
    pub(crate) fn gaming_toggle(&mut self) {
        self.gaming = !self.gaming;
        if self.gaming {
            self.sleep_others(); self.ext_views.clear(); self.suggest_close(); self.split = None; self.ensure_active_view();
            self.notice = "Gaming-Modus an: Hintergrund-Tabs schlafen, Erweiterungen pausiert".into();
        } else { self.start_all_bg(); self.notice = "Gaming-Modus aus".into(); }
    }

    // ---------- Arbeitsbereiche ----------
    fn collect_session(&self) -> data::Session {
        let ok = |t: &&Tab| !t.private && (t.url.starts_with("http") || t.url.starts_with("liquid:"));
        let tabs: Vec<data::SessTab> = self.tabs.iter().filter(ok).map(|t| data::SessTab { url: t.url.clone(), title: t.title.clone(), group: t.group }).collect();
        let active = self.tabs.iter().filter(ok).position(|t| t.id == self.active).unwrap_or(0);
        data::Session { tabs, active }
    }
    fn load_tabs(&mut self, tabs: &[data::SessTab], active: usize) {
        self.tabs.clear(); self.split = None; self.overlay = None;
        for st in tabs {
            let id = self.next_id; self.next_id += 1;
            self.tabs.push(Tab { id, url: st.url.clone(), title: st.title.clone(), view: None, last_active: Instant::now(), private: false,
                flags: net::TabFlags::new(), token: String::new(), group: st.group, last_gesture: Instant::now() });
        }
        if self.tabs.is_empty() { self.new_tab(None, false); return; }
        self.active = self.tabs[active.min(self.tabs.len() - 1)].id;
        self.ensure_active_view();
        self.relayout();
    }
    fn ws_store(&mut self, name: &str) {
        let s = self.collect_session();
        let w = crate::settings::Workspace { name: name.into(), tabs: s.tabs, active: s.active };
        match self.settings.workspaces.iter_mut().find(|x| x.name == name) { Some(x) => *x = w, None => self.settings.workspaces.push(w) }
    }
    pub(crate) fn ws_save(&mut self, name: &str) {
        let name = name.trim().chars().take(40).collect::<String>();
        if name.is_empty() { return; }
        self.ws_store(&name);
        self.settings.active_workspace = name;
        self.settings.save();
    }
    pub(crate) fn ws_open(&mut self, name: &str) {
        let Some(target) = self.settings.workspaces.iter().find(|w| w.name == name).cloned() else { return };
        let cur = self.settings.active_workspace.clone();
        if !cur.is_empty() && self.settings.workspaces.iter().any(|w| w.name == cur) { self.ws_store(&cur); }
        self.settings.active_workspace = name.to_string();
        self.settings.save();
        self.load_tabs(&target.tabs, target.active);
    }
    pub(crate) fn ws_delete(&mut self, name: &str) {
        self.settings.workspaces.retain(|w| w.name != name);
        if self.settings.active_workspace == name { self.settings.active_workspace.clear(); }
        self.settings.save();
    }

    // ---------- Vorlesen ----------
    pub(crate) fn tts_read(&mut self) {
        let p = self.px.clone();
        let js = "(function(){const s=String(getSelection()||'').trim();if(s)return s.slice(0,30000);const a=document.querySelector('article')||document.querySelector('main')||document.body;return (a.innerText||'').replace(/\\s+/g,' ').trim().slice(0,30000);})()";
        if let Some(v) = self.active_view() { let _ = v.evaluate_script_with_callback(js, move |r| { let _ = p.send_event(Ev::TtsText(r)); }); }
    }
    pub(crate) fn tts_stop(&mut self) { if let Some(mut c) = self.tts.take() { let _ = c.kill(); } }
    pub(crate) fn tts_speak(&mut self, raw: String) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let text: String = serde_json::from_str(&raw).unwrap_or_default();
        if text.trim().is_empty() { self.notice = "Kein Text zum Vorlesen gefunden".into(); return; }
        self.tts_stop();
        let custom = self.settings.tts_cmd.trim().to_string();
        let mut c = if !custom.is_empty() {
            if cfg!(windows) { let mut c = Command::new("cmd"); c.args(["/C", &custom]); c } else { let mut c = Command::new("sh"); c.args(["-c", &custom]); c }
        } else if cfg!(windows) {
            let voice = self.settings.tts_voice.replace('\'', "''");
            let script = format!("Add-Type -AssemblyName System.Speech;$t=[Console]::In.ReadToEnd();$s=New-Object System.Speech.Synthesis.SpeechSynthesizer;$s.Rate={};if('{voice}'){{try{{$s.SelectVoice('{voice}')}}catch{{}}}};$s.Speak($t)",
                self.settings.tts_rate.clamp(-10, 10));
            let mut c = Command::new("powershell"); c.args(["-NoProfile", "-NonInteractive", "-Command", &script]); c
        } else { self.notice = "Vorlesen: eigenes TTS-Kommando in den Einstellungen eintragen (Windows-Stimme nur unter Windows)".into(); return; };
        #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
        match c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(mut child) => {
                if let Some(mut si) = child.stdin.take() { let t = text.clone(); std::thread::spawn(move || { let _ = si.write_all(t.as_bytes()); }); }
                self.tts = Some(child);
            }
            Err(e) => self.notice = format!("Vorlesen fehlgeschlagen: {e}"),
        }
    }
    pub(crate) fn tts_voices(&self) {
        if !cfg!(windows) { return; }
        let p = self.px.clone();
        std::thread::spawn(move || {
            let mut c = std::process::Command::new("powershell");
            c.args(["-NoProfile", "-NonInteractive", "-Command", "Add-Type -AssemblyName System.Speech;(New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices()|%{$_.VoiceInfo.Name}"]);
            #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
            let out = c.output().map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let _ = p.send_event(Ev::Notice(format!("Stimmen: {out}")));
        });
    }

    // ---------- Seite ans Handy ----------
    pub(crate) fn share_phone(&mut self) {
        let Some(url) = self.tabs.iter().find(|t| t.id == self.active).map(|t| t.url.clone()).filter(|u| u.starts_with("http")) else { self.notice = "Keine Webseite geöffnet".into(); return };
        match qrcode::QrCode::new(url.as_bytes()) {
            Ok(code) => {
                let svg = code.render::<qrcode::render::svg::Color>().min_dimensions(260, 260).quiet_zone(true).build();
                self.qr = Some((svg, url));
                self.open_overlay("qr");
            }
            Err(_) => self.notice = "Adresse zu lang für einen QR-Code".into(),
        }
    }
    pub(crate) fn qr_save(&mut self) {
        use std::io::Write;
        let (Some((_, url)), folder) = (self.qr.clone(), self.settings.sync_folder.clone()) else { return };
        if folder.is_empty() { self.notice = "Zuerst einen Sync-Ordner wählen (Einstellungen)".into(); return; }
        match std::fs::OpenOptions::new().create(true).append(true).open(std::path::Path::new(&folder).join("from-liquidflow.txt")) {
            Ok(mut f) => { let _ = writeln!(f, "{url}"); self.notice = "Link im Sync-Ordner abgelegt (from-liquidflow.txt)".into(); }
            Err(e) => self.notice = format!("Ablegen fehlgeschlagen: {e}"),
        }
    }

    // ---------- Datenschutz-Bericht ----------
    pub(crate) fn save_stats(&self) { if self.settings.stats_enabled { data::save("stats.json", &self.net.stats_snapshot()); } }
    fn privacy_json(&self) -> Value {
        let st = self.net.stats_snapshot();
        let today = now() / 86400;
        let week: Vec<Value> = (today.saturating_sub(6)..=today).map(|d| json!({"day": d, "n": st.daily.get(&d).copied().unwrap_or(0)})).collect();
        let total: u64 = week.iter().map(|w| w["n"].as_u64().unwrap_or(0)).sum();
        let mut top: Vec<(&String, &u64)> = st.top.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        let mut page: Vec<(String, u32)> = self.tabs.iter().find(|t| t.id == self.active).map(|t| t.flags.blocked.lock().unwrap().iter().map(|(k, v)| (k.clone(), *v)).collect()).unwrap_or_default();
        page.sort_by(|a, b| b.1.cmp(&a.1));
        json!({"week": week, "total": total, "top": top.into_iter().take(10).map(|(d, n)| json!({"d": d, "n": n})).collect::<Vec<_>>(),
               "page": page.into_iter().take(40).map(|(d, n)| json!({"d": d, "n": n})).collect::<Vec<_>>(), "on": self.settings.stats_enabled})
    }

    // ---------- HTTPS-Warnseite ----------
    pub(crate) fn https_probe(&mut self, id: u32, url: String) {
        let (agent, flags) = self.browser_agent();
        let p = self.px.clone();
        std::thread::spawn(move || {
            let https = url.replacen("http://", "https://", 1);
            let ok = match agent.head(&https).timeout(Duration::from_secs(6)).call() { Ok(_) | Err(ureq::Error::Status(_, _)) => true, Err(_) => false };
            drop(flags);
            let _ = p.send_event(Ev::ProbeDone(id, url, ok));
        });
    }
    pub(crate) fn probe_done(&mut self, id: u32, url: String, ok: bool) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return };
        if ok {
            let https = url.replacen("http://", "https://", 1);
            self.tabs[i].url = https.clone();
            if let Some(v) = &self.tabs[i].view { let _ = v.load_url(&https); }
            return;
        }
        let nonce = crate::token();
        self.http_nonce.insert(nonce.clone(), (id, url.clone()));
        self.tabs[i].url = url.clone();
        let host = esc(&host_of(&url));
        let html = format!(r#"<!doctype html><meta charset="utf-8"><title>Nicht sicher</title><style>
html,body{{height:100%;margin:0;background:radial-gradient(circle at 30% 20%,#3a1a2a,#0a0f1e);color:#e8f1ff;font:15px system-ui,sans-serif}}
body{{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:16px;text-align:center;padding:24px}}
.i{{width:64px;height:64px;border-radius:50% 50% 50% 12px;background:linear-gradient(135deg,#fb7185,#fbbf24);display:flex;align-items:center;justify-content:center;font-size:30px}}
h1{{margin:0;font-weight:600}}p{{margin:0;opacity:.75;max-width:460px}}.r{{display:flex;gap:10px}}
a{{padding:10px 20px;border-radius:12px;border:1px solid rgba(255,255,255,.2);background:rgba(255,255,255,.08);color:inherit;text-decoration:none}}a.p{{background:linear-gradient(135deg,#5ad1ff,#7b6bff);color:#06101f;border:0;font-weight:600}}</style>
<div class="i">⚠</div><h1>Keine sichere Verbindung</h1><p><b>{host}</b> bietet kein HTTPS an. Deine Daten könnten unterwegs mitgelesen oder verändert werden.</p>
<div class="r"><a class="p" href="javascript:history.back()">Zurück</a><a href="{}#lf-allow-{nonce}">Trotzdem öffnen</a></div>"#, esc(&url));
        if let Some(v) = &self.tabs[i].view { let _ = v.load_html(&html); }
    }
    pub(crate) fn allow_http(&mut self, id: u32, nonce: String) {
        let Some((tid, url)) = self.http_nonce.remove(&nonce) else { return };
        if tid != id { return; }
        self.http_ok.insert(host_of(&url));
        self.sync();
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == id) { t.url = url.clone(); if let Some(v) = &t.view { let _ = v.load_url(&url); } }
    }

    // ---------- Passwort-Check (Have I Been Pwned, k-Anonymität) ----------
    pub(crate) fn hibp_run(&mut self) {
        if !self.settings.hibp_enabled { self.notice = "Passwort-Check ist in den Einstellungen ausgeschaltet".into(); return; }
        let Some(list) = self.vault.as_ref().map(|v| v.entries.iter().map(|e| (e.id.clone(), e.pass.clone())).collect::<Vec<_>>()) else { return };
        let (agent, flags) = self.browser_agent();
        let p = self.px.clone();
        self.notice = "Prüfe Passwörter…".into();
        std::thread::spawn(move || {
            use sha1::{Digest, Sha1};
            let mut out = vec![];
            for (id, pw) in list {
                let h = format!("{:X}", Sha1::digest(pw.as_bytes()));
                let (pre, suf) = h.split_at(5);
                let body = agent.get(&format!("https://api.pwnedpasswords.com/range/{pre}")).set("Add-Padding", "true").call().ok().and_then(|r| r.into_string().ok());
                let n = body.map(|b| b.lines().find_map(|l| { let (s, c) = l.split_once(':')?; (s.trim().eq_ignore_ascii_case(suf)).then(|| c.trim().parse::<u64>().unwrap_or(0)) }).unwrap_or(0));
                out.push((id, n));
            }
            drop(flags);
            let _ = p.send_event(Ev::Pwned(out));
        });
    }
    pub(crate) fn pwned_done(&mut self, r: Vec<(String, Option<u64>)>) {
        let bad = r.iter().filter(|x| x.1.map_or(false, |n| n > 0)).count();
        let failed = r.iter().any(|x| x.1.is_none());
        for (id, n) in r { self.pwned.insert(id, n); }
        self.notice = if failed { "Passwort-Check: Server nicht erreichbar".into() } else if bad > 0 { format!("{bad} Passwort/Passwörter in Datenlecks gefunden!") } else { "Keine geleakten Passwörter gefunden".into() };
    }

    pub(crate) fn vault_json(&self) -> Value {
        let t = now();
        let entries: Vec<Value> = self.vault.as_ref().map(|v| v.entries.iter().map(|e| {
            let code = totp::parse(&e.totp).map(|x| (totp::code(&x, t), totp::remaining(&x, t)));
            json!({"id": e.id, "host": e.host, "user": e.user, "dup": v.entries.iter().filter(|x| x.pass == e.pass).count() > 1,
                   "weak": e.pass.chars().count() < 10, "pwned": self.pwned.get(&e.id).and_then(|x| *x),
                   "code": code.as_ref().map(|c| c.0.clone()), "rem": code.as_ref().map(|c| c.1)})
        }).collect()).unwrap_or_default();
        json!({"exists": vault::exists(), "unlocked": self.vault.is_some(), "error": self.vault_err, "entries": entries})
    }

    // ---------- 2FA ----------
    pub(crate) fn totp_fill(&mut self) {
        let Some(t) = self.tabs.iter().find(|t| t.id == self.active) else { return };
        let (host, secure) = (host_of(&t.url), t.url.starts_with("https://"));
        let Some(v) = &self.vault else { self.open_overlay("vault"); return };
        if !secure { self.notice = "2FA-Autofill nur auf HTTPS-Seiten".into(); return; }
        let code = v.entries.iter().find(|e| norm(&e.host) == host && !e.totp.is_empty()).and_then(|e| totp::parse(&e.totp)).map(|x| totp::code(&x, now()));
        match code {
            Some(c) => self.eval_active(&format!("(function(c){{const el=document.activeElement;if(el&&['INPUT','TEXTAREA'].includes(el.tagName)){{Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el),'value').set.call(el,c);el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}));}}}})({})", serde_json::to_string(&c).unwrap())),
            None => self.notice = format!("Kein 2FA-Eintrag für {host}"),
        }
    }

    // ---------- Download-Schutz (VirusTotal) ----------
    pub(crate) fn vt_check(&mut self, path: String) {
        const RISKY: [&str; 14] = ["exe", "msi", "bat", "cmd", "ps1", "vbs", "js", "scr", "dll", "jar", "iso", "lnk", "hta", "zip"];
        let ext = std::path::Path::new(&path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !self.settings.vt_enabled || self.settings.vt_key.trim().is_empty() || !RISKY.contains(&ext.as_str()) { return; }
        if let Some(d) = self.downloads.iter_mut().rev().find(|d| d.path == path) { d.vt = "prüfe…".into(); }
        let (agent, flags) = self.browser_agent();
        let (key, p) = (self.settings.vt_key.trim().to_string(), self.px.clone());
        std::thread::spawn(move || {
            use sha2::{Digest, Sha256};
            use std::io::Read;
            let res = (|| -> Result<String, String> {
                let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                let (mut h, mut buf) = (Sha256::new(), vec![0u8; 1 << 16]);
                loop { let n = f.read(&mut buf).map_err(|e| e.to_string())?; if n == 0 { break; } h.update(&buf[..n]); }
                let hash = format!("{:x}", h.finalize());
                match agent.get(&format!("https://www.virustotal.com/api/v3/files/{hash}")).set("x-apikey", &key).call() {
                    Ok(r) => {
                        let v: Value = r.into_json().map_err(|e| e.to_string())?;
                        let s = &v["data"]["attributes"]["last_analysis_stats"];
                        let g = |k: &str| s[k].as_u64().unwrap_or(0);
                        let (mal, sus) = (g("malicious"), g("suspicious"));
                        let total = mal + sus + g("harmless") + g("undetected");
                        Ok(if mal > 0 { format!("⚠ GEFAHR: {mal}/{total} Scanner melden Schadsoftware") } else if sus > 0 { format!("⚠ verdächtig ({sus}/{total})") } else { format!("✓ sauber (0/{total})") })
                    }
                    Err(ureq::Error::Status(404, _)) => Ok("? unbekannt – noch nie bei VirusTotal gesehen, Vorsicht".into()),
                    Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => Ok("API-Schlüssel ungültig".into()),
                    Err(ureq::Error::Status(429, _)) => Ok("Limit erreicht – später erneut versuchen".into()),
                    Err(e) => Err(e.to_string()),
                }
            })();
            drop(flags);
            let _ = p.send_event(Ev::Vt(path, res.unwrap_or_else(|e| format!("Prüfung fehlgeschlagen ({e})"))));
        });
    }

    // ---------- Import ----------
    pub(crate) fn import_run(&mut self, ids: Vec<String>, bm: bool, hist: bool) {
        let p = self.px.clone();
        self.notice = "Importiere…".into();
        std::thread::spawn(move || {
            let (mut all_bm, mut all_h, mut errs) = (vec![], vec![], vec![]);
            let src = importer::sources();
            for id in ids {
                let Some(s) = src.iter().find(|s| s.id == id) else { continue };
                match importer::import(s, bm, hist) { Ok((b, h)) => { all_bm.extend(b); all_h.extend(h); } Err(e) => errs.push(format!("{}: {e}", s.name)) }
            }
            let _ = p.send_event(Ev::ImportData(all_bm, all_h, errs.join("; ")));
        });
    }
    pub(crate) fn import_done(&mut self, bm: Vec<data::Bookmark>, h: Vec<data::HistEntry>, errs: String) {
        let (mut nb, mut nh) = (0, 0);
        for b in bm { if !self.bookmarks.iter().any(|x| x.url == b.url) { self.bookmarks.push(b); nb += 1; } }
        let known: std::collections::HashSet<String> = self.history.iter().map(|x| x.url.clone()).collect();
        for e in h { if !known.contains(&e.url) { self.history.push(e); nh += 1; } }
        self.history.sort_by_key(|x| x.t);
        if self.history.len() > 5000 { let cut = self.history.len() - 5000; self.history.drain(..cut); }
        data::save("bookmarks.json", &self.bookmarks);
        data::save("history.json", &self.history);
        self.notice = if errs.is_empty() { format!("Importiert: {nb} Lesezeichen, {nh} Verlaufseinträge") } else { format!("Importiert: {nb} Lesezeichen, {nh} Verlauf – {errs}") };
    }

    // ---------- Zustand für die Oberfläche ----------
    pub(crate) fn extra_all(&self) -> Value {
        let mut v = self.extra_json();
        let mut m = Map::new();
        m.insert("gaming".into(), json!(self.gaming));
        m.insert("workspaces".into(), json!(self.settings.workspaces.iter().map(|w| json!({"name": w.name, "tabs": w.tabs.len()})).collect::<Vec<_>>()));
        match self.overlay_role.as_str() {
            "privacy" => { m.insert("privacy".into(), self.privacy_json()); }
            "onboard" | "import" => { m.insert("imports".into(), json!(importer::sources().iter().map(|s| json!({"id": s.id, "name": s.name})).collect::<Vec<_>>())); }
            "qr" => { m.insert("qr".into(), json!(self.qr.as_ref().map(|(s, u)| json!({"svg": s, "url": u})))); }
            _ => {}
        }
        if let Some(o) = v.as_object_mut() { o.extend(m); }
        v
    }

    pub(crate) fn more3(&mut self, t: &str, v: &Value) {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        match t {
            "ws_save" => self.ws_save(&s("name")),
            "ws_open" => self.ws_open(&s("name")),
            "ws_delete" => self.ws_delete(&s("name")),
            "hibp_run" => self.hibp_run(),
            "vault_refresh" => {}
            "qr_save" => self.qr_save(),
            "tts_voices" => self.tts_voices(),
            "import_run" => {
                let ids: Vec<String> = v["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
                self.import_run(ids, v["bm"].as_bool().unwrap_or(true), v["hist"].as_bool().unwrap_or(true));
            }
            _ => {}
        }
    }
}
