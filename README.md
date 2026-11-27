# Liquid Flow
Windows-Installer bauen: Repo auf GitHub pushen -> Actions -> "Build Windows Installer" -> Artifact `LiquidFlow-Setup` (enthält LiquidFlow-Setup.exe).
Lokal: `cargo run --release`
Extensions (entpackt): `%APPDATA%\LiquidFlow\extensions\<name>\manifest.json`
Voraussetzung auf Windows: WebView2 Runtime (bei Windows 11 schon da).

## Werbeblocker & DNS
Alle Tabs laufen über einen lokalen SOCKS5-Proxy (`net.rs`): Blocklist-Check, optional Tor, eigener DNS (Quad9/Cloudflare/Google/IP).
Listen: Link, TXT, ZIP oder Ordner (Hosts-Format, Domain-Listen, `||domain^`). Cache: `%APPDATA%\LiquidFlow\blocklists\`.

## Seiten-Schutz
Globale Standardwerte in den Einstellungen, Overrides pro Seite über 🛡 oder Strg+K (`shield.rs`):
Werbeblocker (per-Tab-Proxy), Anti-Fingerprinting, WebRTC-Schutz, Nur HTTPS, Tracking-Parameter, Dark-Mode, Zoom.
Weitere Funktionen: privater Tab (🕶), neue Tor-Identität, Lesemodus, Startseite mit Schnellwahl.

## Tastenkürzel & mehr
Alle Kürzel sind in den Einstellungen frei belegbar (`keys.rs`). Standard: Strg+K Palette, Strg+T/W/Shift+T Tabs, Strg+L, Strg+F, Strg+D, Strg+H, Strg+J, F11, F12, Strg+Shift+U neue Tor-Identität.
Enthalten: Verlauf, Lesezeichen, Downloads, Suche auf der Seite, privater Tab, Daten beim Beenden löschen, Akzentfarbe, App-Icon.
Seiten können Tasten nur mit geheimem Token melden (kein Fernsteuern durch Webseiten).

## Neu in 0.3
- **Erweiterungen**: Web-Store-ID/Link, .crx/.zip oder Ordner installieren (Einstellungen). Adapter in `ext.rs` (Content-Scripts, Background, Messaging, storage, tabs, Popup).
- **Passwort-Tresor** (`vault.rs`): Argon2id + ChaCha20-Poly1305, Autofill nur per Tastendruck, nur HTTPS, nur passende Seite.
- **Sync** (`sync.rs`): Ordner-basiert (OneDrive/Dropbox/Syncthing), verschlüsselt mit dem Master-Passwort.
- **Auto-Update** (`update.rs`): GitHub Releases, Tag `v0.x.y` pushen -> Release mit SHA-256; im Browser Repo `owner/name` eintragen.
- **Split-View, Tab-Gruppen, Kamera/Mikro/Standort pro Seite sperren, Drittanbieter-Cookies (experimentell)**.

## Neu in 0.4
Sitzung wiederherstellen, Adressleisten-Vorschläge, Popup-/Autoplay-/Cookie-Banner-Blocker, Standardbrowser (Installer + Einzelinstanz), Leak-Test,
Panik-Taste (Ctrl+Alt+Shift+X), .onion + Bridges, Tor-Kreis pro Tab, Profile (`--profile name`), Themes/Hintergrund/Startseite mit Uhr & Wetter,
Speicher-Anzeige, Tools (Notizen/Rechner), Fokus-Modus (F9), Mausgesten, Screenshot (Windows-Snip-Tool).

## Tor-Route, Bridges, Tab-Vorschau
- **Route**: 🧅-Anzeige in der Leiste (Land je Hop, Tooltip mit IPs) für den aktiven Tab.
- **obfs4/meek/Snowflake**: CI lädt lyrebird.exe + snowflake-client.exe aus dem Tor Expert Bundle nach `pt\` (Installer legt sie neben die exe). Bridge-Zeilen von bridges.torproject.org in den Einstellungen einfügen, dann VPN aus/an.
- **Tab-Vorschau**: Hover über einen Tab zeigt Miniaturbild (WebView2 CapturePreview, Cargo-Feature `previews`; fällt im CI automatisch weg, falls der Windows-Build damit scheitert).

## Installer & Updater (0.5)
- **`LiquidFlowSetup.exe`** (~3 MB) ist der Installer wie bei Chrome/Brave: lädt die aktuelle Version aus dem GitHub-Release, prüft SHA-256, installiert pro Benutzer nach `%LOCALAPPDATA%\LiquidFlow` (ohne Admin), legt Verknüpfungen an, registriert den Browser (Standard-Apps, Deinstallation in den Windows-Einstellungen).
- **Updater**: dieselbe Datei liegt installiert unter `…\LiquidFlow\LiquidFlowSetup.exe` und läuft stündlich + bei jeder Anmeldung still im Hintergrund (`--update --silent`). Neue Versionen landen in `versions\<ver>\`, der Browser zeigt „↻ Neu starten“.
- **Veröffentlichen**: `git tag v0.5.0 && git push origin v0.5.0` → GitHub baut und erstellt das Release mit allen Dateien.
- Kommandozeile: `--silent`, `--dir <Ordner>`, `--uninstall`, `--update`, `--no-desktop`, `--no-updates`.
- **Offline-Installer**: `LiquidFlow-Offline-Setup.exe` (Inno Setup) bleibt als Ausweichlösung.

## Neu in 0.6
- **Gaming-Modus** (Strg+Shift+M): Hintergrund-Tabs schlafen, Erweiterungen und Hintergrundjobs pausiert.
- **Arbeitsbereiche** (Strg+Alt+K): Tabs + Gruppen als Set speichern und wechseln.
- **Tasten-Navigation**: Taste `f` zeigt Link-Hinweise, `F` (Umschalt) öffnet im neuen Tab. Taste und Buchstaben einstellbar, pro Seite abschaltbar.
- **Bild-im-Bild / Video-Tempo**: Alt+P, Alt+] / Alt+[ / Alt+\ (Mini-Player als Ausweichlösung).
- **Vorlesen** (Alt+V / Alt+B): Windows-Stimme oder eigenes TTS-Kommando (liest Text von stdin).
- **Seite ans Handy**: QR-Code (Strg+Alt+Q), optional Link im Sync-Ordner ablegen.
- **Datenschutz-Bericht** (Strg+Alt+P): 7-Tage-Statistik, Top-Tracker, geblockte Domains der aktuellen Seite.
- **HTTPS-Warnseite** mit „Trotzdem öffnen“, **Passwort-Check** (Have I Been Pwned, k-Anonymität), **2FA-Codes (TOTP)** im Tresor (Strg+Shift+K fügt den Code ein).
- **Download-Schutz**: Hash-Abfrage bei VirusTotal (eigener API-Schlüssel, nur der Hash wird gesendet).
- **Import** aus Chrome, Edge, Brave, Vivaldi, Firefox (Lesezeichen + Verlauf), auch im Onboarding.
