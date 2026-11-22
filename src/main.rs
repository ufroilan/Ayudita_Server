#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod autostart;
mod config;
mod conn;
mod dev;
mod escpos;
mod icon;
mod model;
mod printer;
mod realtime;
mod session;
mod state;
mod store;
mod ticket;
mod tray;
mod ui;
mod worker;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use crate::config::{Config, Paths};
use crate::printer::SimFail;
use crate::state::{Dev, SessionInfo, Shared};
use crate::store::Store;

const HELP: &str = "\
Ayudita Comandas: imprime las comandas de Ayudita en una impresora térmica.

Uso: ayudita-comandas [opciones]

  --minimizado               Arranca escondida en la bandeja (lo usa el arranque automático).

Desarrollo:
  --sin-impresora            Enciende la impresora virtual (como en Ajustes → Desarrollo, donde se
                             apaga): guarda cada comanda como .bin (ESC/POS) y .txt en la
                             carpeta «salida» de los datos de la app.
                             También con AYUDITA_SIN_IMPRESORA=1.
  --simular-fallo=TIPO       La impresora virtual falla:
                             sin-papel | tapa-abierta | desconectada
  --comanda-demo             Mete comandas de ejemplo (nueva, cortesía, cuenta y una anulación).
  --ayudita=URL              Activa el modo desarrollo con esa dirección de Ayudita
                             (p. ej. http://localhost:3000). Se guarda; se desactiva en Ajustes → Desarrollo.

Variables de entorno de desarrollo:
  AYUDITA_DATOS=carpeta              Guarda configuración, cola y sesión en esa carpeta.
  AYUDITA_SESION_EN_ARCHIVO=1        Guarda las cookies en un archivo en vez del llavero.
  AYUDITA_CAPTURA=archivo.png        Guarda una captura de la ventana a los 3 s.
  AYUDITA_CAPTURA_SALIR=1            …y cierra la app después.
  RUST_LOG=debug                     Más registro en la consola.

La pestaña Desarrollo de Ajustes solo aparece al pulsar Option + D (Alt + D en Windows y Linux).
Al salir de la app se borran los archivos temporales (las comandas de la impresora virtual).
";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .find_map(|a| a.strip_prefix(&format!("{name}=")).map(str::to_string))
    };
    if flag("--help") || flag("-h") {
        print!("{HELP}");
        return;
    }
    let no_printer =
        flag("--sin-impresora") || std::env::var("AYUDITA_SIN_IMPRESORA").is_ok_and(|v| v == "1");
    let fail = match value("--simular-fallo") {
        None => None,
        Some(v) => match SimFail::parse(&v) {
            Some(f) => Some(f),
            None => {
                eprintln!(
                    "--simular-fallo no reconoce «{v}». Usa: sin-papel, tapa-abierta o desconectada."
                );
                std::process::exit(2);
            }
        },
    };

    let paths = Paths::new();
    // En macOS el bloqueo es un archivo: va en la carpeta de datos, no en la carpeta actual.
    let instance_name = if cfg!(target_os = "macos") {
        paths.data_dir.join("instancia.lock").display().to_string()
    } else {
        format!(
            "ayudita-comandas-{:x}",
            fxhash(&paths.data_dir.display().to_string())
        )
    };
    let instance = single_instance::SingleInstance::new(&instance_name).ok();
    if instance.as_ref().is_some_and(|i| !i.is_single()) {
        eprintln!("Ayudita Comandas ya está abierta (mira en la bandeja del sistema).");
        return;
    }

    let mut cfg = Config::load(&paths.config);
    // `--sin-impresora` enciende la impresora virtual igual que el interruptor de Ajustes, así que
    // se puede apagar desde ahí.
    if no_printer && !cfg.dev_virtual_printer {
        cfg.dev_virtual_printer = true;
        cfg.save(&paths.config);
    }
    if let Some(url) = value("--ayudita") {
        cfg.dev_mode = true;
        cfg.dev_base_url = url.trim().trim_end_matches('/').to_string();
        cfg.save(&paths.config);
    }
    let store = Store::load(&paths.store);
    let session: SessionInfo = std::fs::read(&paths.session)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let dev = Dev {
        fail: Mutex::new(fail),
    };
    let shared = Arc::new(Shared::new(paths, cfg.clone(), store, session, dev, tx));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    rt.spawn(conn::run(shared.clone(), rx));
    rt.spawn(worker::run_printer(shared.clone()));
    rt.spawn(worker::run_poller(shared.clone()));
    rt.spawn(worker::refresh_printers(shared.clone()));
    if flag("--comanda-demo") {
        let s = shared.clone();
        rt.spawn(async move {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            dev::inject(s);
        });
    }
    if cfg.autostart
        && !no_printer
        && let Err(e) = autostart::apply(true)
    {
        log::warn!("No se pudo activar el arranque automático: {e}");
    }

    let (w, h, rgba) = icon::rgba(128);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ayudita Comandas")
            .with_app_id("ayudita-comandas")
            .with_inner_size([800.0, 700.0])
            .with_min_inner_size([720.0, 600.0])
            .with_icon(egui::IconData {
                rgba,
                width: w,
                height: h,
            }),
        ..Default::default()
    };
    let opts = ui::Options {
        start_hidden: flag(autostart::HIDDEN_ARG),
        screenshot: std::env::var_os("AYUDITA_CAPTURA").map(PathBuf::from),
    };
    let handle = rt.handle().clone();
    let app_shared = shared.clone();
    let result = eframe::run_native(
        "Ayudita Comandas",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, app_shared, handle, opts)))),
    );
    if let Err(e) = result {
        log::error!("No se pudo abrir la ventana: {e}");
    }
    shared.store().save();
    rt.shutdown_timeout(Duration::from_secs(2));
    drop(instance);
    clean_temp_files(&shared.paths);
}

/// Al salir se borra lo temporal: las comandas de la impresora virtual, los `.tmp` de las
/// escrituras atómicas y el bloqueo de instancia. La cola, los ajustes y la sesión se conservan.
fn clean_temp_files(paths: &Paths) {
    if paths.output.exists() {
        match std::fs::remove_dir_all(&paths.output) {
            Ok(()) => log::info!("Borrada la carpeta temporal {}", paths.output.display()),
            Err(e) => log::warn!("No se pudo borrar {}: {e}", paths.output.display()),
        }
    }
    let _ = std::fs::remove_file(paths.data_dir.join("instancia.lock"));
    for dir in [&paths.config_dir, &paths.data_dir] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "tmp") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}
