//! Tastenkürzel: frei belegbar (Einstellungen). Seiten dürfen nur mit geheimem Token Tasten melden.
use crate::settings::Settings;
use std::collections::HashMap;

pub const ACTIONS: &[(&str, &str, &str)] = &[
    ("palette", "Befehlspalette", "Ctrl+K"), ("new_tab", "Neuer Tab", "Ctrl+T"), ("new_private", "Neuer privater Tab", "Ctrl+Shift+N"),
    ("close_tab", "Tab schließen", "Ctrl+W"), ("reopen_tab", "Geschlossenen Tab öffnen", "Ctrl+Shift+T"),
    ("next_tab", "Nächster Tab", "Ctrl+Tab"), ("prev_tab", "Vorheriger Tab", "Ctrl+Shift+Tab"),
    ("focus_url", "Adressleiste", "Ctrl+L"), ("reload", "Neu laden", "F5"), ("back", "Zurück", "Alt+ArrowLeft"),
    ("forward", "Vorwärts", "Alt+ArrowRight"), ("find", "Auf Seite suchen", "Ctrl+F"),
    ("zoom_in", "Vergrößern", "Ctrl+="), ("zoom_out", "Verkleinern", "Ctrl+-"), ("zoom_reset", "Zoom zurücksetzen", "Ctrl+0"),
    ("bookmark", "Lesezeichen setzen", "Ctrl+D"), ("bookmarks", "Lesezeichen", "Ctrl+Shift+O"), ("history", "Verlauf", "Ctrl+H"),
    ("downloads", "Downloads", "Ctrl+J"), ("shield", "Seiten-Schutz", "Ctrl+Shift+S"), ("settings", "Einstellungen", "Ctrl+,"),
    ("fullscreen", "Vollbild", "F11"), ("devtools", "Entwicklertools", "F12"), ("print", "Drucken", "Ctrl+P"),
    ("reader", "Lesemodus", "Alt+R"), ("dark_toggle", "Dark-Mode für diese Seite", ""), ("dial_add", "Zur Schnellwahl hinzufügen", ""),
    ("clear_data", "Browserdaten löschen", "Ctrl+Shift+Delete"), ("vpn_toggle", "Tor-VPN an/aus", "Ctrl+Shift+V"),
    ("tor_new", "Neue Tor-Identität", "Ctrl+Shift+U"),
    ("split_toggle", "Split-View an/aus", "Ctrl+Shift+E"), ("split_swap", "Split-View tauschen", ""),
    ("tab_group", "Tab-Gruppe wechseln", "Ctrl+Shift+G"), ("vault", "Passwort-Tresor", "Ctrl+Shift+P"),
    ("pw_fill", "Passwort ausfüllen", "Ctrl+Shift+L"), ("pw_save", "Login speichern", ""),
    ("sync_now", "Jetzt synchronisieren", ""), ("panic", "PANIK: alles löschen & beenden", "Ctrl+Alt+Shift+X"),
    ("focus_mode", "Fokus-Modus", "F9"), ("sleep_others", "Hintergrund-Tabs schlafen legen", ""), ("tools", "Werkzeuge (Notizen, Rechner)", "Ctrl+Shift+Y"),
    ("leak", "Leak-Test", ""), ("gaming_toggle", "Gaming-Modus", "Ctrl+Shift+M"), ("workspaces", "Arbeitsbereiche", "Ctrl+Alt+K"),
    ("privacy", "Datenschutz-Bericht", "Ctrl+Alt+P"), ("share_phone", "Seite ans Handy (QR)", "Ctrl+Alt+Q"), ("import", "Import aus anderem Browser", ""),
    ("totp_fill", "2FA-Code einfügen", "Ctrl+Shift+K"), ("tts_read", "Vorlesen", "Alt+V"), ("tts_stop", "Vorlesen stoppen", "Alt+B"),
    ("pip", "Bild-im-Bild", "Alt+P"), ("speed_up", "Video schneller", "Alt+]"), ("speed_down", "Video langsamer", "Alt+["), ("speed_reset", "Video-Tempo normal", "Alt+\\"), ("profiles", "Profile", ""), ("screenshot", "Screenshot (Ausschnitt)", "Ctrl+Shift+X"), ("update_install", "Update installieren", ""),
];

pub fn effective(user: &HashMap<String, String>) -> Vec<(String, String)> {
    ACTIONS.iter().map(|(id, _, d)| (id.to_string(), user.get(*id).cloned().unwrap_or(d.to_string()))).collect()
}

pub fn state(s: &Settings) -> Vec<serde_json::Value> {
    effective(&s.keymap).into_iter().zip(ACTIONS).map(|((id, c), (_, label, _))| serde_json::json!({ "id": id, "label": label, "combo": c })).collect()
}

pub fn script(s: &Settings, token: &str) -> String {
    let mut c: Vec<String> = effective(&s.keymap).into_iter().map(|x| x.1).filter(|c| !c.is_empty()).collect();
    c.push("Ctrl+Shift++".into());
    c.sort(); c.dedup();
    let local: std::collections::BTreeMap<String, &str> = effective(&s.keymap).into_iter().filter(|(_, c)| !c.is_empty())
        .filter_map(|(id, c)| ["pip", "speed_up", "speed_down", "speed_reset"].iter().find(|x| **x == id).map(|x| (c, *x))).collect();
    JS.replace("__LOCAL__", &serde_json::to_string(&local).unwrap()).replace("__GEST__", if s.mouse_gestures { "true" } else { "false" }).replace("__TOKEN__", token).replace("__COMBOS__", &serde_json::to_string(&c).unwrap())
}

const JS: &str = r#"
(function(){
  const post=window.ipc&&window.ipc.postMessage.bind(window.ipc), T='__TOKEN__', B=new Set(__COMBOS__);
  const combo=e=>{ let m=''; if(e.ctrlKey)m+='Ctrl+'; if(e.altKey)m+='Alt+'; if(e.shiftKey)m+='Shift+'; if(e.metaKey)m+='Meta+';
    let k=e.key; if(k.length===1)k=k.toUpperCase(); if(k===' ')k='Space'; return m+k; };
  addEventListener('pointerdown',()=>{ const n=Date.now(); if(n-(window.__lfg||0)>300){ window.__lfg=n; post&&post(T+'|g'); } },true);
  if(__GEST__){ let sx=0,sy=0,down=false,did=false;
    addEventListener('mousedown',e=>{ if(e.button===2){down=true;did=false;sx=e.clientX;sy=e.clientY;} },true);
    addEventListener('mouseup',e=>{ if(e.button===2&&down){ down=false; const dx=e.clientX-sx, dy=e.clientY-sy;
      if(Math.max(Math.abs(dx),Math.abs(dy))>70){ did=true; const a=Math.abs(dx)>Math.abs(dy)?(dx<0?'back':'forward'):(dy>0?'new_tab':'reload'); post&&post(T+'|act:'+a); } } },true);
    addEventListener('contextmenu',e=>{ if(did){ e.preventDefault(); did=false; } },true); }
  const LOCAL=__LOCAL__;
  const mini=v=>{ if(v.__mini!==undefined){ v.style.cssText=v.__mini; delete v.__mini; return; } v.__mini=v.style.cssText;
    [['position','fixed'],['right','16px'],['bottom','16px'],['width','340px'],['height','auto'],['z-index','2147483646'],['border-radius','12px'],['box-shadow','0 8px 40px #000a']].forEach(([k,x])=>v.style.setProperty(k,x,'important')); };
  window.__lfMedia=a=>{
    const vs=[...document.querySelectorAll('video')];
    const v=vs.filter(x=>!x.paused).sort((x,y)=>(y.videoWidth*y.videoHeight)-(x.videoWidth*x.videoHeight))[0]||vs[0]; if(!v) return;
    const toast=t=>{ let d=document.getElementById('__lft'); if(!d){ d=document.createElement('div'); d.id='__lft';
      d.style.cssText='position:fixed;left:16px;bottom:16px;z-index:2147483647;background:#101830cc;color:#e8f1ff;padding:8px 14px;border-radius:12px;font:600 14px system-ui;backdrop-filter:blur(8px)'; document.documentElement.appendChild(d); }
      d.textContent=t; clearTimeout(d._t); d._t=setTimeout(()=>d.remove(),1200); };
    if(a==='pip'){ if(document.pictureInPictureElement){ document.exitPictureInPicture(); return; }
      if(document.pictureInPictureEnabled&&v.requestPictureInPicture) v.requestPictureInPicture().catch(()=>mini(v)); else mini(v); return; }
    v.playbackRate=a==='speed_reset'?1:Math.min(4,Math.max(0.25,Math.round((v.playbackRate+(a==='speed_up'?0.25:-0.25))*100)/100)); toast('Tempo ×'+v.playbackRate);
  };
  addEventListener('keydown',()=>{ const n=Date.now(); if(n-(window.__lfg||0)>300){ window.__lfg=n; post&&post(T+'|g'); } },true);
  addEventListener('keydown',e=>{
    if(['Control','Shift','Alt','Meta'].includes(e.key)) return;
    const c=combo(e); if(LOCAL[c]){ e.preventDefault(); e.stopPropagation(); window.__lfMedia(LOCAL[c]); return; }
    if(B.has(c)&&post){ e.preventDefault(); e.stopPropagation(); post(T+'|'+c); }
  },true);
})();
"#;
