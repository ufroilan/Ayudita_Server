//! Cliente HTTP de Ayudita. Todas las llamadas llevan las cookies de la sesión.

use std::sync::Mutex;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::Value;

use crate::model::{self, Comanda};
use crate::session::{self, Cookies, Vault};

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// 401: la sesión se perdió (o, en el login, credenciales incorrectas).
    Unauthorized(String),
    /// 403/404 en `connect`: la cuenta no pertenece a ningún negocio.
    NoBusiness(String),
    Status(u16, String),
    Network(String),
    BadResponse(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized(m) | ApiError::NoBusiness(m) => write!(f, "{m}"),
            ApiError::Status(code, m) => write!(f, "Ayudita respondió {code}: {m}"),
            ApiError::Network(m) => write!(f, "Sin conexión con Ayudita: {m}"),
            ApiError::BadResponse(m) => write!(f, "Respuesta inesperada de Ayudita: {m}"),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConnectInfo {
    pub business: ConnectBusiness,
    pub user: ConnectUser,
    pub realtime: RealtimeInfo,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConnectBusiness {
    pub id: String,
    pub name: String,
    pub currency: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConnectUser {
    pub id: String,
    pub name: Option<String>,
    pub last_name: Option<String>,
}

impl ConnectUser {
    pub fn full_name(&self) -> String {
        [self.name.as_deref(), self.last_name.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RealtimeInfo {
    pub url: String,
    pub key: String,
    pub topic: String,
    pub event: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Token {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    /// Segundos Unix.
    #[serde(rename = "expiresAt")]
    pub expires_at: f64,
}

pub struct Api {
    http: reqwest::Client,
    base: String,
    cookies: Mutex<Cookies>,
    vault: Vault,
}

impl Api {
    pub fn new(base: String, vault: Vault) -> Api {
        // TLS con `ring` (sin compilar C extra en Windows). Si ya estaba instalado, no pasa nada.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .user_agent(concat!("AyuditaComandas/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(25))
            // Una redirección a /login es una sesión perdida, no una respuesta que seguir.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("cliente HTTP");
        let cookies = Mutex::new(vault.load());
        Api {
            http,
            base,
            cookies,
            vault,
        }
    }

    pub fn has_session(&self) -> bool {
        !self
            .cookies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    pub fn forget_session(&self) {
        self.cookies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.vault.clear();
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value), ApiError> {
        let url = format!("{}{}", self.base, path);
        let mut req = self
            .http
            .request(method, &url)
            .header("Accept", "application/json");
        let cookie =
            session::cookie_header(&self.cookies.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(cookie) = cookie {
            req = req.header("Cookie", cookie);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(describe(&e)))?;
        let status = resp.status();
        let mut changed = false;
        {
            let mut jar = self.cookies.lock().unwrap_or_else(|e| e.into_inner());
            for h in resp.headers().get_all("set-cookie") {
                if let Ok(h) = h.to_str() {
                    changed |= session::apply_set_cookie(&mut jar, h);
                }
            }
            if changed && !jar.is_empty() {
                self.vault.save(&jar);
            }
        }
        let location = resp
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = resp.text().await.unwrap_or_default();
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_redirection() {
            if location.contains("login") {
                return Err(ApiError::Unauthorized(
                    "Ayudita pide iniciar sesión de nuevo".into(),
                ));
            }
            return Err(ApiError::Status(
                status.as_u16(),
                format!("redirección inesperada a {location}"),
            ));
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized(message(&json).unwrap_or_else(
                || "Tu sesión de Ayudita caducó. Vuelve a iniciar sesión.".into(),
            )));
        }
        Ok((status, json))
    }

    fn fail(status: StatusCode, json: &Value) -> ApiError {
        ApiError::Status(
            status.as_u16(),
            message(json)
                .unwrap_or_else(|| status.canonical_reason().unwrap_or("error").to_string()),
        )
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<(), ApiError> {
        // Una sesión anterior no debe mezclarse con la nueva.
        self.cookies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let body = serde_json::json!({ "email": email.trim(), "password": password });
        let (status, json) = match self.call(Method::POST, "/api/auth/login", Some(body)).await {
            Err(ApiError::Unauthorized(m)) => {
                return Err(ApiError::Unauthorized(if m.contains("caducó") {
                    "Correo o contraseña incorrectos".into()
                } else {
                    m
                }));
            }
            other => other?,
        };
        if status.is_client_error() {
            // Validación (p. ej. «Al menos 8 caracteres»): se enseña el mensaje tal cual.
            return Err(ApiError::Unauthorized(
                message(&json).unwrap_or_else(|| "Revisa el correo y la contraseña".into()),
            ));
        }
        if !status.is_success() {
            return Err(Self::fail(status, &json));
        }
        if !self.has_session() {
            return Err(ApiError::BadResponse(
                "el inicio de sesión no devolvió ninguna cookie".into(),
            ));
        }
        Ok(())
    }

    pub async fn connect(&self) -> Result<ConnectInfo, ApiError> {
        let (status, json) = self
            .call(Method::GET, "/api/pos/print/connect", None)
            .await?;
        if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
            return Err(ApiError::NoBusiness(
                "Esta cuenta no pertenece a ningún negocio. Entra con la cuenta del local (no con una de administrador)."
                    .into(),
            ));
        }
        if !status.is_success() {
            return Err(Self::fail(status, &json));
        }
        let info: ConnectInfo =
            serde_json::from_value(json).map_err(|e| ApiError::BadResponse(e.to_string()))?;
        if info.realtime.url.is_empty() || info.realtime.topic.is_empty() {
            return Err(ApiError::BadResponse("faltan los datos del canal".into()));
        }
        Ok(info)
    }

    pub async fn supabase_token(&self) -> Result<Token, ApiError> {
        let (status, json) = self
            .call(Method::GET, "/api/auth/supabase-token", None)
            .await?;
        if !status.is_success() {
            return Err(Self::fail(status, &json));
        }
        serde_json::from_value(json).map_err(|e| ApiError::BadResponse(e.to_string()))
    }

    pub async fn refresh(&self) -> Result<(), ApiError> {
        let (status, json) = self.call(Method::POST, "/api/auth/refresh", None).await?;
        if !status.is_success() {
            return Err(Self::fail(status, &json));
        }
        Ok(())
    }

    pub async fn pending(&self, since: &str) -> Result<Vec<Comanda>, ApiError> {
        let since: String = url::form_urlencoded::byte_serialize(since.as_bytes()).collect();
        let path = format!("/api/pos/print/pending?since={since}");
        let (status, json) = self.call(Method::GET, &path, None).await?;
        if !status.is_success() {
            return Err(Self::fail(status, &json));
        }
        Ok(model::parse_pending(json))
    }
}

fn message(json: &Value) -> Option<String> {
    ["message", "error", "msg"]
        .iter()
        .find_map(|k| json.get(*k).and_then(Value::as_str))
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "no responde a tiempo".into()
    } else if e.is_connect() {
        "no se puede conectar (¿hay internet?)".into()
    } else {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};

    async fn server() -> String {
        async fn login(axum::Json(body): axum::Json<Value>) -> axum::response::Response {
            if body["password"] == "corta" {
                return (
                    StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({"message": "Al menos 8 caracteres"})),
                )
                    .into_response();
            }
            if body["password"] == "1234" {
                (
                    [(header::SET_COOKIE, "s=abc; Path=/; HttpOnly")],
                    axum::Json(serde_json::json!({"ok": true})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"message": "Contraseña incorrecta"})),
                )
                    .into_response()
            }
        }
        async fn connect(headers: HeaderMap) -> axum::response::Response {
            match headers.get(header::COOKIE).and_then(|c| c.to_str().ok()) {
                Some("s=abc") => StatusCode::FORBIDDEN.into_response(),
                _ => StatusCode::UNAUTHORIZED.into_response(),
            }
        }
        async fn pending(axum::extract::RawQuery(q): axum::extract::RawQuery) -> axum::Json<Value> {
            axum::Json(
                serde_json::json!([{ "event": "pos.order.created", "sent_at": q, "order": { "id": "o1" } }]),
            )
        }
        async fn refresh() -> axum::response::Response {
            (
                StatusCode::FOUND,
                [(header::LOCATION, "/login?redirect=/api/auth/refresh")],
            )
                .into_response()
        }
        let app = Router::new()
            .route("/api/auth/refresh", post(refresh))
            .route("/api/auth/login", post(login))
            .route("/api/pos/print/connect", get(connect))
            .route("/api/pos/print/pending", get(pending));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn api(base: String) -> (Api, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("ayudita-cookies-{}.json", uuid::Uuid::new_v4()));
        (Api::new(base, Vault::in_file(path.clone())), path)
    }

    #[tokio::test]
    async fn login_errors_cookies_and_no_business() {
        let base = server().await;
        let (api, path) = api(base.clone());
        assert_eq!(
            api.login("a@b.c", "mala").await,
            Err(ApiError::Unauthorized("Contraseña incorrecta".into()))
        );
        assert!(!api.has_session());
        assert_eq!(
            api.login("a@b.c", "corta").await,
            Err(ApiError::Unauthorized("Al menos 8 caracteres".into()))
        );
        api.login("a@b.c", "1234").await.unwrap();
        assert!(matches!(
            api.refresh().await,
            Err(ApiError::Unauthorized(_))
        ));
        assert!(api.has_session());
        // Las cookies quedan guardadas y se envían.
        let again = Api::new(base, Vault::in_file(path.clone()));
        assert!(again.has_session());
        assert!(matches!(
            again.connect().await,
            Err(ApiError::NoBusiness(_))
        ));
        again.forget_session();
        assert!(!path.exists());
        assert!(matches!(
            again.connect().await,
            Err(ApiError::Unauthorized(_))
        ));
    }

    #[tokio::test]
    async fn pending_encodes_since() {
        let (api, _) = api(server().await);
        let list = api.pending("2026-10-15T20:01:02.123+00:00").await.unwrap();
        assert_eq!(
            list[0].sent_at,
            "since=2026-10-15T20%3A01%3A02.123%2B00%3A00"
        );
    }
}
