#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
mod core;

use core::*;
use std::path::PathBuf;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

enum Ev { Prog(f32, String), Done(Result<String, String>), Picked(PathBuf), Quit }

#[derive(Default)]
struct Args { silent: bool, update: bool, uninstall: bool, launch: bool, no_desktop: bool, no_updates: bool, defaults: bool,
              dir: Option<PathBuf>, payload: Option<PathBuf>, version: Option<String>, repo: Option<String>, api: Option<String> }

fn parse() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--silent" => a.silent = true, "--update" => a.update = true, "--uninstall" => a.uninstall = true, "--launch" => a.launch = true,
            "--no-desktop" => a.no_desktop = true, "--no-updates" => a.no_updates = true, "--defaults" => a.defaults = true,
            "--dir" => a.dir = it.next().map(PathBuf::from), "--payload" => a.payload = it.next().map(PathBuf::from),
            "--version" => a.version = it.next(), "--repo" => a.repo = it.next(), "--api" => a.api = it.next(),
            _ => {}
        }
    }
    a
}

/// Liegt das Setup im Installationsordner (…\LiquidFlow\LiquidFlowSetup.exe), ist das der Root.
fn installed_root() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.to_path_buf();
    p.join("current.txt").exists().then_some(p)
}

fn main() {
    let a = parse();
    let root = a.dir.clone().or_else(installed_root).unwrap_or_else(default_root);

    if a.update {
        let cfg = load_config(&root);
        let repo = a.repo.clone().filter(|r| !r.is_empty()).or(Some(cfg.repo).filter(|r| !r.is_empty())).unwrap_or_else(|| REPO.to_string());
        match update(&root, &repo, a.api.as_deref()) {
            Ok(Some(v)) => { println!("updated {v}"); if a.launch { launch(&root); } }
            Ok(None) => { println!("up-to-date"); if a.launch { launch(&root); } }
            Err(e) => { let _ = std::fs::write(root.join("update.log"), &e); eprintln!("{e}"); std::process::exit(1); }
        }
        return;
    }

    let opts = Opts { root: root.clone(), desktop: !a.no_desktop, updates: !a.no_updates, defaults: a.defaults,
        repo: a.repo.clone().unwrap_or_default(), payload: a.payload.clone(), version: a.version.clone(), api: a.api.clone() };

    if a.silent {
        let r = if a.uninstall { uninstall(&root, false, &mut |_, _| {}).map(|_| "removed".to_string()) } else { install(&opts, &mut |_, _| {}) };
        match r { Ok(v) => println!("ok {v}"), Err(e) => { eprintln!("{e}"); std::process::exit(1); } }
        return;
    }

    let mode = if a.uninstall { "uninstall" } else { "install" };
    let el = EventLoopBuilder::<Ev>::with_user_event().build();
    let px = el.create_proxy();
    let window = WindowBuilder::new().with_title("Liquid Flow – Setup").with_inner_size(LogicalSize::new(560.0, 460.0)).with_resizable(false).build(&el).unwrap();
    let init = format!("window.MODE={};window.DEF_DIR={};", serde_json::to_string(mode).unwrap(), serde_json::to_string(&root.to_string_lossy()).unwrap());
    let p = px.clone();
    let (a_payload, a_version, a_repo, a_api) = (a.payload.clone(), a.version.clone(), a.repo.clone().unwrap_or_default(), a.api.clone());
    let wv = WebViewBuilder::new().with_html(include_str!("ui.html")).with_initialization_script(&init)
        .with_ipc_handler(move |r| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(r.body()) else { return };
            let p = p.clone();
            match v["t"].as_str().unwrap_or("") {
                "install" => {
                    let o = Opts { root: PathBuf::from(v["dir"].as_str().unwrap_or("")), desktop: v["desktop"].as_bool().unwrap_or(true), updates: v["updates"].as_bool().unwrap_or(true),
                        defaults: v["defaults"].as_bool().unwrap_or(false), repo: a_repo.clone(), payload: a_payload.clone(), version: a_version.clone(), api: a_api.clone() };
                    std::thread::spawn(move || {
                        let pp = p.clone();
                        let r = install(&o, &mut |f, m| { let _ = pp.send_event(Ev::Prog(f, m.to_string())); });
                        let _ = p.send_event(Ev::Done(r));
                    });
                }
                "uninstall" => {
                    let (root, data) = (PathBuf::from(v["dir"].as_str().unwrap_or("")), v["data"].as_bool().unwrap_or(false));
                    std::thread::spawn(move || {
                        let pp = p.clone();
                        let r = uninstall(&root, data, &mut |f, m| { let _ = pp.send_event(Ev::Prog(f, m.to_string())); }).map(|_| "removed".to_string());
                        let _ = p.send_event(Ev::Done(r));
                    });
                }
                "pick" => { std::thread::spawn(move || { if let Some(d) = rfd::FileDialog::new().pick_folder() { let _ = p.send_event(Ev::Picked(d.join("LiquidFlow"))); } }); }
                "launch" => { launch(&PathBuf::from(v["dir"].as_str().unwrap_or(""))); let _ = p.send_event(Ev::Quit); }
                "close" => { let _ = p.send_event(Ev::Quit); }
                _ => {}
            }
        }).build(&window).unwrap();

    el.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Prog(f, m)) => { let _ = wv.evaluate_script(&format!("setProgress({f},{})", serde_json::to_string(&m).unwrap())); }
            Event::UserEvent(Ev::Done(Ok(v))) => { let _ = wv.evaluate_script(&format!("showDone({})", serde_json::to_string(&v).unwrap())); }
            Event::UserEvent(Ev::Done(Err(e))) => { let _ = wv.evaluate_script(&format!("showError({})", serde_json::to_string(&e).unwrap())); }
            Event::UserEvent(Ev::Picked(d)) => { let _ = wv.evaluate_script(&format!("setDir({})", serde_json::to_string(&d.to_string_lossy()).unwrap())); }
            Event::UserEvent(Ev::Quit) | Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
