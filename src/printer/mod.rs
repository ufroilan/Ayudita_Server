//! Envío de bytes ESC/POS y consulta de estado para cada tipo de impresora.

mod file;
mod network;
#[cfg(unix)]
mod system_unix;
#[cfg(windows)]
mod system_windows;
mod usb;

use std::path::PathBuf;

use crate::config::PrinterSel;

pub use usb::UsbPrinter;

#[cfg(unix)]
use system_unix as system;
#[cfg(windows)]
use system_windows as system;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PState {
    Ready,
    Busy,
    PaperNearEnd,
    PaperOut,
    CoverOpen,
    Offline,
    Error,
    /// Responde pero no informa de su estado (habitual en impresoras del sistema).
    Unknown,
    NotConfigured,
    Checking,
}

impl PState {
    pub fn is_problem(self) -> bool {
        matches!(
            self,
            PState::PaperOut
                | PState::CoverOpen
                | PState::Offline
                | PState::Error
                | PState::NotConfigured
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            PState::Ready => "Lista",
            PState::Busy => "Imprimiendo",
            PState::PaperNearEnd => "Queda poco papel",
            PState::PaperOut => "Sin papel",
            PState::CoverOpen => "Tapa abierta",
            PState::Offline => "Desconectada",
            PState::Error => "Error",
            PState::Unknown => "Conectada",
            PState::NotConfigured => "Sin elegir",
            PState::Checking => "Comprobando…",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrinterStatus {
    pub state: PState,
    pub detail: String,
}

impl PrinterStatus {
    pub fn new(state: PState, detail: impl Into<String>) -> Self {
        PrinterStatus {
            state,
            detail: detail.into(),
        }
    }
}

impl From<crate::escpos::RtStatus> for PrinterStatus {
    fn from(s: crate::escpos::RtStatus) -> Self {
        if s.paper_out {
            PrinterStatus::new(PState::PaperOut, "Cambia el rollo de papel")
        } else if s.cover_open {
            PrinterStatus::new(PState::CoverOpen, "Cierra la tapa de la impresora")
        } else if s.error {
            PrinterStatus::new(PState::Error, "La impresora indica un error")
        } else if s.offline {
            PrinterStatus::new(PState::Offline, "La impresora está fuera de línea")
        } else if s.paper_near_end {
            PrinterStatus::new(PState::PaperNearEnd, "Prepara un rollo nuevo")
        } else {
            PrinterStatus::new(PState::Ready, "")
        }
    }
}

/// Fallo simulado del modo desarrollador.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimFail {
    PaperOut,
    CoverOpen,
    Disconnected,
}

impl SimFail {
    pub fn parse(s: &str) -> Option<SimFail> {
        match s.trim().to_lowercase().as_str() {
            "sin-papel" | "papel" => Some(SimFail::PaperOut),
            "tapa-abierta" | "tapa" => Some(SimFail::CoverOpen),
            "desconectada" | "offline" => Some(SimFail::Disconnected),
            _ => None,
        }
    }

    pub fn status(self) -> PrinterStatus {
        match self {
            SimFail::PaperOut => PrinterStatus::new(PState::PaperOut, "Simulado: sin papel"),
            SimFail::CoverOpen => PrinterStatus::new(PState::CoverOpen, "Simulado: tapa abierta"),
            SimFail::Disconnected => {
                PrinterStatus::new(PState::Offline, "Simulado: impresora desconectada")
            }
        }
    }
}

/// A dónde se manda de verdad una impresión.
#[derive(Clone, Debug)]
pub enum Target {
    Real(PrinterSel),
    /// Impresora virtual del modo `--sin-impresora`.
    Virtual {
        dir: PathBuf,
        fail: Option<SimFail>,
    },
}

pub struct PrintJob<'a> {
    pub bytes: &'a [u8],
    pub title: &'a str,
    /// Texto plano para los archivos del modo desarrollador.
    pub preview: &'a str,
}

/// Imprime. `Ok(Some(nota))` si salió con algún matiz (p. ej. «enviada a la cola del sistema»).
pub async fn print(target: &Target, job: PrintJob<'_>) -> Result<Option<String>, String> {
    match target {
        Target::Virtual { dir, fail } => {
            if let Some(fail) = fail {
                return Err(fail.status().detail);
            }
            file::print(dir, &job)
                .await
                .map(|path| Some(format!("Guardada en {path}")))
        }
        Target::Real(sel) => match sel {
            PrinterSel::Unset => Err("No hay ninguna impresora elegida".into()),
            PrinterSel::System { name } => system::print(name, job.bytes, job.title).await,
            PrinterSel::Network { host, port } => network::print(host, *port, job.bytes).await,
            PrinterSel::Usb {
                vendor_id,
                product_id,
                serial,
                ..
            } => usb::print(*vendor_id, *product_id, serial.clone(), job.bytes.to_vec()).await,
            PrinterSel::File { dir } => file::print(&PathBuf::from(dir), &job)
                .await
                .map(|path| Some(format!("Guardada en {path}"))),
        },
    }
}

pub async fn status(target: &Target) -> PrinterStatus {
    match target {
        Target::Virtual {
            fail: Some(fail), ..
        } => fail.status(),
        Target::Virtual { .. } => PrinterStatus::new(PState::Ready, "Impresora virtual"),
        Target::Real(sel) => match sel {
            PrinterSel::Unset => PrinterStatus::new(PState::NotConfigured, "Elige una impresora"),
            PrinterSel::System { name } => system::status(name).await,
            PrinterSel::Network { host, port } => network::status(host, *port).await,
            PrinterSel::Usb {
                vendor_id,
                product_id,
                serial,
                ..
            } => usb::status(*vendor_id, *product_id, serial.clone()).await,
            PrinterSel::File { .. } => PrinterStatus::new(PState::Ready, "Guarda en archivos"),
        },
    }
}

#[derive(Clone, Debug, Default)]
pub struct SystemPrinters {
    pub names: Vec<String>,
    pub default: Option<String>,
}

/// Impresoras instaladas en el sistema y la predeterminada. Bloqueante.
pub fn list_system() -> SystemPrinters {
    system::list()
}

/// Impresoras USB conectadas (clase de impresora). Bloqueante.
pub fn list_usb() -> Vec<UsbPrinter> {
    usb::list()
}
