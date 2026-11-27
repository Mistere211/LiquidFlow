//! Lokaler SOCKS5-Proxy (127.0.0.1:<zufälliger Port>), den ALLE Tabs nutzen.
//! Pro Verbindung: 1) Blocklist-Check (Domain + Parent-Domains, O(1) Lookups)
//! 2) VPN an -> über Tor (Arti), Kill-Switch: Tor nicht bereit -> Verbindung verweigern
//! 3) sonst direkt, mit System-DNS oder eigenem DNS (Quad9/Cloudflare/Google/IP) per hickory.
use arti_client::{StreamPrefs, TorClient, TorClientConfig};
use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::TokioAsyncResolver;
use std::collections::HashSet;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tor_geoip::CountryCode;
use tor_rtcompat::PreferredRuntime;

type Tor = TorClient<PreferredRuntime>;
const OK: [u8; 10] = [5, 0, 0, 1, 0, 0, 0, 0, 0, 0];
fn fail(code: u8) -> [u8; 10] { [5, code, 0, 1, 0, 0, 0, 0, 0, 0] }

pub struct Shared {
    blocklist: RwLock<Arc<HashSet<String>>>,
    dns_cfg: Mutex<(String, String)>,
    resolver: Mutex<Option<((String, String), Arc<TokioAsyncResolver>)>>,
    tor: RwLock<Option<Tor>>,
    vpn_on: AtomicBool,
    kill: AtomicBool,
    connecting: AtomicBool,
    country: Mutex<String>,
    pub blocked: AtomicU64,
    stats: Mutex<Stats>,
    pub stats_on: AtomicBool,
    tor_gen: AtomicU64,
    bridges: Mutex<String>,
    status: Box<dyn Fn(&str) + Send + Sync>,
}

#[derive(Clone)]
pub struct Net { pub s: Arc<Shared>, handle: Handle }

/// Pro-Tab-Zustand, den der Proxy dieses Tabs liest (Brave-Shields-Prinzip: Regel gilt für die Seite, nicht die Ziel-Domain).
/// Blockier-Statistik (Tage seit 1970 -> Anzahl, Domains -> Anzahl), wird im Profil gespeichert
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
#[serde(default)]
pub struct Stats { pub daily: std::collections::BTreeMap<u64, u64>, pub top: std::collections::HashMap<String, u64> }

pub struct TabFlags { pub blocked: Mutex<std::collections::HashMap<String, u32>>, pub route: Mutex<Vec<String>>, pub adblock: AtomicBool, pub isolate: AtomicBool, iso: Mutex<Option<(u64, Tor)>>, generation: AtomicU64 }
impl TabFlags { pub fn new() -> Arc<Self> { Arc::new(Self { blocked: Mutex::new(Default::default()), route: Mutex::new(vec![]), adblock: AtomicBool::new(true), isolate: AtomicBool::new(false), iso: Mutex::new(None), generation: AtomicU64::new(0) }) } }

impl Net {
    pub fn start(status: impl Fn(&str) + Send + Sync + 'static) -> Net {
        let s = Arc::new(Shared {
            blocklist: RwLock::new(Arc::new(HashSet::new())),
            dns_cfg: Mutex::new(("system".into(), String::new())),
            resolver: Mutex::new(None),
            tor: RwLock::new(None),
            vpn_on: AtomicBool::new(false), kill: AtomicBool::new(true), connecting: AtomicBool::new(false),
            country: Mutex::new("de".into()), blocked: AtomicU64::new(0), stats: Mutex::new(Stats::default()), stats_on: AtomicBool::new(true), tor_gen: AtomicU64::new(0), bridges: Mutex::new(String::new()), status: Box::new(status),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
            tx.send(rt.handle().clone()).unwrap();
            rt.block_on(std::future::pending::<()>());
        });
        Net { s, handle: rx.recv().unwrap() }
    }

    /// Eigener lokaler Proxy-Port für einen Tab. Endet, wenn der Tab geschlossen oder neu erzeugt wird.
    pub fn listener(&self, flags: &Arc<TabFlags>) -> u16 {
        let std_l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let _ = std_l.set_nonblocking(true);
        let port = std_l.local_addr().unwrap().port();
        let my = flags.generation.fetch_add(1, Relaxed) + 1;
        let (weak, s) = (Arc::downgrade(flags), self.s.clone());
        self.handle.spawn(async move {
            let Ok(l) = TcpListener::from_std(std_l) else { return };
            loop {
                match tokio::time::timeout(std::time::Duration::from_secs(10), l.accept()).await {
                    Ok(Ok((c, _))) => {
                        let Some(f) = weak.upgrade() else { break };
                        if f.generation.load(Relaxed) != my { break; }
                        let s = s.clone();
                        tokio::spawn(async move { let _ = handle(c, s, f).await; });
                    }
                    Ok(Err(_)) => {}
                    Err(_) => match weak.upgrade() { Some(f) if f.generation.load(Relaxed) == my => {}, _ => break },
                }
            }
        });
        port
    }

    /// Neue Tor-Identität: neue Circuits für alle folgenden Verbindungen.
    pub fn new_identity(&self) {
        let c = self.s.tor.read().unwrap().clone();
        if let Some(c) = c { *self.s.tor.write().unwrap() = Some(c.isolated_client()); self.s.tor_gen.fetch_add(1, Relaxed); }
    }

    pub fn set_bridges(&self, b: &str) { *self.s.bridges.lock().unwrap() = b.to_string(); }

    pub fn set_blocklist(&self, set: HashSet<String>) { *self.s.blocklist.write().unwrap() = Arc::new(set); }
    pub fn stats_snapshot(&self) -> Stats { self.s.stats.lock().unwrap().clone() }
    pub fn load_stats(&self, st: Stats) { *self.s.stats.lock().unwrap() = st; }
    pub fn set_dns(&self, mode: &str, custom: &str) { *self.s.dns_cfg.lock().unwrap() = (mode.into(), custom.into()); }

    pub fn set_vpn(&self, on: bool, kill: bool, country: &str) {
        let s = &self.s;
        s.kill.store(kill, Relaxed);
        *s.country.lock().unwrap() = country.into();
        s.vpn_on.store(on, Relaxed);
        if !on { *s.tor.write().unwrap() = None; (s.status)("off"); return; }
        if s.tor.read().unwrap().is_some() { (s.status)("connected"); return; }
        if s.connecting.swap(true, Relaxed) { return; }
        s.connecting.store(false, Relaxed);
        let s2 = s.clone();
        self.handle.spawn(boot(s2));
    }
}

fn build_cfg(s: &Shared) -> Result<TorClientConfig, String> {
    let lines = s.bridges.lock().unwrap().clone();
    let mut b = TorClientConfig::builder();
    b.address_filter().allow_onion_addrs(true);
    // Pluggable Transports (obfs4/meek über lyrebird, snowflake): Programme liegen im Installationsordner unter pt\
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("pt"))) {
        let exe = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_string() };
        let mut add = |bin: String, names: &[&str]| -> Result<(), String> {
            let p = dir.join(bin);
            if !p.exists() { return Ok(()); }
            let mut t = arti_client::config::pt::TransportConfigBuilder::default();
            t.protocols(names.iter().filter_map(|n| n.parse().ok()).collect::<Vec<tor_linkspec::PtTransportName>>());
            t.path(arti_client::config::CfgPath::new(p.to_string_lossy().into_owned()));
            t.run_on_startup(true);
            b.bridges().transports().push(t);
            Ok(())
        };
        add(exe("lyrebird"), &["obfs4", "meek_lite", "scramblesuit"])?;
        add(exe("snowflake-client"), &["snowflake"])?;
    }
    for l in lines.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let br: arti_client::config::BridgeConfigBuilder = l.trim_start_matches("Bridge ").parse().map_err(|e| format!("Bridge: {e}"))?;
        b.bridges().bridges().push(br);
    }
    b.build().map_err(|e| e.to_string())
}

async fn boot(s: Arc<Shared>) {
    if s.connecting.swap(true, Relaxed) { return; }
    if s.vpn_on.load(Relaxed) { (s.status)("connecting"); }
    match build_cfg(&s) {
        Ok(cfg) => match TorClient::create_bootstrapped(cfg).await {
            Ok(c) => { *s.tor.write().unwrap() = Some(c); s.tor_gen.fetch_add(1, Relaxed); if s.vpn_on.load(Relaxed) { (s.status)("connected"); } }
            Err(_) => (s.status)("error"),
        },
        Err(_) => (s.status)("error"),
    }
    s.connecting.store(false, Relaxed);
}

/// .onion ohne VPN: Tor bei Bedarf starten und bis zu 90 s warten
async fn boot_wait(s: &Arc<Shared>) {
    if !s.connecting.load(Relaxed) && s.tor.read().unwrap().is_none() { tokio::spawn(boot(s.clone())); }
    for _ in 0..180 {
        if s.tor.read().unwrap().is_some() { return; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// Route (Land + IP je Hop) des Tor-Kreises merken, damit die Oberfläche sie anzeigen kann.
fn record_route(f: &TabFlags, up: &arti_client::DataStream) {
    use tor_linkspec::HasAddrs;
    use tor_proto::stream::ClientStreamCtrl;
    static DB: std::sync::OnceLock<Arc<tor_geoip::GeoipDb>> = std::sync::OnceLock::new();
    let Some(circ) = up.client_stream_ctrl().and_then(|c| c.circuit()) else { return };
    let db = DB.get_or_init(tor_geoip::GeoipDb::new_embedded);
    let mut hops = vec![];
    for h in circ.path_ref().iter() {
        if let Some(a) = h.as_chan_target().and_then(|t| t.addrs().first().copied()) {
            let cc = db.lookup_country_code(a.ip()).map(|c| c.to_string().to_uppercase()).unwrap_or_else(|| "??".into());
            hops.push(format!("{cc} {}", a.ip()));
        }
    }
    if !hops.is_empty() { *f.route.lock().unwrap() = hops; }
}

fn iso_client(s: &Shared, f: &TabFlags, base: Tor) -> Tor {
    let g = s.tor_gen.load(Relaxed);
    let mut m = f.iso.lock().unwrap();
    if let Some((gg, c)) = &*m { if *gg == g { return c.clone(); } }
    let c = base.isolated_client();
    *m = Some((g, c.clone()));
    c
}

fn blocked(set: &HashSet<String>, host: &str) -> Option<String> {
    if set.is_empty() { return None; }
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    let mut h: &str = &lower;
    loop {
        if set.contains(h) { return Some(h.to_string()); }
        match h.find('.') { Some(i) => h = &h[i + 1..], None => return None }
    }
}

fn resolver(s: &Shared, cfg: &(String, String)) -> Arc<TokioAsyncResolver> {
    let mut g = s.resolver.lock().unwrap();
    if let Some((k, r)) = &*g { if k == cfg { return r.clone(); } }
    let group = match cfg.0.as_str() {
        "quad9" => NameServerConfigGroup::quad9_https(),
        "cloudflare" => NameServerConfigGroup::cloudflare_https(),
        "google" => NameServerConfigGroup::google_https(),
        _ => {
            let ip: IpAddr = cfg.1.trim().parse().unwrap_or(IpAddr::from([9, 9, 9, 9]));
            NameServerConfigGroup::from_ips_clear(&[ip], 53, true)
        }
    };
    let r = Arc::new(TokioAsyncResolver::tokio(ResolverConfig::from_parts(None, vec![], group), ResolverOpts::default()));
    *g = Some((cfg.clone(), r.clone()));
    r
}

async fn direct(s: &Shared, host: &str, port: u16) -> io::Result<TcpStream> {
    let cfg = s.dns_cfg.lock().unwrap().clone();
    if cfg.0 == "system" || host.parse::<IpAddr>().is_ok() { return TcpStream::connect((host, port)).await; }
    let r = resolver(s, &cfg);
    let ips = r.lookup_ip(host).await.map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let mut last = io::Error::from(io::ErrorKind::NotFound);
    for ip in ips.iter().take(4) {
        match TcpStream::connect((ip, port)).await { Ok(c) => return Ok(c), Err(e) => last = e }
    }
    Err(last)
}

async fn handle(mut c: TcpStream, s: Arc<Shared>, flags: Arc<TabFlags>) -> io::Result<()> {
    let mut h = [0u8; 2];
    c.read_exact(&mut h).await?;
    let mut m = vec![0u8; h[1] as usize];
    c.read_exact(&mut m).await?;
    c.write_all(&[5, 0]).await?;
    let mut r = [0u8; 4];
    c.read_exact(&mut r).await?;
    let host = match r[3] {
        1 => { let mut a = [0u8; 4]; c.read_exact(&mut a).await?; Ipv4Addr::from(a).to_string() }
        3 => {
            let mut l = [0u8; 1]; c.read_exact(&mut l).await?;
            let mut d = vec![0u8; l[0] as usize]; c.read_exact(&mut d).await?;
            String::from_utf8_lossy(&d).into_owned()
        }
        4 => { let mut a = [0u8; 16]; c.read_exact(&mut a).await?; Ipv6Addr::from(a).to_string() }
        _ => return Ok(()),
    };
    let mut p = [0u8; 2];
    c.read_exact(&mut p).await?;
    let port = u16::from_be_bytes(p);

    let set = s.blocklist.read().unwrap().clone();
    if let Some(dom) = flags.adblock.load(Relaxed).then(|| blocked(&set, &host)).flatten() {
        s.blocked.fetch_add(1, Relaxed);
        *flags.blocked.lock().unwrap().entry(dom.clone()).or_default() += 1;
        if s.stats_on.load(Relaxed) {
            let day = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86400).unwrap_or(0);
            let mut st = s.stats.lock().unwrap();
            *st.daily.entry(day).or_default() += 1;
            *st.top.entry(dom).or_default() += 1;
            while st.daily.len() > 60 { let k = *st.daily.keys().next().unwrap(); st.daily.remove(&k); }
            if st.top.len() > 800 { st.top.retain(|_, n| *n > 1); }
        }
        c.write_all(&fail(2)).await?;
        return Ok(());
    }

    let onion = host.ends_with(".onion");
    if onion || s.vpn_on.load(Relaxed) {
        let mut tor = s.tor.read().unwrap().clone();
        if tor.is_none() && onion { boot_wait(&s).await; tor = s.tor.read().unwrap().clone(); }
        match tor {
            Some(t) => {
                let t = if flags.isolate.load(Relaxed) { iso_client(&s, &flags, t) } else { t };
                let cc = s.country.lock().unwrap().clone();
                let mut prefs = StreamPrefs::new();
                if !onion { if let Ok(code) = cc.parse::<CountryCode>() { prefs.exit_country(code); } }
                prefs.connect_to_onion_services(arti_client::config::BoolOrAuto::Explicit(true));
                match t.connect_with_prefs((host.as_str(), port), &prefs).await {
                    Ok(mut up) => { record_route(&flags, &up); c.write_all(&OK).await?; let _ = tokio::io::copy_bidirectional(&mut c, &mut up).await; }
                    Err(_) => { c.write_all(&fail(5)).await?; }
                }
                return Ok(());
            }
            None if onion || s.kill.load(Relaxed) => { c.write_all(&fail(5)).await?; return Ok(()); } // Kill-Switch
            None => {}
        }
    }
    match direct(&s, &host, port).await {
        Ok(mut up) => { c.write_all(&OK).await?; let _ = tokio::io::copy_bidirectional(&mut c, &mut up).await; }
        Err(_) => { c.write_all(&fail(4)).await?; }
    }
    Ok(())
}
