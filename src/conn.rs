//! La conexión con Ayudita: inicio de sesión, canal Realtime, reconexión con esperas crecientes,
//! renovación del token, latido diario y «lo que se perdió».

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{sleep, sleep_until, timeout};
use tokio_tungstenite::tungstenite::Message;

use crate::api::{Api, ApiError, ConnectInfo, Token};
use crate::model::Comanda;
use crate::realtime::{self, Incoming};
use crate::session::Vault;
use crate::state::{BusinessInfo, Conn, ConnCmd, SessionInfo, Shared, UserInfo};
use crate::worker;

const BACKOFF: [u64; 7] = [1, 2, 5, 10, 20, 40, 60];
const DAILY: Duration = Duration::from_secs(24 * 3600);
const HEARTBEAT: Duration = Duration::from_secs(25);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
const TOKEN_MARGIN_SECS: f64 = 60.0;

enum End {
    Logout,
    Expired(String),
    NoBusiness(String),
    Reload,
}

enum ChannelEnd {
    End(End),
    /// Se perdió el canal (`CHANNEL_ERROR`, `TIMED_OUT`, `CLOSED`).
    Lost {
        reason: String,
        was_subscribed: bool,
    },
}

pub async fn run(shared: Arc<Shared>, mut rx: UnboundedReceiver<ConnCmd>) {
    let mut message: Option<String> = None;
    loop {
        let base = shared.cfg().base_url();
        let vault = Vault::new(&base, shared.paths.config_dir.clone());
        let api = Arc::new(Api::new(base.clone(), vault));
        log::info!("Ayudita: {base}");

        if !api.has_session() {
            shared.set_conn(Conn::LoggedOut {
                message: message.take(),
            });
            if !wait_for_login(&shared, &api, &mut rx).await {
                continue; // cambió la dirección de Ayudita
            }
        }

        let end = session(&shared, &api, &mut rx).await;
        match end {
            End::Logout => {
                api.forget_session();
                {
                    // Otra cuenta puede ser de otro negocio: no se arrastra nada de esta.
                    let mut store = shared.store();
                    store.last_sent_at = None;
                    store.asks.clear();
                    store.save();
                }
                shared.status().session = SessionInfo::default();
                shared.save_session();
            }
            End::Expired(m) => {
                log::warn!("Sesión caducada: {m}");
                api.forget_session();
                message = Some("Tu sesión de Ayudita caducó. Vuelve a iniciar sesión; la cola de impresión se conserva.".into());
            }
            End::NoBusiness(m) => {
                api.forget_session();
                message = Some(m);
            }
            End::Reload => {}
        }
    }
}

/// Espera a que el usuario inicie sesión. `false` si hay que recargar la dirección de Ayudita.
async fn wait_for_login(shared: &Shared, api: &Api, rx: &mut UnboundedReceiver<ConnCmd>) -> bool {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            ConnCmd::Login { email, password } => {
                shared.set_conn(Conn::LoggingIn);
                match api.login(&email, &password).await {
                    Ok(()) => {
                        shared.status().session.email = Some(email.trim().to_string());
                        shared.save_session();
                        let mut store = shared.store();
                        store.init_last_sent_at(Utc::now());
                        store.save();
                        return true;
                    }
                    Err(e) => shared.set_conn(Conn::LoggedOut {
                        message: Some(e.to_string()),
                    }),
                }
            }
            ConnCmd::Reload => return false,
            ConnCmd::Logout | ConnCmd::RetryNow => {}
        }
    }
    // El canal de órdenes solo se cierra al salir de la app.
    std::future::pending::<()>().await;
    false
}

/// Una sesión: se mantiene conectado (reintentando) hasta cerrar sesión o perderla.
async fn session(shared: &Arc<Shared>, api: &Arc<Api>, rx: &mut UnboundedReceiver<ConnCmd>) -> End {
    let mut attempt: usize = 0;
    let mut info: Option<ConnectInfo> = None;
    let mut last_daily = Instant::now();
    let mut token: Option<Token> = None;

    loop {
        if attempt == 0 {
            shared.set_conn(Conn::Connecting);
        }
        let result: Result<ChannelEnd, ApiError> = async {
            if info.is_none() || last_daily.elapsed() >= DAILY {
                api.refresh().await?;
                let i = api.connect().await?;
                remember(shared, &i);
                info = Some(i);
                last_daily = Instant::now();
            }
            let info = info.clone().expect("connect");
            if !token_fresh(&token) {
                token = Some(api.supabase_token().await?);
            }
            Ok(channel(shared, api, &info, &mut token, &mut last_daily, rx).await)
        }
        .await;

        let reason = match result {
            Err(ApiError::Unauthorized(m)) => return End::Expired(m),
            Err(ApiError::NoBusiness(m)) => return End::NoBusiness(m),
            Err(e) => e.to_string(),
            Ok(ChannelEnd::End(end)) => return end,
            Ok(ChannelEnd::Lost {
                reason,
                was_subscribed,
            }) => {
                if was_subscribed {
                    attempt = 0;
                }
                reason
            }
        };
        log::warn!("Conexión perdida: {reason}");
        let wait = Duration::from_secs(BACKOFF[attempt.min(BACKOFF.len() - 1)]);
        attempt += 1;
        let at = Instant::now() + wait;
        shared.set_conn(Conn::Reconnecting {
            at,
            attempt: attempt as u32,
            reason,
        });
        tokio::select! {
            _ = sleep(wait) => {}
            Some(cmd) = rx.recv() => match cmd {
                ConnCmd::Logout => return End::Logout,
                ConnCmd::Reload => return End::Reload,
                ConnCmd::RetryNow | ConnCmd::Login { .. } => {}
            }
        }
    }
}

fn token_fresh(token: &Option<Token>) -> bool {
    token
        .as_ref()
        .is_some_and(|t| t.expires_at - now_secs() > TOKEN_MARGIN_SECS)
}

fn now_secs() -> f64 {
    Utc::now().timestamp_millis() as f64 / 1000.0
}

fn remember(shared: &Shared, info: &ConnectInfo) {
    {
        let mut st = shared.status();
        st.session.business = Some(BusinessInfo {
            id: info.business.id.clone(),
            name: info.business.name.clone(),
            timezone: info.business.timezone.clone().unwrap_or_default(),
        });
        let name = info.user.full_name();
        st.session.user = Some(UserInfo {
            id: info.user.id.clone(),
            name,
        });
    }
    shared.save_session();
    shared.repaint();
}

async fn channel(
    shared: &Arc<Shared>,
    api: &Arc<Api>,
    info: &ConnectInfo,
    token: &mut Option<Token>,
    last_daily: &mut Instant,
    rx: &mut UnboundedReceiver<ConnCmd>,
) -> ChannelEnd {
    let lost = |reason: String, was_subscribed: bool| ChannelEnd::Lost {
        reason,
        was_subscribed,
    };
    let url = realtime::ws_url(&info.realtime.url, &info.realtime.key);
    let ws = match timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return lost(format!("no se pudo abrir el canal: {e}"), false),
        Err(_) => return lost("TIMED_OUT al abrir el canal".into(), false),
    };
    let (mut sink, mut stream) = ws.split();
    let topic = realtime::channel_topic(&info.realtime.topic);
    let mut next_ref: u64 = 1;
    let mut new_ref = || {
        next_ref += 1;
        next_ref.to_string()
    };
    let join_ref = new_ref();
    // El token va en el propio `phx_join`: sin él el canal dice SUBSCRIBED pero no recibe nada.
    let access = token
        .as_ref()
        .map(|t| t.access_token.clone())
        .unwrap_or_default();
    if let Err(e) = sink
        .send(Message::Text(
            realtime::join(&topic, &access, &join_ref).into(),
        ))
        .await
    {
        return lost(format!("CLOSED: {e}"), false);
    }

    let join_deadline = tokio::time::Instant::now() + JOIN_TIMEOUT;
    let mut subscribed = false;
    let mut heartbeat =
        tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    let mut hb_pending: Option<String> = None;
    let mut catch_up_at: Option<tokio::time::Instant> = None;
    let mut token_retry_at: Option<tokio::time::Instant> = None;
    let far = || tokio::time::Instant::now() + Duration::from_secs(3600);

    loop {
        let token_due = token_retry_at.unwrap_or_else(|| {
            let secs = token
                .as_ref()
                .map(|t| t.expires_at - now_secs() - TOKEN_MARGIN_SECS)
                .unwrap_or(0.0);
            // Como mínimo unos segundos entre renovaciones, por si el token dura muy poco.
            tokio::time::Instant::now() + Duration::from_secs_f64(secs.max(5.0))
        });
        let daily_due = tokio::time::Instant::from_std(*last_daily + DAILY);

        tokio::select! {
            msg = stream.next() => {
                let text = match msg {
                    None => return lost("CLOSED: se cortó la conexión".into(), subscribed),
                    Some(Err(e)) => return lost(format!("CLOSED: {e}"), subscribed),
                    Some(Ok(Message::Close(_))) => return lost("CLOSED por el servidor".into(), subscribed),
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(_)) => continue,
                };
                let Ok(msg) = serde_json::from_str::<realtime::Msg>(&text) else { continue };
                match realtime::classify(&msg, &topic, &join_ref, &info.realtime.event) {
                    Incoming::Subscribed => {
                        subscribed = true;
                        shared.set_conn(Conn::Connected { since: Utc::now() });
                        log::info!("Canal {topic}: SUBSCRIBED");
                        catch_up_at = Some(tokio::time::Instant::now());
                    }
                    Incoming::ChannelError(reason) => {
                        // Puede ser un token caducado o rechazado: se pide otro al reconectar.
                        *token = None;
                        return lost(format!("CHANNEL_ERROR: {reason}"), subscribed);
                    }
                    Incoming::Closed(reason) => return lost(format!("CLOSED: {reason}"), subscribed),
                    Incoming::HeartbeatReply(r) => {
                        if hb_pending.as_deref() == Some(r.as_str()) {
                            hb_pending = None;
                        }
                    }
                    Incoming::Broadcast(payload) => match serde_json::from_value::<Comanda>(payload) {
                        Ok(c) => worker::receive(shared, c),
                        Err(e) => log::warn!("Comanda ilegible: {e}"),
                    },
                    Incoming::Other => {}
                }
            }
            _ = sleep_until(join_deadline), if !subscribed => {
                return lost("TIMED_OUT: el canal no confirmó la suscripción".into(), false);
            }
            _ = heartbeat.tick() => {
                if hb_pending.is_some() {
                    return lost("TIMED_OUT: el servidor no contesta al latido".into(), subscribed);
                }
                let r = new_ref();
                if sink.send(Message::Text(realtime::heartbeat(&r).into())).await.is_err() {
                    return lost("CLOSED al enviar el latido".into(), subscribed);
                }
                hb_pending = Some(r);
            }
            _ = sleep_until(catch_up_at.unwrap_or_else(far)), if catch_up_at.is_some() => {
                catch_up_at = None;
                match catch_up(shared, api).await {
                    Ok(()) => {}
                    Err(ApiError::Unauthorized(m)) => return ChannelEnd::End(End::Expired(m)),
                    Err(e) => {
                        log::warn!("No se pudo pedir lo pendiente: {e}");
                        shared.notice("No se pudo comprobar qué llegó mientras estabas desconectado; se reintenta en 30 s");
                        catch_up_at = Some(tokio::time::Instant::now() + Duration::from_secs(30));
                    }
                }
            }
            _ = sleep_until(token_due) => {
                match api.supabase_token().await {
                    Ok(t) => {
                        let r = new_ref();
                        let msg = realtime::access_token(&topic, &t.access_token, &r, &join_ref);
                        *token = Some(t);
                        token_retry_at = None;
                        if sink.send(Message::Text(msg.into())).await.is_err() {
                            return lost("CLOSED al renovar el token".into(), subscribed);
                        }
                    }
                    Err(ApiError::Unauthorized(m)) => return ChannelEnd::End(End::Expired(m)),
                    Err(e) => {
                        log::warn!("No se pudo renovar el token: {e}");
                        token_retry_at = Some(tokio::time::Instant::now() + Duration::from_secs(15));
                    }
                }
            }
            _ = sleep_until(daily_due) => {
                // Mantiene viva la sesión y avisa a Ayudita de que la app sigue conectada.
                let result = async {
                    api.refresh().await?;
                    api.connect().await
                }.await;
                match result {
                    Ok(i) => {
                        remember(shared, &i);
                        *last_daily = Instant::now();
                    }
                    Err(ApiError::Unauthorized(m)) => return ChannelEnd::End(End::Expired(m)),
                    Err(ApiError::NoBusiness(m)) => return ChannelEnd::End(End::NoBusiness(m)),
                    Err(e) => {
                        log::warn!("Fallo en la renovación diaria: {e}");
                        // Se reintenta en una hora.
                        *last_daily = Instant::now() - DAILY + Duration::from_secs(3600);
                    }
                }
            }
            Some(cmd) = rx.recv() => match cmd {
                ConnCmd::Logout => {
                    let _ = sink.send(Message::Text(realtime::leave(&topic, &new_ref(), &join_ref).into())).await;
                    let _ = sink.close().await;
                    return ChannelEnd::End(End::Logout);
                }
                ConnCmd::Reload => {
                    let _ = sink.close().await;
                    return ChannelEnd::End(End::Reload);
                }
                ConnCmd::RetryNow | ConnCmd::Login { .. } => {}
            }
        }
    }
}

/// Pide a Ayudita lo que llegó mientras no se escuchaba y lo deja esperando confirmación.
async fn catch_up(shared: &Shared, api: &Api) -> Result<(), ApiError> {
    let since = {
        let mut store = shared.store();
        store.init_last_sent_at(Utc::now());
        store.last_sent_at.clone().unwrap_or_default()
    };
    let list = api.pending(&since).await?;
    log::info!("Pendientes desde {since}: {}", list.len());
    let cfg = shared.cfg();
    let asks = {
        let mut store = shared.store();
        let n = store.absorb_pending(list, |t| cfg.accepts_place(t), Utc::now());
        store.save();
        n
    };
    if asks > 0 {
        shared.status().show_window = true;
    }
    shared.repaint();
    Ok(())
}
