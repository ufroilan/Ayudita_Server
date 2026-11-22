//! Cookies de la sesión de Ayudita guardadas cifradas en el llavero del sistema
//! (Keychain en macOS, Credential Manager en Windows, Secret Service en Linux).
//! La contraseña no se guarda nunca.

use std::collections::BTreeMap;
use std::path::PathBuf;

const SERVICE: &str = "Ayudita Comandas";

pub type Cookies = BTreeMap<String, String>;

/// Dónde se guardan las cookies. Cada dirección de Ayudita tiene su propia sesión.
#[derive(Clone)]
pub struct Vault {
    account: String,
    /// Solo desarrollo (`AYUDITA_SESION_EN_ARCHIVO=1`, o una carpeta de datos aislada con
    /// `AYUDITA_DATOS`, que así no comparte la sesión del llavero) o si el sistema no tiene llavero.
    fallback_file: PathBuf,
    use_file: bool,
}

impl Vault {
    pub fn new(base_url: &str, config_dir: PathBuf) -> Vault {
        let use_file = std::env::var("AYUDITA_SESION_EN_ARCHIVO").is_ok_and(|v| v == "1")
            || std::env::var_os("AYUDITA_DATOS").is_some();
        let safe: String = base_url
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        Vault {
            account: base_url.to_string(),
            fallback_file: config_dir.join(format!("sesion-{safe}.json")),
            use_file,
        }
    }

    #[cfg(test)]
    pub fn in_file(path: PathBuf) -> Vault {
        Vault {
            account: String::new(),
            fallback_file: path,
            use_file: true,
        }
    }

    pub fn load(&self) -> Cookies {
        let text = if self.use_file {
            std::fs::read_to_string(&self.fallback_file).ok()
        } else {
            match keyring::Entry::new(SERVICE, &self.account).and_then(|e| e.get_password()) {
                Ok(text) => Some(text),
                // Si el llavero no está disponible se mira el archivo de respaldo.
                Err(_) => std::fs::read_to_string(&self.fallback_file).ok(),
            }
        };
        text.and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, cookies: &Cookies) {
        let text = serde_json::to_string(cookies).unwrap_or_default();
        if !self.use_file {
            match keyring::Entry::new(SERVICE, &self.account).and_then(|e| e.set_password(&text)) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&self.fallback_file);
                    return;
                }
                Err(e) => log::warn!(
                    "No se pudo usar el llavero del sistema ({e}); se guarda en un archivo privado"
                ),
            }
        }
        if let Err(e) = crate::config::write_atomic(&self.fallback_file, text.as_bytes()) {
            log::error!("No se pudo guardar la sesión: {e}");
        }
    }

    pub fn clear(&self) {
        if !self.use_file
            && let Ok(entry) = keyring::Entry::new(SERVICE, &self.account)
        {
            let _ = entry.delete_credential();
        }
        let _ = std::fs::remove_file(&self.fallback_file);
    }
}

/// Aplica una cabecera `Set-Cookie` al almacén. Devuelve si cambió algo.
pub fn apply_set_cookie(cookies: &mut Cookies, header: &str) -> bool {
    let mut parts = header.split(';');
    let Some((name, value)) = parts.next().and_then(|nv| nv.split_once('=')) else {
        return false;
    };
    let (name, value) = (name.trim().to_string(), value.trim().to_string());
    if name.is_empty() {
        return false;
    }
    let expired = parts.any(|attr| {
        let attr = attr.trim().to_ascii_lowercase();
        attr == "max-age=0"
            || attr.starts_with("max-age=-")
            || attr.contains("expires=thu, 01 jan 1970")
    });
    if expired || value.is_empty() {
        return cookies.remove(&name).is_some();
    }
    cookies.insert(name, value.clone()) != Some(value)
}

pub fn cookie_header(cookies: &Cookies) -> Option<String> {
    if cookies.is_empty() {
        return None;
    }
    Some(
        cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_cookie_add_update_remove() {
        let mut c = Cookies::new();
        assert!(apply_set_cookie(
            &mut c,
            "sb-access=abc; Path=/; HttpOnly; Secure; SameSite=Lax"
        ));
        assert!(apply_set_cookie(
            &mut c,
            "sb-refresh=r1; Path=/; Max-Age=604800"
        ));
        assert!(!apply_set_cookie(&mut c, "sb-access=abc; Path=/"));
        assert!(apply_set_cookie(&mut c, "sb-access=def; Path=/"));
        assert_eq!(cookie_header(&c).unwrap(), "sb-access=def; sb-refresh=r1");
        assert!(apply_set_cookie(&mut c, "sb-refresh=; Path=/; Max-Age=0"));
        assert_eq!(cookie_header(&c).unwrap(), "sb-access=def");
    }

    #[test]
    fn cookie_value_may_contain_equals() {
        let mut c = Cookies::new();
        apply_set_cookie(&mut c, "token=base64==; Path=/");
        assert_eq!(c["token"], "base64==");
    }
}
