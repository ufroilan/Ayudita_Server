//! Ayudita simulado para desarrollar y probar la app sin el Ayudita real.
//!
//!     cargo run --example mock_ayudita            # escucha en http://127.0.0.1:3000
//!     cargo run -- --ayudita=http://127.0.0.1:3000 --sin-impresora
//!
//! Cuentas: cualquier correo con la contraseña `1234`. `admin@ayudita.test` no tiene negocio (403).
//!
//! Control (con curl):
//!     curl -X POST 127.0.0.1:3000/mock/order                 # nueva orden (venta)
//!     curl -X POST '127.0.0.1:3000/mock/order?kind=cortesia&place=Barra%207&type=barra'
//!     curl -X POST 127.0.0.1:3000/mock/void/last             # anula la última
//!     curl -X POST 127.0.0.1:3000/mock/prepare/last          # «empieza a prepararla» en Cocina
//!     curl -X POST 127.0.0.1:3000/mock/drop                  # corta los WebSocket (reconexión)
//!     curl -X POST 127.0.0.1:3000/mock/ws-down/30            # rechaza WebSocket durante 30 s
//!     curl -X POST 127.0.0.1:3000/mock/expire                # caduca las sesiones (401)
//!     curl 127.0.0.1:3000/mock/state
//!
//! `MOCK_TOKEN_TTL=90` cambia la duración del token de Supabase (s) para probar la renovación.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use tokio::sync::broadcast;

const BUSINESS_ID: &str = "b0000000-0000-4000-8000-000000000001";

#[derive(Clone)]
struct Event {
    order_id: String,
    event: String,
    sent_at: String,
    body: Value,
}

struct Mock {
    port: u16,
    sessions: HashSet<String>,
    tokens: u64,
    history: Vec<Event>,
    preparing: HashSet<String>,
    seq: i64,
    ws_down_until: Option<Instant>,
    token_ttl: i64,
    connects: u64,
}

type Shared = Arc<(Mutex<Mock>, broadcast::Sender<Value>, broadcast::Sender<()>)>;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("MOCK_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let token_ttl = std::env::var("MOCK_TOKEN_TTL")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3600);
    let (tx, _) = broadcast::channel(64);
    let (drop_tx, _) = broadcast::channel(4);
    let state: Shared = Arc::new((
        Mutex::new(Mock {
            port,
            sessions: HashSet::new(),
            tokens: 0,
            history: Vec::new(),
            preparing: HashSet::new(),
            seq: 40,
            ws_down_until: None,
            token_ttl,
            connects: 0,
        }),
        tx,
        drop_tx,
    ));
    let app = Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/refresh", post(refresh))
        .route("/api/auth/supabase-token", get(supabase_token))
        .route("/api/pos/print/connect", get(connect))
        .route("/api/pos/print/pending", get(pending))
        .route("/realtime/v1/websocket", get(websocket))
        .route("/mock/order", post(mock_order))
        .route("/mock/void/{id}", post(mock_void))
        .route("/mock/prepare/{id}", post(mock_prepare))
        .route("/mock/drop", post(mock_drop))
        .route("/mock/ws-down/{secs}", post(mock_ws_down))
        .route("/mock/expire", post(mock_expire))
        .route("/mock/state", get(mock_state))
        .with_state(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Ayudita simulado en http://{addr}  (contraseña: 1234)");
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "No se puede usar el puerto {port} ({e}). ¿Ya hay otro simulado abierto? Usa ese, o arranca este con MOCK_PORT=3001."
            );
            std::process::exit(1);
        }
    };
    axum::serve(listener, app).await.unwrap();
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, false)
}

fn session_of(headers: &HeaderMap, m: &Mock) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == "ayudita-session")
        .map(|(_, v)| v.to_string())
        .filter(|v| m.sessions.contains(v))
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "message": "No has iniciado sesión" })),
    )
        .into_response()
}

async fn login(State(s): State<Shared>, Json(body): Json<Value>) -> Response {
    let email = body["email"].as_str().unwrap_or("").to_string();
    let password = body["password"].as_str().unwrap_or("");
    println!("POST /api/auth/login {email}");
    if password != "1234" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "Correo o contraseña incorrectos" })),
        )
            .into_response();
    }
    let id = format!(
        "{}{}",
        if email.starts_with("admin") {
            "admin-"
        } else {
            ""
        },
        uuid::Uuid::new_v4()
    );
    s.0.lock().unwrap().sessions.insert(id.clone());
    (
        [(
            header::SET_COOKIE,
            format!("ayudita-session={id}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800"),
        )],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

async fn refresh(State(s): State<Shared>, headers: HeaderMap) -> Response {
    let mut m = s.0.lock().unwrap();
    let Some(old) = session_of(&headers, &m) else {
        return unauthorized();
    };
    let new = if old.starts_with("admin-") {
        format!("admin-{}", uuid::Uuid::new_v4())
    } else {
        uuid::Uuid::new_v4().to_string()
    };
    m.sessions.remove(&old);
    m.sessions.insert(new.clone());
    println!("POST /api/auth/refresh → sesión renovada");
    (
        [(
            header::SET_COOKIE,
            format!("ayudita-session={new}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800"),
        )],
        Json(json!({ "ok": true })),
    )
        .into_response()
}

async fn supabase_token(State(s): State<Shared>, headers: HeaderMap) -> Response {
    let mut m = s.0.lock().unwrap();
    if session_of(&headers, &m).is_none() {
        return unauthorized();
    }
    m.tokens += 1;
    let exp = Utc::now().timestamp() + m.token_ttl;
    println!(
        "GET /api/auth/supabase-token → tok-{} (caduca en {} s)",
        m.tokens, m.token_ttl
    );
    Json(json!({ "accessToken": format!("tok-{}", m.tokens), "expiresAt": exp })).into_response()
}

async fn connect(State(s): State<Shared>, headers: HeaderMap) -> Response {
    let mut m = s.0.lock().unwrap();
    let Some(session) = session_of(&headers, &m) else {
        return unauthorized();
    };
    if session.starts_with("admin-") {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "message": "Sin negocio" })),
        )
            .into_response();
    }
    m.connects += 1;
    println!("GET /api/pos/print/connect");
    Json(json!({
        "business": { "id": BUSINESS_ID, "name": "Malabar", "currency": "BOB", "timezone": "America/La_Paz" },
        "user": { "id": "u1", "name": "Walter", "last_name": "Obrero" },
        "realtime": {
            "url": format!("http://127.0.0.1:{}", m.port),
            "key": "sb_publishable_mock",
            "topic": format!("comandas:{BUSINESS_ID}"),
            "event": "comanda"
        }
    }))
    .into_response()
}

async fn pending(
    State(s): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let m = s.0.lock().unwrap();
    if session_of(&headers, &m).is_none() {
        return unauthorized();
    }
    let since = q
        .get("since")
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok());
    let voided: HashSet<&str> = m
        .history
        .iter()
        .filter(|e| e.event == "pos.order.voided")
        .map(|e| e.order_id.as_str())
        .collect();
    let list: Vec<Value> = m
        .history
        .iter()
        .filter(|e| match since {
            Some(t) => chrono::DateTime::parse_from_rfc3339(&e.sent_at)
                .map(|s| s > t)
                .unwrap_or(true),
            None => true,
        })
        // Lo que la cocina empezó a preparar ya no se devuelve.
        .filter(|e| !(e.event == "pos.order.created" && m.preparing.contains(&e.order_id)))
        // Si entró y se anuló, solo la anulación.
        .filter(|e| !(e.event == "pos.order.created" && voided.contains(e.order_id.as_str())))
        .map(|e| {
            let mut b = e.body.clone();
            if let Some(o) = b.as_object_mut() {
                o.remove("id");
            }
            b
        })
        .collect();
    println!(
        "GET /api/pos/print/pending since={:?} → {}",
        q.get("since"),
        list.len()
    );
    Json(list).into_response()
}

async fn websocket(
    State(s): State<Shared>,
    ws: WebSocketUpgrade,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let down =
        s.0.lock()
            .unwrap()
            .ws_down_until
            .is_some_and(|t| t > Instant::now());
    if down {
        println!("WS rechazado (caída simulada)");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if q.get("apikey").map(String::as_str) != Some("sb_publishable_mock") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| serve_ws(socket, s))
}

async fn serve_ws(mut socket: WebSocket, s: Shared) {
    println!("WS conectado");
    let mut rx = s.1.subscribe();
    let mut drop_rx = s.2.subscribe();
    let mut joined: Option<(String, String)> = None;
    loop {
        tokio::select! {
            msg = socket.recv() => {
                let Some(Ok(Message::Text(text))) = msg else { break };
                let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
                let topic = m["topic"].as_str().unwrap_or("").to_string();
                let r = m["ref"].clone();
                match m["event"].as_str().unwrap_or("") {
                    "heartbeat" => {
                        let reply = json!({"topic":"phoenix","event":"phx_reply","payload":{"status":"ok","response":{}},"ref":r});
                        let _ = socket.send(Message::Text(reply.to_string().into())).await;
                    }
                    "phx_join" => {
                        let token = m["payload"]["access_token"].as_str().unwrap_or("");
                        let private = m["payload"]["config"]["private"].as_bool().unwrap_or(false);
                        let ok = token.starts_with("tok-") && private && topic == format!("realtime:comandas:{BUSINESS_ID}");
                        println!("WS phx_join {topic} token={token} → {}", if ok { "ok" } else { "error" });
                        let payload = if ok {
                            json!({"status":"ok","response":{"postgres_changes":[]}})
                        } else {
                            json!({"status":"error","response":{"reason":"Unauthorized: You do not have permissions to read from this Channel topic"}})
                        };
                        let reply = json!({"topic":topic,"event":"phx_reply","payload":payload,"ref":r,"join_ref":r});
                        let _ = socket.send(Message::Text(reply.to_string().into())).await;
                        if ok {
                            joined = Some((topic, r.as_str().unwrap_or("").to_string()));
                        }
                    }
                    "access_token" => {
                        println!("WS access_token → {}", m["payload"]["access_token"]);
                    }
                    "phx_leave" => {
                        println!("WS phx_leave");
                        joined = None;
                    }
                    _ => {}
                }
            }
            ev = rx.recv() => {
                let Ok(payload) = ev else { continue };
                if let Some((topic, _)) = &joined {
                    let msg = json!({"topic":topic,"event":"broadcast","payload":{"type":"broadcast","event":"comanda","payload":payload},"ref":null});
                    let _ = socket.send(Message::Text(msg.to_string().into())).await;
                }
            }
            _ = drop_rx.recv() => {
                println!("WS cortado a propósito");
                break;
            }
        }
    }
    println!("WS desconectado");
}

fn sample_lines(kind: &str) -> Value {
    match kind {
        "cortesia" => {
            json!([{ "product": "Café americano", "variant": null, "qty": 1, "note": "", "unit_price": 8, "line_total": 8 }])
        }
        "cuenta" => json!([
            { "product": "Pique macho", "variant": "Para 2", "qty": 1, "note": "Poco picante", "unit_price": 70, "line_total": 70 },
            { "product": "Limonada", "variant": "Jarra", "qty": 1, "note": "", "unit_price": 20, "line_total": 20 }
        ]),
        _ => json!([
            { "product": "Mojito", "variant": "Normal", "qty": 2, "note": "Sin hielo", "unit_price": 10, "line_total": 20 },
            { "product": "Hamburguesa doble", "variant": "Con queso", "qty": 1, "note": "Sin cebolla", "unit_price": 35, "line_total": 35 }
        ]),
    }
}

fn publish(s: &Shared, order_id: String, event: &str, order: Value) -> Value {
    let sent_at = now_iso();
    let body = json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "event": event,
        "sent_at": sent_at,
        "business": { "id": BUSINESS_ID, "name": "Malabar", "currency": "BOB", "timezone": "America/La_Paz" },
        "order": order,
        "campo_del_futuro": "se ignora"
    });
    s.0.lock().unwrap().history.push(Event {
        order_id,
        event: event.into(),
        sent_at,
        body: body.clone(),
    });
    let listeners = s.1.send(body.clone()).unwrap_or(0);
    println!("{event} → {listeners} conexión(es) WS");
    body
}

async fn mock_order(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    let kind = q.get("kind").cloned().unwrap_or_else(|| "venta".into());
    let place = q
        .get("place")
        .cloned()
        .unwrap_or_else(|| "Terraza · Mesa 4".into());
    let place_type = q.get("type").cloned().unwrap_or_else(|| "mesa".into());
    let seq = {
        let mut m = s.0.lock().unwrap();
        m.seq += 1;
        m.seq
    };
    let id = uuid::Uuid::new_v4().to_string();
    let lines = sample_lines(&kind);
    let total: f64 = lines
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["line_total"].as_f64())
        .sum();
    let order = json!({
        "id": id, "code": format!("{}MAL{seq:04}VTA", Utc::now().format("%Y%m%d")), "number": 770 + seq, "seq": seq,
        "kind": kind, "status": "pedida", "created_at": now_iso(), "voided_at": null,
        "place": { "type": place_type, "label": place, "floor": null, "table_number": null },
        "note": q.get("note").cloned().unwrap_or_default(), "customer_name": null, "waiter": "Ana Pérez", "total": total,
        "lines": lines
    });
    Json(publish(&s, id, "pos.order.created", order)).into_response()
}

fn find_order(s: &Shared, id: &str) -> Option<Value> {
    let m = s.0.lock().unwrap();
    m.history
        .iter()
        .rev()
        .find(|e| e.event == "pos.order.created" && (id == "last" || e.order_id == id))
        .map(|e| e.body["order"].clone())
}

async fn mock_void(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    let Some(mut order) = find_order(&s, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    order["status"] = json!("anulada");
    order["voided_at"] = json!(now_iso());
    let order_id = order["id"].as_str().unwrap_or("").to_string();
    Json(publish(&s, order_id, "pos.order.voided", order)).into_response()
}

async fn mock_prepare(State(s): State<Shared>, Path(id): Path<String>) -> Response {
    let Some(order) = find_order(&s, &id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let order_id = order["id"].as_str().unwrap_or("").to_string();
    s.0.lock().unwrap().preparing.insert(order_id.clone());
    println!("Cocina empieza a preparar #{}", order["seq"]);
    Json(json!({ "preparing": order_id })).into_response()
}

async fn mock_drop(State(s): State<Shared>) -> &'static str {
    let _ = s.2.send(());
    "ok\n"
}

async fn mock_ws_down(State(s): State<Shared>, Path(secs): Path<u64>) -> &'static str {
    s.0.lock().unwrap().ws_down_until = Some(Instant::now() + Duration::from_secs(secs));
    let _ = s.2.send(());
    "ok\n"
}

async fn mock_expire(State(s): State<Shared>) -> &'static str {
    s.0.lock().unwrap().sessions.clear();
    let _ = s.2.send(());
    "ok\n"
}

async fn mock_state(State(s): State<Shared>) -> Json<Value> {
    let m = s.0.lock().unwrap();
    Json(json!({
        "sessions": m.sessions.len(),
        "tokens": m.tokens,
        "connects": m.connects,
        "history": m.history.iter().map(|e| json!({"event": e.event, "order": e.order_id, "seq": e.body["order"]["seq"], "sent_at": e.sent_at})).collect::<Vec<_>>(),
        "preparing": m.preparing.iter().collect::<Vec<_>>(),
    }))
}
