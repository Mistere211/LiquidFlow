//! Seiten-Schutz (Brave-Shields-Prinzip): globale Standardwerte + Overrides pro Seite.
use crate::settings::{Settings, SiteRule};
use std::collections::HashMap;

pub const START: &str = "liquid://start";

#[derive(Clone, Default)]
pub struct Rules { pub hints: bool, pub interstitial: bool, pub http_ok: std::collections::HashSet<String>, pub cookies: bool, pub adblock: bool, pub fingerprint: bool, pub webrtc: bool, pub https_only: bool, pub strip: bool, pub sites: HashMap<String, SiteRule> }

pub fn norm(h: &str) -> String { h.trim_start_matches("www.").to_ascii_lowercase() }
pub fn host_of(u: &str) -> String { url::Url::parse(u).ok().and_then(|x| x.host_str().map(norm)).unwrap_or_default() }

impl Rules {
    pub fn from(s: &Settings) -> Self {
        Self { hints: s.hints_enabled, interstitial: s.https_interstitial, http_ok: Default::default(), cookies: s.cookie_banners, adblock: s.ad_block, fingerprint: s.fingerprint_protect, webrtc: s.webrtc_protect,
               https_only: s.https_only, strip: s.strip_tracking, sites: s.site_rules.clone() }
    }
    fn site(&self, h: &str) -> Option<&SiteRule> { self.sites.get(h) }
    pub fn adblock(&self, h: &str) -> bool { self.site(h).and_then(|r| r.adblock).unwrap_or(self.adblock) }
    pub fn fingerprint(&self, h: &str) -> bool { self.site(h).and_then(|r| r.fingerprint).unwrap_or(self.fingerprint) }
    pub fn webrtc(&self, h: &str) -> bool { self.site(h).and_then(|r| r.webrtc).unwrap_or(self.webrtc) }
    pub fn https_only(&self, h: &str) -> bool { self.site(h).and_then(|r| r.https_only).unwrap_or(self.https_only) }
    pub fn strip(&self, h: &str) -> bool { self.site(h).and_then(|r| r.strip_tracking).unwrap_or(self.strip) }
    pub fn cookies(&self, h: &str) -> bool { self.site(h).and_then(|r| r.cookies).unwrap_or(self.cookies) }
    pub fn hints(&self, h: &str) -> bool { self.site(h).and_then(|r| r.hints).unwrap_or(self.hints) }
    pub fn dark(&self, h: &str) -> bool { self.site(h).and_then(|r| r.dark).unwrap_or(false) }
    pub fn zoom(&self, h: &str) -> Option<f64> { self.site(h).and_then(|r| r.zoom) }
    fn flag(&self, h: &str, k: &str) -> bool {
        self.site(h).and_then(|r| match k { "block_cam" => r.block_cam, "block_mic" => r.block_mic, "block_geo" => r.block_geo, _ => None }).unwrap_or(false)
    }
    pub fn get(&self, h: &str, k: &str) -> bool {
        match k { "adblock" => self.adblock(h), "fingerprint" => self.fingerprint(h), "webrtc" => self.webrtc(h),
                  "https_only" => self.https_only(h), "strip_tracking" => self.strip(h), "dark" => self.dark(h), "cookies" => self.cookies(h), "hints" => self.hints(h), _ => self.flag(h, k) }
    }
}

fn is_tracker(k: &str) -> bool {
    k.starts_with("utm_") || matches!(k, "fbclid" | "gclid" | "dclid" | "gbraid" | "wbraid" | "msclkid" | "mc_eid" | "igshid" | "yclid" | "twclid" | "ttclid" | "_ga" | "ref_src")
}

/// http-Navigation, die erst geprüft werden soll (HTTPS erreichbar? sonst Warnseite)
pub fn needs_probe(u: &str, r: &Rules) -> bool {
    let Ok(url) = url::Url::parse(u) else { return false };
    let Some(host) = url.host_str().map(norm) else { return false };
    let local = host.ends_with(".onion") || host == "localhost" || host.ends_with(".local") || host.parse::<std::net::IpAddr>().is_ok();
    url.scheme() == "http" && r.interstitial && r.https_only(&host) && !local && !r.http_ok.contains(&host)
}

/// Navigation umschreiben: http -> https, Tracking-Parameter raus. None = unverändert.
pub fn rewrite(u: &str, r: &Rules) -> Option<String> {
    let mut url = url::Url::parse(u).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" { return None; }
    let host = url.host_str().map(norm)?;
    let mut changed = false;
    let local = host.ends_with(".onion") || host == "localhost" || host.ends_with(".local") || host.parse::<std::net::IpAddr>().is_ok();
    if url.scheme() == "http" && r.https_only(&host) && !local && !r.interstitial {
        url.set_scheme("https").ok()?;
        if url.port() == Some(80) { let _ = url.set_port(None); }
        changed = true;
    }
    if r.strip(&host) && url.query().is_some() {
        let all: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let kept: Vec<_> = all.iter().filter(|(k, _)| !is_tracker(k)).cloned().collect();
        if kept.len() != all.len() {
            if kept.is_empty() { url.set_query(None); } else { url.query_pairs_mut().clear().extend_pairs(kept); }
            changed = true;
        }
    }
    changed.then(|| url.to_string())
}

pub fn script(s: &Settings) -> String {
    let cfg = serde_json::json!({ "g": { "fp": s.fingerprint_protect, "webrtc": s.webrtc_protect, "vol": s.volume, "cb": s.cookie_banners, "hints": s.hints_enabled, "hkey": s.hint_key, "hchars": s.hint_chars }, "sites": s.site_rules });
    JS.replace("__CFG__", &cfg.to_string())
}

const JS: &str = r#"
(function(){
  const C=__CFG__, h=location.hostname.replace(/^www\./,'').toLowerCase(), site=C.sites[h]||{};
  const fp=site.fingerprint??C.g.fp, rtc=site.webrtc??C.g.webrtc;
  if(rtc){ for(const k of ['RTCPeerConnection','webkitRTCPeerConnection','RTCDataChannel']) try{ Object.defineProperty(window,k,{value:undefined,configurable:true}); }catch(e){} }
  if(fp){
    const seed=Math.floor(Math.random()*1e9);
    const noise=d=>{ for(let i=0;i<d.length;i+=4) d[i]=(d[i]+((seed>>(i%24))&1))&255; };
    try{
      const gid=CanvasRenderingContext2D.prototype.getImageData;
      CanvasRenderingContext2D.prototype.getImageData=function(...a){ const r=gid.apply(this,a); noise(r.data); return r; };
      const mark=cv=>{ try{ const c=cv.getContext('2d'); if(c&&cv.width&&cv.height){ const im=gid.call(c,0,0,Math.min(cv.width,16),Math.min(cv.height,16)); noise(im.data); c.putImageData(im,0,0);} }catch(e){} };
      const tdu=HTMLCanvasElement.prototype.toDataURL; HTMLCanvasElement.prototype.toDataURL=function(...a){ mark(this); return tdu.apply(this,a); };
      const tb=HTMLCanvasElement.prototype.toBlob; HTMLCanvasElement.prototype.toBlob=function(...a){ mark(this); return tb.apply(this,a); };
    }catch(e){}
    try{ for(const P of [window.WebGLRenderingContext,window.WebGL2RenderingContext]) if(P){ const gp=P.prototype.getParameter;
      P.prototype.getParameter=function(p){ if(p===37445) return 'Google Inc.'; if(p===37446) return 'ANGLE (Generic Renderer)'; return gp.call(this,p); }; } }catch(e){}
    try{ const d=(o,k,v)=>Object.defineProperty(o,k,{get:()=>v,configurable:true});
      d(navigator,'hardwareConcurrency',4); d(navigator,'deviceMemory',8); d(navigator,'languages',['en-US','en']); }catch(e){}
  }
  document.addEventListener('play',e=>{ if(e.target&&e.target.volume!==undefined) e.target.volume=C.g.vol; },true);
  if(site.block_cam||site.block_mic){ const md=navigator.mediaDevices; if(md&&md.getUserMedia){ const g=md.getUserMedia.bind(md);
    md.getUserMedia=c=>((site.block_cam&&c&&c.video)||(site.block_mic&&c&&c.audio))?Promise.reject(new DOMException('Blocked','NotAllowedError')):g(c); } }
  if(site.block_geo&&navigator.geolocation){ const deny=(a,b)=>{ b&&b({code:1,message:'Blocked'}); };
    navigator.geolocation.getCurrentPosition=deny; navigator.geolocation.watchPosition=(a,b)=>{ deny(a,b); return 0; }; }
  if((site.hints??C.g.hints)&&C.g.hkey){
    const CH=(C.g.hchars||'asdfghjkl').toLowerCase(), HK=C.g.hkey; let on=false, boxes=[], buf='', nt=false;
    const editable=e=>{ const t=e&&e.tagName; return t==='INPUT'||t==='TEXTAREA'||t==='SELECT'||(e&&e.isContentEditable); };
    const vis=e=>{ const r=e.getBoundingClientRect(); return r.width>4&&r.height>4&&r.bottom>0&&r.right>0&&r.top<innerHeight&&r.left<innerWidth&&getComputedStyle(e).visibility!=='hidden'; };
    const labels=n=>{ let len=1; while(Math.pow(CH.length,len)<n) len++; const L=[]; for(let i=0;i<n;i++){ let s='',x=i; for(let k=0;k<len;k++){ s=CH[x%CH.length]+s; x=Math.floor(x/CH.length); } L.push(s); } return L; };
    const clear=()=>{ boxes.forEach(b=>b.remove()); boxes=[]; on=false; buf=''; };
    const show=newtab=>{ clear(); nt=newtab;
      const els=[...document.querySelectorAll('a[href],button,input,textarea,select,[role=button],[onclick]')].filter(vis).slice(0,400); const L=labels(els.length);
      els.forEach((e,i)=>{ const r=e.getBoundingClientRect(), d=document.createElement('div'); d.textContent=L[i].toUpperCase();
        d.style.cssText='position:fixed;z-index:2147483647;left:'+Math.max(0,r.left)+'px;top:'+Math.max(0,r.top)+'px;background:#5ad1ff;color:#06101f;font:bold 11px system-ui;padding:1px 4px;border-radius:4px;box-shadow:0 1px 4px #0006;pointer-events:none';
        d._e=e; d._l=L[i]; document.documentElement.appendChild(d); boxes.push(d); });
      on=boxes.length>0; buf=''; };
    addEventListener('keydown',e=>{
      if(on){ e.preventDefault(); e.stopPropagation(); if(e.key==='Escape'){ clear(); return; }
        if(e.key.length===1){ buf+=e.key.toLowerCase(); const m=boxes.filter(b=>b._l.startsWith(buf)); boxes.forEach(b=>b.style.display=m.includes(b)?'':'none');
          if(m.length===1&&m[0]._l===buf){ const t=m[0]._e; clear(); if(t.tagName==='A'&&nt) window.open(t.href); else if(['INPUT','TEXTAREA','SELECT'].includes(t.tagName)) t.focus(); else t.click(); }
          else if(!m.length) clear(); }
        return; }
      if(e.ctrlKey||e.altKey||e.metaKey||editable(document.activeElement)||editable(e.target)) return;
      if(e.key===HK||e.key===HK.toUpperCase()&&HK.toLowerCase()!==HK.toUpperCase()){ e.preventDefault(); show(e.key!==HK); } },true);
  }
  if(site.cookies??C.g.cb){
    const st=document.createElement('style');
    st.textContent='#onetrust-banner-sdk,#onetrust-consent-sdk,#CybotCookiebotDialog,#CybotCookiebotDialogBodyUnderlay,.cc-window,.cc-banner,#cookie-banner,#cookiebanner,#cookie-notice,.cookie-banner,.cookie-notice,#usercentrics-root,[id^="sp_message_container"],.fc-consent-root,#didomi-host,.truste_box_overlay,#qc-cmp2-container{display:none!important}html,body{overflow:auto!important}';
    (document.head||document.documentElement).appendChild(st);
    const rej=['#onetrust-reject-all-handler','#CybotCookiebotDialogBodyButtonDecline','button.fc-cta-do-not-consent','#didomi-notice-disagree-button','.cc-deny','[data-testid="uc-deny-all-button"]'];
    const txt=/^(alle ablehnen|ablehnen|nur notwendige|nur erforderliche|reject all|decline|deny|only necessary|necessary only|refuse)/i;
    let n=0; const tm=setInterval(()=>{
      for(const s of rej){ const b=document.querySelector(s); if(b){ b.click(); clearInterval(tm); return; } }
      for(const b of document.querySelectorAll('[id*=cookie i] button,[class*=cookie i] button,[id*=consent i] button,[class*=consent i] button,[role=dialog] button')){
        if(txt.test((b.textContent||'').trim())){ b.click(); clearInterval(tm); return; } }
      if(++n>20) clearInterval(tm); },500);
  }
  if(site.dark){ const s=document.createElement('style');
    s.textContent='html{filter:invert(1) hue-rotate(180deg)!important;background:#fff}img,video,canvas,picture{filter:invert(1) hue-rotate(180deg)!important}';
    (document.head||document.documentElement).appendChild(s); }
})();
"#;

pub const READER_JS: &str = r#"(function(){
  const src=document.querySelector('article')||document.querySelector('main')||document.body;
  const ps=[...src.querySelectorAll('h1,h2,h3,p,li,blockquote,img')].filter(e=>e.tagName==='IMG'||e.textContent.trim().length>40||/^H/.test(e.tagName));
  const body=ps.map(e=>e.tagName==='IMG'?'<img src="'+e.src+'">':'<'+e.tagName.toLowerCase()+'>'+e.textContent.trim().replace(/</g,'&lt;')+'</'+e.tagName.toLowerCase()+'>').join('');
  document.body.innerHTML='<div style="max-width:700px;margin:40px auto;font:19px/1.7 Georgia,serif;padding:0 20px;color:#222">'+body+'</div>';
  document.body.style.background='#faf6ee'; document.querySelectorAll('img').forEach(i=>i.style.maxWidth='100%');
})();"#;

pub fn theme_bg(name: &str) -> &'static str {
    match name {
        "ocean" => "linear-gradient(135deg,#0f4c75,#0a1628)", "aurora" => "linear-gradient(135deg,#0b3d2e,#1a1040)",
        "sunset" => "linear-gradient(135deg,#4a1942,#1b0a2a)", "graphite" => "linear-gradient(135deg,#1c1f26,#0c0d10)",
        _ => "radial-gradient(circle at 30% 20%,#16305a,#0a0f1e)",
    }
}

pub fn start_html(s: &Settings) -> String {
    use base64::Engine;
    let action = s.search_url("").trim_end_matches('+').rsplit_once('?').map(|x| x.0.to_string()).unwrap_or_default();
    let esc = |x: &str| x.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let dials: String = s.quick_dials.iter().map(|d| format!(
        "<a href=\"{u}\"><img src=\"https://icons.duckduckgo.com/ip3/{h}.ico\" onerror=\"this.style.visibility='hidden'\"><span>{n}</span></a>",
        u = esc(&d.url), h = host_of(&d.url), n = esc(&d.name))).collect();
    let bg = match std::fs::read(crate::data::home().join("bg.img")) {
        Ok(b) if !b.is_empty() => {
            let mime = if b.starts_with(b"\x89PNG") { "image/png" } else if b.len() > 12 && &b[8..12] == b"WEBP" { "image/webp" } else { "image/jpeg" };
            format!("linear-gradient(rgba(0,0,0,.35),rgba(0,0,0,.35)),url(data:{mime};base64,{}) center/cover", base64::engine::general_purpose::STANDARD.encode(b))
        }
        _ => theme_bg(&s.theme).to_string(),
    };
    START_TPL.replace("__BG__", &bg).replace("__ACTION__", &esc(&action)).replace("__DIALS__", &dials)
        .replace("__CITY__", &serde_json::to_string(&s.weather_city).unwrap_or_else(|_| "\"\"".into()))
}

const START_TPL: &str = r#"<!doctype html><meta charset="utf-8"><title>Liquid Flow</title><style>
html,body{height:100%;margin:0;background:__BG__;color:#e8f1ff;font:15px system-ui,sans-serif}
body{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:22px}
.logo{width:56px;height:56px;border-radius:50% 50% 50% 12px;background:linear-gradient(135deg,#5ad1ff,#7b6bff)}
#clock{font-size:64px;font-weight:200;letter-spacing:2px}#date{opacity:.7;margin-top:-14px}#wx{opacity:.8;min-height:20px}
form{width:min(560px,86vw)}input{width:100%;box-sizing:border-box;padding:14px 20px;border-radius:99px;border:1px solid rgba(255,255,255,.18);background:rgba(255,255,255,.1);backdrop-filter:blur(14px);color:inherit;font:inherit;outline:0}
.d{display:flex;gap:14px;flex-wrap:wrap;justify-content:center}.d a{width:92px;padding:14px 6px;border-radius:16px;background:rgba(255,255,255,.08);border:1px solid rgba(255,255,255,.14);backdrop-filter:blur(14px);display:flex;flex-direction:column;align-items:center;gap:8px;color:inherit;text-decoration:none;transition:transform .2s cubic-bezier(.3,1.6,.5,1)}
.d a:hover{transform:translateY(-4px) scale(1.06)}.d img{width:28px;height:28px;border-radius:7px}</style>
<div class="logo"></div><div id="clock"></div><div id="date"></div><div id="wx"></div>
<form action="__ACTION__" method="get"><input name="q" placeholder="Suchen…" autofocus></form><div class="d">__DIALS__</div>
<script>
const tick=()=>{const n=new Date();clock.textContent=n.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});
 const h=n.getHours();date.textContent=(h<5?'Gute Nacht':h<11?'Guten Morgen':h<18?'Guten Tag':'Guten Abend')+' · '+n.toLocaleDateString([], {weekday:'long',day:'numeric',month:'long'});};
tick();setInterval(tick,10000);
const CITY=__CITY__;
if(CITY){fetch('https://geocoding-api.open-meteo.com/v1/search?count=1&name='+encodeURIComponent(CITY)).then(r=>r.json()).then(g=>{const p=g.results&&g.results[0];if(!p)return;
 return fetch('https://api.open-meteo.com/v1/forecast?current_weather=true&latitude='+p.latitude+'&longitude='+p.longitude).then(r=>r.json()).then(w=>{wx.textContent=p.name+': '+Math.round(w.current_weather.temperature)+' °C';});}).catch(()=>{});}
</script>"#;
