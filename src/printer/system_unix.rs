//! Impresoras del sistema en macOS y Linux (CUPS). Se envían los bytes en crudo (`-o raw`), así
//! que vale cualquier controlador con el que esté instalada la impresora.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;

use super::{PState, PrinterStatus, SystemPrinters};

fn cmd(program: &str) -> Command {
    let mut c = Command::new(program);
    // Mensajes en inglés para poder interpretarlos.
    c.env("LC_ALL", "C").env("LANG", "C").kill_on_drop(true);
    c
}

fn std_cmd(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.env("LC_ALL", "C").env("LANG", "C");
    c
}

pub fn list() -> SystemPrinters {
    let run = |args: &[&str]| {
        std_cmd("lpstat")
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let mut names: Vec<String> = run(&["-e"])
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    names.sort();
    names.dedup();
    // macOS traduce los mensajes de `lpstat` aunque se pida inglés: el nombre es la última palabra
    // de la línea (los nombres de CUPS no llevan espacios) y se comprueba que exista.
    let default = run(&["-d"])
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .map(|n| n.trim_end_matches('.').to_string())
        .find(|n| names.contains(n));
    SystemPrinters { names, default }
}

pub async fn print(name: &str, bytes: &[u8], title: &str) -> Result<Option<String>, String> {
    let (st, blocks) = inspect(name).await;
    if blocks {
        return Err(st.detail);
    }
    let mut child = cmd("lp")
        .args(["-d", name, "-o", "raw", "-t", title])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("No se pudo usar el sistema de impresión: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes).await.map_err(|e| e.to_string())?;
        drop(stdin);
    }
    let out = timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| "El sistema de impresión no responde".to_string())?
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(Some("Enviada a la cola del sistema".into()))
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            "El sistema rechazó la impresión".into()
        } else {
            err
        })
    }
}

pub async fn status(name: &str) -> PrinterStatus {
    inspect(name).await.0
}

async fn run(program: &str, args: &[&str]) -> Option<(bool, String)> {
    let out = timeout(
        Duration::from_secs(5),
        cmd(program).args(args).stderr(Stdio::null()).output(),
    )
    .await
    .ok()?
    .ok()?;
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

/// Estado según los atributos IPP de CUPS (`lpoptions -p`), que no dependen del idioma.
/// El segundo valor dice si no tiene sentido mandarle trabajos (pausada o desinstalada).
async fn inspect(name: &str) -> (PrinterStatus, bool) {
    let Some((ok, text)) = run("lpoptions", &["-p", name]).await else {
        return (
            PrinterStatus::new(
                PState::Unknown,
                "No se pudo consultar el sistema de impresión",
            ),
            false,
        );
    };
    if !ok || text.trim().is_empty() {
        return (
            PrinterStatus::new(
                PState::NotConfigured,
                format!("«{name}» ya no está instalada en el sistema"),
            ),
            true,
        );
    }
    let attr = |key: &str| {
        text.split_whitespace()
            .find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .map(|v| v.trim_matches(|c| c == '\'' || c == '"').to_string())
    };
    let state = attr("printer-state").unwrap_or_default();
    let accepting = attr("printer-is-accepting-jobs").is_none_or(|v| v == "true");
    let reasons = attr("printer-state-reasons").unwrap_or_default();
    let has = |r: &str| reasons.split(',').any(|x| x.starts_with(r));

    if state == "5" || !accepting {
        let detail = "Está en pausa en el sistema: reanúdala en los ajustes de impresoras";
        return (PrinterStatus::new(PState::Offline, detail), true);
    }
    if has("media-empty") || has("media-needed") {
        return (
            PrinterStatus::new(PState::PaperOut, "Cambia el rollo de papel"),
            false,
        );
    }
    if has("door-open") || has("cover-open") {
        return (
            PrinterStatus::new(PState::CoverOpen, "Cierra la tapa de la impresora"),
            false,
        );
    }
    if has("media-jam") {
        return (PrinterStatus::new(PState::Error, "Papel atascado"), false);
    }
    let queued = run("lpstat", &["-o", name])
        .await
        .map(|(_, t)| t.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    if state == "4" {
        return (PrinterStatus::new(PState::Busy, ""), false);
    }
    if has("offline") || has("connecting-to-device") || has("timed-out") {
        let detail = if queued > 0 {
            format!(
                "No responde; {queued} trabajo(s) esperando en el sistema. ¿Está encendida y conectada?"
            )
        } else {
            "El sistema la vio sin conexión la última vez. ¿Está encendida y conectada?".to_string()
        };
        return (PrinterStatus::new(PState::Offline, detail), false);
    }
    if queued > 0 {
        let detail = format!(
            "{queued} trabajo(s) atascados en la cola del sistema: ¿está encendida y conectada?"
        );
        return (PrinterStatus::new(PState::Error, detail), false);
    }
    if has("media-low") {
        return (
            PrinterStatus::new(PState::PaperNearEnd, "Prepara un rollo nuevo"),
            false,
        );
    }
    (
        PrinterStatus::new(PState::Unknown, "Instalada en el sistema"),
        false,
    )
}
