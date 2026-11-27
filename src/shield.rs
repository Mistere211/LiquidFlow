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
        "ocean" => "#101a20", "aurora" => "#111c19",
        "sunset" => "#201719", "graphite" => "#11161d",
        _ => "#11161d",
    }
}

pub fn start_html(s: &Settings) -> String {
    use base64::Engine;
    let action = s.search_url("").trim_end_matches('+').rsplit_once('?').map(|x| x.0.to_string()).unwrap_or_default();
    let esc = |x: &str| x.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let dials: String = s.quick_dials.iter().map(|d| format!(
        "<a href=\"{u}\" title=\"{host}\"><span class=\"dial-mark\" aria-hidden=\"true\">{mark}</span><span class=\"dial-name\">{n}</span><span class=\"dial-host\">{host}</span></a>",
        u = esc(&d.url), host = esc(&host_of(&d.url)), mark = esc(&d.name.chars().next().unwrap_or('S').to_uppercase().to_string()), n = esc(&d.name))).collect();
    let bg = match std::fs::read(crate::data::home().join("bg.img")) {
        Ok(b) if !b.is_empty() => {
            let mime = if b.starts_with(b"\x89PNG") { "image/png" } else if b.len() > 12 && &b[8..12] == b"WEBP" { "image/webp" } else { "image/jpeg" };
            format!("url(data:{mime};base64,{}) center/cover", base64::engine::general_purpose::STANDARD.encode(b))
        }
        _ => theme_bg(&s.theme).to_string(),
    };
    START_TPL.replace("__BG__", &bg).replace("__ACTION__", &esc(&action)).replace("__DIALS__", &dials)
        .replace("__CITY__", &serde_json::to_string(&s.weather_city).unwrap_or_else(|_| "\"\"".into()))
}

const START_TPL: &str = r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Liquid Flow · Neuer Tab</title><style>
:root{color-scheme:dark;--page:#11161d;--surface:#191f26;--line:#303a43;--text:#e5e9ec;--muted:#9ba6af;--quiet:#77838d;--accent:#70c8bd}
*{box-sizing:border-box}html,body{min-height:100%;margin:0;background:__BG__;color:var(--text);font:14px/1.5 "Segoe UI Variable","Segoe UI",system-ui,sans-serif}
body{min-height:100vh;display:flex;justify-content:center;background-color:var(--page);background-position:center;background-size:cover;background-attachment:fixed}
body::before{content:"";position:fixed;inset:0;background:rgba(8,12,16,.48);pointer-events:none}
.page{position:relative;z-index:1;width:min(820px,calc(100% - 64px));min-height:100vh;padding:54px 0 42px;display:flex;flex-direction:column}
header{display:flex;align-items:center;justify-content:space-between;gap:16px;padding-bottom:24px;border-bottom:1px solid rgba(180,195,202,.13)}
.identity{display:flex;align-items:center;gap:10px;font-size:11px;letter-spacing:.14em;font-weight:650;color:#d9e0e2}
.mark{width:25px;height:25px;display:grid;place-items:center;border:1px solid rgba(112,200,189,.5);border-radius:6px;color:var(--accent);background:#1b3030}
.mark svg{width:16px;height:16px;fill:none;stroke:currentColor;stroke-width:1.6;stroke-linecap:round;stroke-linejoin:round}
#wx{color:var(--muted);font-size:12px;min-height:18px;text-align:right}
main{width:min(640px,100%);margin:clamp(54px,13vh,124px) auto 0}
.time{display:flex;align-items:baseline;gap:14px}
#clock{font-size:clamp(38px,5vw,54px);line-height:1;font-weight:350;letter-spacing:-.055em;font-variant-numeric:tabular-nums;color:#f0f3f4}
#date{color:var(--muted);font-size:13px}
.welcome{margin:21px 0 25px;color:#aab4bb;font-size:15px}
form{display:flex;align-items:center;width:100%;height:48px;padding:0 12px;gap:10px;background:var(--surface);border:1px solid var(--line);border-radius:7px;transition:border-color 150ms ease,box-shadow 150ms ease}
form:focus-within{border-color:var(--accent);box-shadow:0 0 0 3px rgba(112,200,189,.14)}
form svg{width:17px;height:17px;flex:none;fill:none;stroke:var(--muted);stroke-width:1.7;stroke-linecap:round;stroke-linejoin:round}
input{width:100%;height:100%;padding:0;border:0;background:transparent;color:var(--text);font:inherit;outline:0}
input::placeholder{color:var(--quiet)}form kbd{font:11px "Cascadia Code",Consolas,monospace;color:var(--quiet);white-space:nowrap}
.shortcuts{margin-top:42px}
.section-title{margin:0 0 12px;color:#8e9ba3;font-size:10px;font-weight:650;letter-spacing:.13em;text-transform:uppercase}
.d{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));border-top:1px solid rgba(180,195,202,.14);border-left:1px solid rgba(180,195,202,.14)}
.d a{min-width:0;min-height:72px;padding:11px 13px;display:grid;grid-template-columns:30px minmax(0,1fr);grid-template-rows:1fr 1fr;column-gap:10px;align-items:center;border-right:1px solid rgba(180,195,202,.14);border-bottom:1px solid rgba(180,195,202,.14);color:inherit;text-decoration:none;transition:background 120ms ease}
.d a:hover,.d a:focus-visible{background:rgba(112,200,189,.07);outline:0}
.dial-mark{grid-row:1/3;display:grid;place-items:center;width:29px;height:29px;border:1px solid #3c4b50;border-radius:6px;background:#1a2529;color:var(--accent);font-size:12px;font-weight:650}
.dial-name{align-self:end;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px}
.dial-host{align-self:start;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--quiet);font:10px "Cascadia Code",Consolas,monospace}
footer{margin-top:auto;padding-top:38px;color:#718089;font-size:11px}
@media(max-width:620px){.page{width:calc(100% - 32px);padding-top:26px}main{margin-top:clamp(46px,10vh,90px)}.d{grid-template-columns:repeat(2,minmax(0,1fr))}.d a{min-height:64px}.shortcuts{margin-top:32px}}
@media(prefers-reduced-motion:reduce){*,*::before,*::after{transition:none!important;scroll-behavior:auto!important}}
</style>
<div class="page"><header><div class="identity"><span class="mark" aria-hidden="true"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8"/><path d="M4 12h16M12 4a13 13 0 0 1 0 16M12 4a13 13 0 0 0 0 16"/></svg></span><span>LIQUID FLOW</span></div><div id="wx" aria-live="polite"></div></header>
<main><div class="time"><div id="clock" aria-label="Uhrzeit"></div><div id="date"></div></div><p class="welcome">Was möchtest du heute entdecken?</p>
<form action="__ACTION__" method="get" role="search"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></svg><input name="q" aria-label="Im Web suchen" placeholder="Im Web suchen oder Adresse eingeben" autofocus><kbd>Enter</kbd></form>
<section class="shortcuts" aria-label="Schnellzugriffe"><h2 class="section-title">Schnellzugriffe</h2><div class="d">__DIALS__</div></section></main><footer>Ein ruhiger Ort für deinen nächsten Schritt.</footer></div>
<script>
const tick=()=>{const n=new Date();clock.textContent=n.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});
 const h=n.getHours();date.textContent=(h<5?'Gute Nacht':h<11?'Guten Morgen':h<18?'Guten Tag':'Guten Abend')+' · '+n.toLocaleDateString([], {weekday:'long',day:'numeric',month:'long'});};
tick();setInterval(tick,10000);
const CITY=__CITY__;
if(CITY){fetch('https://geocoding-api.open-meteo.com/v1/search?count=1&name='+encodeURIComponent(CITY)).then(r=>r.json()).then(g=>{const p=g.results&&g.results[0];if(!p)return;
 return fetch('https://api.open-meteo.com/v1/forecast?current_weather=true&latitude='+p.latitude+'&longitude='+p.longitude).then(r=>r.json()).then(w=>{wx.textContent=p.name+': '+Math.round(w.current_weather.temperature)+' °C';});}).catch(()=>{});}
</script>"#;
