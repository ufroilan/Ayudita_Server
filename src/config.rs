//! Ajustes de la app (no secretos) y rutas en disco.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ticket::Paper;

/// Dirección de Ayudita fijada al compilar (`AYUDITA_BASE_URL=... cargo build`).
pub const DEFAULT_BASE_URL: &str = match option_env!("AYUDITA_BASE_URL") {
    Some(url) => url,
    None => "https://ayudita-beta.vercel.app",
};

/// Dirección de Ayudita en desarrollo, la que se propone al activar el modo desarrollo.
pub const DEV_BASE_URL: &str = "http://localhost:3000";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum PrinterSel {
    /// Aún no se ha elegido: al arrancar se propone la predeterminada del sistema.
    #[default]
    Unset,
    /// Impresora instalada en el sistema (CUPS o Windows), se le mandan bytes en crudo.
    System { name: String },
    /// Impresora de red, puerto RAW (normalmente 9100).
    Network { host: String, port: u16 },
    /// USB directo, sin controlador.
    Usb {
        vendor_id: u16,
        product_id: u16,
        serial: Option<String>,
        label: String,
    },
    /// Guarda cada comanda en archivos (desarrollo).
    File { dir: String },
}

impl PrinterSel {
    pub fn label(&self) -> String {
        match self {
            PrinterSel::Unset => "Sin elegir".into(),
            PrinterSel::System { name } => name.clone(),
            PrinterSel::Network { host, port } => format!("Red {host}:{port}"),
            PrinterSel::Usb { label, .. } => format!("USB {label}"),
            PrinterSel::File { dir } => format!("Archivos en {dir}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Modo desarrollo: usa `dev_base_url` en vez de la dirección de producción.
    pub dev_mode: bool,
    pub dev_base_url: String,
    /// En modo desarrollo, imprimir en la impresora virtual (archivos) en vez de la real.
    pub dev_virtual_printer: bool,
    pub printer: PrinterSel,
    pub paper: Paper,
    pub copies: u8,
    pub show_prices: bool,
    pub autostart: bool,
    /// Tipos de `place.type` que imprime este PC. Vacío = todos.
    /// Preparado para separar cocina y barra; sin interfaz en la primera versión.
    pub place_types: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            dev_mode: false,
            dev_base_url: DEV_BASE_URL.into(),
            dev_virtual_printer: false,
            printer: PrinterSel::Unset,
            paper: Paper::Mm80,
            copies: 1,
            show_prices: false,
            autostart: true,
            place_types: Vec::new(),
        }
    }
}

impl Config {
    pub fn base_url(&self) -> String {
        let dev = self.dev_base_url.trim();
        let url = if self.dev_mode && !dev.is_empty() {
            dev
        } else {
            DEFAULT_BASE_URL
        };
        url.trim_end_matches('/').to_string()
    }

    pub fn accepts_place(&self, place_type: &str) -> bool {
        self.place_types.is_empty() || self.place_types.iter().any(|t| t == place_type)
    }

    pub fn load(path: &Path) -> Config {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Err(e) = write_json_atomic(path, self) {
            log::error!("No se pudo guardar la configuración: {e}");
        }
    }
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub config: PathBuf,
    pub store: PathBuf,
    pub session: PathBuf,
    pub output: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn new() -> Paths {
        let dirs = directories::ProjectDirs::from("com", "Ayudita", "Ayudita Comandas");
        // Desarrollo: `AYUDITA_DATOS=/una/carpeta` aísla configuración, cola y sesión.
        let custom = std::env::var_os("AYUDITA_DATOS").map(PathBuf::from);
        let (config_dir, data_dir) = match (&custom, &dirs) {
            (Some(dir), _) => (dir.clone(), dir.clone()),
            (None, Some(d)) => (d.config_dir().to_path_buf(), d.data_dir().to_path_buf()),
            (None, None) => (PathBuf::from("."), PathBuf::from(".")),
        };
        let _ = std::fs::create_dir_all(&config_dir);
        let _ = std::fs::create_dir_all(&data_dir);
        Paths {
            config: config_dir.join("config.json"),
            config_dir: config_dir.clone(),
            session: config_dir.join("session.json"),
            store: data_dir.join("cola.json"),
            output: data_dir.join("salida"),
            data_dir,
        }
    }
}

/// Escribe JSON de forma atómica: primero a un temporal y luego lo renombra.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    write_atomic(path, &bytes)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)
}
