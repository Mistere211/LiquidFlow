//! Chrome-Extension-Adapter v2 (alles in Rust, Übersetzung chrome.* -> Rust-Host):
//! Installieren aus Web Store/.crx/.zip/Ordner, Content-Scripts (JS+CSS, run_at, matches), Background-Seite,
//! Messaging (runtime/tabs.sendMessage), storage, tabs, scripting, i18n, Popup. Unbekannte APIs sind harmlose No-ops.
//! Grenzen: Content-Scripts laufen im Seiten-Kontext (keine isolierte Welt), kein webRequest/declarativeNetRequest.
use serde_json::{json, Value};
use std::borrow::Cow;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use wry::http::{Request, Response};

pub fn dir() -> PathBuf { crate::data::home().join("extensions") }
pub fn url(id: &str, path: &str) -> String {
    if cfg!(windows) { format!("http://lfext.localhost/{id}/{path}") } else { format!("lfext://localhost/{id}/{path}") }
}

#[derive(Clone)]
pub struct Ext { pub id: String, pub dir: PathBuf, pub m: Value }

fn messages(dir: &Path, m: &Value) -> Value {
    let loc = m["default_locale"].as_str().unwrap_or("en");
    std::fs::read_to_string(dir.join("_locales").join(loc).join("messages.json")).ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or(json!({}))
}

impl Ext {
    pub fn name(&self) -> String {
        let n = self.m["name"].as_str().unwrap_or(&self.id);
        if let Some(k) = n.strip_prefix("__MSG_").and_then(|x| x.strip_suffix("__")) {
            if let Some(o) = messages(&self.dir, &self.m).as_object() {
                if let Some((_, v)) = o.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)) { return v["message"].as_str().unwrap_or(k).to_string(); }
            }
        }
        n.to_string()
    }
    pub fn popup(&self) -> Option<String> {
        self.m["action"]["default_popup"].as_str().or(self.m["browser_action"]["default_popup"].as_str()).map(|s| s.trim_start_matches('/').to_string())
    }
}

pub fn load_all() -> Vec<Ext> {
    let mut v = vec![];
    if let Ok(rd) = std::fs::read_dir(dir()) {
        for e in rd.flatten() {
            if let Ok(s) = std::fs::read_to_string(e.path().join("manifest.json")) {
                if let Ok(m) = serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')) {
                    v.push(Ext { id: e.file_name().to_string_lossy().into_owned(), dir: e.path(), m });
                }
            }
        }
    }
    v
}

fn finish(id: &str) -> Result<String, String> {
    let d = dir().join(id);
    let s = std::fs::read_to_string(d.join("manifest.json")).map_err(|_| "manifest.json fehlt".to_string())?;
    let m: Value = serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    Ok(Ext { id: id.into(), dir: d, m }.name())
}

fn unpack(id: &str, b: &[u8]) -> Result<String, String> {
    let start = if b.len() > 16 && &b[..4] == b"Cr24" {
        let rd = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as usize;
        if rd(4) == 3 { 12 + rd(8) } else { 16 + rd(8) + rd(12) }
    } else { 0 };
    if start >= b.len() { return Err("Datei beschädigt".into()); }
    let mut z = zip::ZipArchive::new(Cursor::new(&b[start..])).map_err(|e| e.to_string())?;
    let target = dir().join(id);
    let _ = std::fs::remove_dir_all(&target);
    z.extract(&target).map_err(|e| e.to_string())?;
    finish(id)
}

pub fn install_store(id: &str) -> Result<String, String> {
    if id.len() != 32 || !id.bytes().all(|c| (b'a'..=b'p').contains(&c)) { return Err("Ungültige Erweiterungs-ID".into()); }
    let u = format!("https://clients2.google.com/service/update2/crx?response=redirect&prodversion=130.0.0.0&acceptformat=crx2,crx3&x=id%3D{id}%26installsource%3Dondemand%26uc");
    let mut buf = Vec::new();
    ureq::get(&u).call().map_err(|e| e.to_string())?.into_reader().take(150 << 20).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    unpack(id, &buf)
}

fn copy_dir(s: &Path, d: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(d)?;
    for e in std::fs::read_dir(s)?.flatten() {
        let (p, t) = (e.path(), d.join(e.file_name()));
        if p.is_dir() { copy_dir(&p, &t)?; } else { std::fs::copy(&p, &t)?; }
    }
    Ok(())
}

pub fn install_path(p: &Path) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    let id = format!("local{:x}", h.finish());
    if p.is_dir() { copy_dir(p, &dir().join(&id)).map_err(|e| e.to_string())?; finish(&id) }
    else { unpack(&id, &std::fs::read(p).map_err(|e| e.to_string())?) }
}

pub fn remove(id: &str) { if id.chars().all(|c| c.is_ascii_alphanumeric()) { let _ = std::fs::remove_dir_all(dir().join(id)); } }

fn mime(p: &str) -> &'static str {
    match p.rsplit('.').next().unwrap_or("") {
        "html" => "text/html", "js" | "mjs" => "text/javascript", "css" => "text/css", "json" => "application/json",
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "svg" => "image/svg+xml",
        "woff2" => "font/woff2", "woff" => "font/woff", "ttf" => "font/ttf", "wasm" => "application/wasm", _ => "application/octet-stream",
    }
}

fn bg_html(dir: &Path) -> String {
    let m: Value = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()).unwrap_or(json!({}));
    let b = &m["background"];
    let module = if b["type"].as_str() == Some("module") { " type=\"module\"" } else { "" };
    let mut tags: Vec<String> = vec![];
    if let Some(sw) = b["service_worker"].as_str() { tags.push(format!("<script{module} src=\"{}\"></script>", sw.trim_start_matches('/'))); }
    for s in b["scripts"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) { tags.push(format!("<script src=\"{}\"></script>", s.trim_start_matches('/'))); }
    if let Some(p) = b["page"].as_str() { return std::fs::read_to_string(dir.join(p)).unwrap_or_default(); }
    format!("<!doctype html><meta charset=\"utf-8\">{}", tags.join(""))
}

/// Liefert Dateien der Erweiterungen (nur lesen, kein CORS-Header -> Webseiten können den Inhalt nicht per fetch auslesen).
pub fn protocol_handler() -> impl Fn(&str, Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> + 'static {
    move |_wv, req| {
        let nf = || Response::builder().status(404).body(Cow::Borrowed(&b""[..])).unwrap();
        let path = req.uri().path().trim_start_matches('/').to_string();
        let (id, rest) = path.split_once('/').unwrap_or((&path, ""));
        if !id.chars().all(|c| c.is_ascii_alphanumeric()) || rest.split('/').any(|s| s == ".." || s.contains('\\')) { return nf(); }
        let d = dir().join(id);
        let body = if rest == "__bg.html" { bg_html(&d).into_bytes() } else { match std::fs::read(d.join(rest)) { Ok(b) => b, Err(_) => return nf() } };
        Response::builder().status(200).header("Content-Type", mime(rest)).body(Cow::Owned(body)).unwrap()
    }
}

fn shim(token: &str, ctx: &str) -> String { SHIM.replace("__TOKEN__", token).replace("__CTX__", ctx) }

/// Init-Script für Hintergrundseite und Popup
pub fn page_script(e: &Ext, token: &str, ctx: &str) -> String {
    format!("{}\nwindow.chrome=window.browser=__lfMake({},{},{});", shim(token, ctx), json!(e.id), e.m, messages(&e.dir, &e.m))
}

/// Init-Script für jeden Tab: Shim + Content-Scripts aller aktiven Erweiterungen
pub fn tab_script(exts: &[Ext], token: &str) -> String {
    if exts.is_empty() { return String::new(); }
    let mut s = shim(token, "content");
    for e in exts {
        let Some(list) = e.m["content_scripts"].as_array() else { continue };
        let mut inner = String::new();
        for cs in list {
            let (m, x) = (cs["matches"].clone(), cs["exclude_matches"].clone());
            let mut body = String::new();
            for css in cs["css"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
                if let Ok(c) = std::fs::read_to_string(e.dir.join(css.trim_start_matches('/'))) {
                    body.push_str(&format!("{{const s=document.createElement('style');s.textContent={};(document.head||document.documentElement).appendChild(s);}}", serde_json::to_string(&c).unwrap()));
                }
            }
            for js in cs["js"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
                if let Ok(c) = std::fs::read_to_string(e.dir.join(js.trim_start_matches('/'))) { body.push_str(&format!("(function(chrome,browser){{{c}\n}})(__c,__c);")); }
            }
            let frames = cs["all_frames"].as_bool().unwrap_or(false);
            let run = if cs["run_at"].as_str() == Some("document_start") { "run();" } else { "if(document.readyState==='loading')addEventListener('DOMContentLoaded',run);else run();" };
            inner.push_str(&format!("if((window.top===window||{frames})&&__lfMatch({m},{x})){{const run=()=>{{{body}}};{run}}}\n"));
        }
        if !inner.is_empty() { s.push_str(&format!("(function(){{const __c=__lfMake({},{},{});{inner}}})();\n", json!(e.id), e.m, messages(&e.dir, &e.m))); }
    }
    s
}

const SHIM: &str = r#"
(function(){
  if(window.__lfMake) return;
  const post=window.ipc&&window.ipc.postMessage.bind(window.ipc), T='__TOKEN__', CTX='__CTX__';
  const isWin=/Windows/.test(navigator.userAgent), BASE=isWin?'http://lfext.localhost/':'lfext://localhost/';
  let seq=0; const pend={}, L={};
  const send=o=>post&&post(T+'|'+JSON.stringify(o));
  window.__lfResolve=(id,val)=>{ const p=pend[id]; if(p){ delete pend[id]; p(val); } };
  const rpc=(op,ext,data)=>new Promise(res=>{ const id=++seq; pend[id]=res; send({op,ext,id,data}); });
  const cb=(p,f)=>f?(p.then(v=>f(v),()=>f()),undefined):p;
  window.__lfDeliver=(ext,cid,msg,sender)=>{
    let done=false, keep=false;
    const reply=v=>{ if(done)return; done=true; send({op:'resp',ext,id:cid,data:v===undefined?null:v}); };
    for(const f of (L[ext]||[])){ try{ if(f(msg,sender,reply)===true) keep=true; }catch(e){} }
    if(!done&&!keep) send({op:'resp',ext,id:cid,none:true});
  };
  const mp=p=>{ if(p==='<all_urls>') return /^(https?|file|ftp):\/\//;
    const m=/^(\*|https?|file|ftp):\/\/([^\/]*)(\/.*)$/.exec(p); if(!m) return null;
    const esc=s=>s.replace(/[.+?^${}()|[\]\\]/g,'\\$&');
    const h=m[2]==='*'?'[^/]*':m[2].startsWith('*.')?'([^/]*\\.)?'+esc(m[2].slice(2)):esc(m[2]);
    return new RegExp('^'+(m[1]==='*'?'https?':m[1])+':\\/\\/'+h+esc(m[3]).replace(/\*/g,'.*')+'$'); };
  window.__lfMatch=(inc,exc)=>(inc||[]).some(p=>{const r=mp(p);return r&&r.test(location.href)})&&!(exc||[]).some(p=>{const r=mp(p);return r&&r.test(location.href)});
  if(CTX==='popup') addEventListener('keydown',e=>{ if(e.key==='Escape') send({op:'close_popup',ext:'*'}); });
  const deep=()=>new Proxy(function(){},{get:(t,k)=>k==='then'?undefined:(k==='addListener'||k==='removeListener'||k==='hasListener')?(()=>false):deep(),apply:()=>Promise.resolve()});
  window.__lfMake=(ext,man,msgs)=>{
    const ls=(L[ext]=L[ext]||[]), al=[];
    const ev=()=>({addListener(){},removeListener(){},hasListener(){return false}});
    const area=a=>({get:(k,f)=>cb(rpc('storage',ext,{a:'get',area:a,k:k===undefined?null:k}),f),set:(o,f)=>cb(rpc('storage',ext,{a:'set',area:a,o}),f),
      remove:(k,f)=>cb(rpc('storage',ext,{a:'remove',area:a,k}),f),clear:f=>cb(rpc('storage',ext,{a:'clear',area:a}),f),onChanged:ev()});
    const api={
      runtime:{id:ext,getURL:p=>BASE+ext+'/'+String(p).replace(/^\//,''),getManifest:()=>man,
        sendMessage:(...a)=>{ const f=typeof a[a.length-1]==='function'?a.pop():null; return cb(rpc('send',ext,{msg:a.length>1?a[1]:a[0]}),f); },
        onMessage:{addListener:f=>ls.push(f),removeListener:f=>{const i=ls.indexOf(f);if(i>=0)ls.splice(i,1)},hasListener:f=>ls.includes(f)},
        onInstalled:ev(),onStartup:ev(),onConnect:ev(),connect:()=>({postMessage(){},onMessage:ev(),onDisconnect:ev(),disconnect(){}}),
        getPlatformInfo:f=>cb(Promise.resolve({os:'win',arch:'x86-64'}),f),openOptionsPage(){},lastError:undefined},
      storage:{local:area('local'),sync:area('sync'),session:area('session'),managed:area('managed'),onChanged:ev()},
      tabs:{query:(q,f)=>cb(rpc('tabs.query',ext,q||{}),f),get:(id,f)=>cb(rpc('tabs.get',ext,{id}),f),create:(p,f)=>cb(rpc('tabs.create',ext,p),f),
        update:(a,b,f)=>cb(rpc('tabs.update',ext,typeof a==='object'?{props:a}:{id:a,props:b}),f),
        sendMessage:(id,msg,o,f)=>cb(rpc('tabs.send',ext,{id,msg}),typeof o==='function'?o:f),onUpdated:ev(),onActivated:ev(),onRemoved:ev()},
      scripting:{executeScript:(o,f)=>cb(rpc('scripting',ext,{tabId:o.target&&o.target.tabId,files:o.files,func:o.func&&String(o.func),args:o.args}),f),insertCSS:()=>Promise.resolve()},
      i18n:{getMessage:(k,s)=>{const m=msgs[k]||msgs[String(k).toLowerCase()]; let t=m?m.message:''; if(s!==undefined)[].concat(s).forEach((x,i)=>t=t.replace('$'+(i+1),x)); return t;},getUILanguage:()=>'de'},
      alarms:{create:(n,i)=>{ if(i&&i.periodInMinutes) setInterval(()=>al.forEach(f=>f({name:n})),i.periodInMinutes*6e4); else if(i&&i.delayInMinutes) setTimeout(()=>al.forEach(f=>f({name:n})),i.delayInMinutes*6e4); },
        onAlarm:{addListener:f=>al.push(f)},clear(){},getAll:f=>cb(Promise.resolve([]),f)},
      action:{setBadgeText(){},setBadgeBackgroundColor(){},setIcon(){},setTitle(){},setPopup(){},onClicked:ev()}
    };
    api.browserAction=api.action;
    return new Proxy(api,{get:(t,k)=>k in t?t[k]:deep()});
  };
})();
"#;
