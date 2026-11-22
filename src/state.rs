//! Estado compartido entre la interfaz y las tareas de fondo.

use std::sync::{Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Instant;

use chrono::{DateTime, Utc};
use eframe::egui;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc};

use crate::config::{Config, Paths, PrinterSel};
use crate::printer::{PState, PrinterStatus, SimFail, SystemPrinters, Target, UsbPrinter};
use crate::store::Store;

#[derive(Clone, Debug, PartialEq)]
pub enum Conn {
    /// Sin sesión: se enseña la pantalla de inicio de sesión.
    LoggedOut {
        message: Option<String>,
    },
    LoggingIn,
    Connecting,
    Connected {
        since: DateTime<Utc>,
    },
    /// Se perdió el canal; se reintenta a esa hora.
    Reconnecting {
        at: Instant,
        attempt: u32,
        reason: String,
    },
}

impl Conn {
    pub fn needs_login(&self) -> bool {
        matches!(self, Conn::LoggedOut { .. } | Conn::LoggingIn)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BusinessInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub timezone: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub name: String,
}

/// Lo que se sabe de la sesión sin ser secreto (para enseñarlo aunque no haya red).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub business: Option<BusinessInfo>,
    pub user: Option<UserInfo>,
    pub email: Option<String>,
}

pub enum ConnCmd {
    Login {
        email: String,
        password: String,
    },
    Logout,
    /// Volver a intentar ya (botón «Reconectar»).
    RetryNow,
    /// Cambió la dirección de Ayudita (modo avanzado).
    Reload,
}

/// Opciones del modo desarrollador que no se guardan.
pub struct Dev {
    /// Fallo simulado de la impresora virtual.
    pub fail: Mutex<Option<SimFail>>,
}

pub struct Status {
    pub conn: Conn,
    pub session: SessionInfo,
    pub printer: PrinterStatus,
    pub system_printers: SystemPrinters,
    pub usb_printers: Vec<UsbPrinter>,
    pub paused: bool,
    /// Un aviso pasajero para la barra de estado (p. ej. «No se pudo pedir lo pendiente»).
    pub notice: Option<(String, Instant)>,
    pub show_window: bool,
}

pub struct Shared {
    pub paths: Paths,
    pub cfg: RwLock<Config>,
    pub store: Mutex<Store>,
    pub status: Mutex<Status>,
    pub dev: Dev,
    pub wake_worker: Notify,
    pub wake_poller: Notify,
    pub conn_tx: mpsc::UnboundedSender<ConnCmd>,
    pub ctx: OnceLock<egui::Context>,
    /// Solo una impresión o consulta de estado a la vez.
    pub printer_lock: tokio::sync::Mutex<()>,
}

impl Shared {
    pub fn new(
        paths: Paths,
        cfg: Config,
        store: Store,
        session: SessionInfo,
        dev: Dev,
        conn_tx: mpsc::UnboundedSender<ConnCmd>,
    ) -> Shared {
        Shared {
            paths,
            cfg: RwLock::new(cfg),
            store: Mutex::new(store),
            status: Mutex::new(Status {
                conn: Conn::Connecting,
                session,
                printer: PrinterStatus::new(PState::Checking, ""),
                system_printers: SystemPrinters::default(),
                usb_printers: Vec::new(),
                paused: false,
                notice: None,
                show_window: false,
            }),
            dev,
            wake_worker: Notify::new(),
            wake_poller: Notify::new(),
            conn_tx,
            ctx: OnceLock::new(),
            printer_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn cfg(&self) -> Config {
        self.cfg.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_cfg(&self, cfg: Config) {
        cfg.save(&self.paths.config);
        *self.cfg.write().unwrap_or_else(|e| e.into_inner()) = cfg;
        self.wake_poller.notify_one();
        self.wake_worker.notify_one();
    }

    pub fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_conn(&self, conn: Conn) {
        self.status().conn = conn;
        self.repaint();
    }

    pub fn notice(&self, text: impl Into<String>) {
        self.status().notice = Some((text.into(), Instant::now()));
        self.repaint();
    }

    pub fn repaint(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }

    pub fn send(&self, cmd: ConnCmd) {
        let _ = self.conn_tx.send(cmd);
    }

    /// Si ahora mismo se imprime en la impresora virtual (archivos).
    pub fn virtual_printer(&self) -> bool {
        self.cfg().dev_virtual_printer
    }

    pub fn dev_mode(&self) -> bool {
        let cfg = self.cfg();
        cfg.dev_mode || cfg.dev_virtual_printer
    }

    /// A dónde imprimir ahora mismo.
    pub fn target(&self) -> Target {
        if self.virtual_printer() {
            Target::Virtual {
                dir: self.paths.output.clone(),
                fail: *self.dev.fail.lock().unwrap_or_else(|e| e.into_inner()),
            }
        } else {
            Target::Real(self.cfg().printer)
        }
    }

    pub fn printer_label(&self) -> String {
        if self.virtual_printer() {
            "Impresora virtual (modo desarrollo)".into()
        } else {
            match self.cfg().printer {
                PrinterSel::Unset => "Sin elegir".into(),
                sel => sel.label(),
            }
        }
    }

    pub fn save_session(&self) {
        let info = self.status().session.clone();
        if let Err(e) = crate::config::write_json_atomic(&self.paths.session, &info) {
            log::warn!("No se pudo guardar la sesión: {e}");
        }
    }
}
