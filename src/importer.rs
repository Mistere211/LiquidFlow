//! Import von Lesezeichen und Verlauf aus Chrome, Edge, Brave, Vivaldi und Firefox (Passwörter sind verschlüsselt und werden nicht importiert).
use crate::data::{Bookmark, HistEntry};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind { Chromium, Firefox }
#[derive(Clone)]
pub struct Source { pub id: String, pub name: String, pub dir: PathBuf, pub kind: Kind }

fn has(dir: &Path, f: &str) -> bool { dir.join(f).exists() }

pub fn sources() -> Vec<Source> {
    let mut v = vec![];
    let local = dirs::data_local_dir().unwrap_or_default();
    for (id, name, rel) in [("chrome", "Chrome", "Google/Chrome/User Data"), ("edge", "Edge", "Microsoft/Edge/User Data"),
                            ("brave", "Brave", "BraveSoftware/Brave-Browser/User Data"), ("vivaldi", "Vivaldi", "Vivaldi/User Data")] {
        let base = local.join(rel);
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        let mut profs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir() && (p.file_name().map_or(false, |n| n == "Default" || n.to_string_lossy().starts_with("Profile ")))).collect();
        profs.sort();
        for p in profs.into_iter().filter(|p| has(p, "Bookmarks") || has(p, "History")) {
            let pn = p.file_name().unwrap().to_string_lossy().into_owned();
            v.push(Source { id: format!("{id}:{pn}"), name: format!("{name} ({pn})"), dir: p, kind: Kind::Chromium });
        }
    }
    if let Ok(rd) = std::fs::read_dir(dirs::config_dir().unwrap_or_default().join("Mozilla/Firefox/Profiles")) {
        for p in rd.flatten().map(|e| e.path()).filter(|p| has(p, "places.sqlite")) {
            let pn = p.file_name().unwrap().to_string_lossy().into_owned();
            v.push(Source { id: format!("firefox:{pn}"), name: format!("Firefox ({})", pn.split_once('.').map_or(pn.as_str(), |x| x.1)), dir: p, kind: Kind::Firefox });
        }
    }
    v
}

fn walk(n: &Value, out: &mut Vec<Bookmark>) {
    match n["type"].as_str() {
        Some("url") => if let Some(u) = n["url"].as_str().filter(|u| u.starts_with("http")) { out.push(Bookmark { title: n["name"].as_str().unwrap_or("").into(), url: u.into() }); },
        _ => for c in n["children"].as_array().into_iter().flatten() { walk(c, out); },
    }
}

/// SQLite-Datei (und WAL) in einen Temp-Ordner kopieren, weil der Browser sie gesperrt halten kann.
fn copy_db(src: &Path, name: &str) -> Result<(PathBuf, PathBuf), String> {
    let tmp = std::env::temp_dir().join(format!("lf-import-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let dst = tmp.join(name);
    std::fs::copy(src.join(name), &dst).map_err(|_| "Datei ist gesperrt – bitte den anderen Browser schließen und erneut versuchen".to_string())?;
    let wal = src.join(format!("{name}-wal"));
    if wal.exists() { let _ = std::fs::copy(&wal, tmp.join(format!("{name}-wal"))); }
    Ok((tmp, dst))
}

fn query(db: &Path, sql: &str) -> Result<Vec<(String, String, i64)>, String> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let mut st = c.prepare(sql).map_err(|e| e.to_string())?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0)))).map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

pub fn import(s: &Source, want_bm: bool, want_hist: bool) -> Result<(Vec<Bookmark>, Vec<HistEntry>), String> {
    let (mut bm, mut hist) = (vec![], vec![]);
    match s.kind {
        Kind::Chromium => {
            if want_bm {
                if let Ok(t) = std::fs::read_to_string(s.dir.join("Bookmarks")) {
                    let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
                    for r in v["roots"].as_object().into_iter().flat_map(|o| o.values()) { walk(r, &mut bm); }
                }
            }
            if want_hist && has(&s.dir, "History") {
                let (tmp, db) = copy_db(&s.dir, "History")?;
                let rows = query(&db, "SELECT url, title, last_visit_time FROM urls WHERE url LIKE 'http%' ORDER BY last_visit_time DESC LIMIT 5000");
                let _ = std::fs::remove_dir_all(&tmp);
                for (url, title, t) in rows? { hist.push(HistEntry { url, title, t: (t / 1_000_000 - 11_644_473_600).max(0) as u64 }); }
            }
        }
        Kind::Firefox => {
            let (tmp, db) = copy_db(&s.dir, "places.sqlite")?;
            if want_bm {
                for (url, title, _) in query(&db, "SELECT p.url, b.title, 0 FROM moz_bookmarks b JOIN moz_places p ON b.fk = p.id WHERE b.type = 1 AND p.url LIKE 'http%'")? { bm.push(Bookmark { title, url }); }
            }
            if want_hist {
                for (url, title, t) in query(&db, "SELECT url, title, last_visit_date FROM moz_places WHERE url LIKE 'http%' AND last_visit_date IS NOT NULL ORDER BY last_visit_date DESC LIMIT 5000")? { hist.push(HistEntry { url, title, t: (t / 1_000_000).max(0) as u64 }); }
            }
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
    hist.sort_by_key(|h| h.t); // älteste zuerst, damit die Reihenfolge wie im Verlauf stimmt
    Ok((bm, hist))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chromium_and_firefox() {
        let d = std::env::temp_dir().join("lf-imp-test");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("c")).unwrap();
        std::fs::write(d.join("c/Bookmarks"), r#"{"roots":{"bookmark_bar":{"type":"folder","children":[{"type":"url","name":"A","url":"https://a.com"},{"type":"folder","children":[{"type":"url","name":"B","url":"https://b.com"},{"type":"url","name":"JS","url":"javascript:1"}]}]},"other":{"type":"folder","children":[]}}}"#).unwrap();
        let c = rusqlite::Connection::open(d.join("c/History")).unwrap();
        c.execute_batch("CREATE TABLE urls(url TEXT,title TEXT,last_visit_time INTEGER); INSERT INTO urls VALUES('https://a.com','A',13300000000000000),('chrome://x','X',1);").unwrap();
        drop(c);
        let s = Source { id: "t".into(), name: "t".into(), dir: d.join("c"), kind: Kind::Chromium };
        let (bm, h) = import(&s, true, true).unwrap();
        assert_eq!(bm.len(), 2); assert_eq!(h.len(), 1); assert!(h[0].t > 1_600_000_000);
        std::fs::create_dir_all(d.join("f")).unwrap();
        let c = rusqlite::Connection::open(d.join("f/places.sqlite")).unwrap();
        c.execute_batch("CREATE TABLE moz_places(id INTEGER PRIMARY KEY,url TEXT,title TEXT,last_visit_date INTEGER); CREATE TABLE moz_bookmarks(id INTEGER PRIMARY KEY,type INTEGER,fk INTEGER,title TEXT);
          INSERT INTO moz_places VALUES(1,'https://f.com','F',1700000000000000),(2,'https://g.com','G',NULL); INSERT INTO moz_bookmarks VALUES(1,1,2,'G');").unwrap();
        drop(c);
        let s = Source { id: "f".into(), name: "f".into(), dir: d.join("f"), kind: Kind::Firefox };
        let (bm, h) = import(&s, true, true).unwrap();
        assert_eq!(bm.len(), 1); assert_eq!(h.len(), 1); assert_eq!(h[0].t, 1_700_000_000);
    }
}
