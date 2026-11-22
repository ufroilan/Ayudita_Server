//! Lo mínimo del protocolo de Supabase Realtime (Phoenix, `vsn=1.0.0`) para escuchar un canal
//! privado de broadcast.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Msg {
    pub topic: String,
    pub event: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(rename = "ref", default)]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_ref: Option<String>,
}

/// `https://xyz.supabase.co` → `wss://xyz.supabase.co/realtime/v1/websocket?apikey=…&vsn=1.0.0`
pub fn ws_url(url: &str, key: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let base = if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url.to_string()
    };
    let base = if base.contains("/realtime/v1") {
        base
    } else {
        format!("{base}/realtime/v1")
    };
    let base = if base.ends_with("/websocket") {
        base
    } else {
        format!("{base}/websocket")
    };
    let key: String = url::form_urlencoded::byte_serialize(key.as_bytes()).collect();
    format!("{base}?apikey={key}&vsn=1.0.0")
}

pub fn channel_topic(topic: &str) -> String {
    if topic.starts_with("realtime:") {
        topic.to_string()
    } else {
        format!("realtime:{topic}")
    }
}

pub fn join(topic: &str, token: &str, reference: &str) -> String {
    let msg = Msg {
        topic: topic.to_string(),
        event: "phx_join".into(),
        payload: json!({
            "config": {
                "broadcast": { "ack": false, "self": false },
                "presence": { "key": "", "enabled": false },
                "postgres_changes": [],
                "private": true
            },
            "access_token": token
        }),
        reference: Some(reference.to_string()),
        join_ref: Some(reference.to_string()),
    };
    serde_json::to_string(&msg).unwrap_or_default()
}

pub fn heartbeat(reference: &str) -> String {
    let msg = Msg {
        topic: "phoenix".into(),
        event: "heartbeat".into(),
        payload: json!({}),
        reference: Some(reference.to_string()),
        join_ref: None,
    };
    serde_json::to_string(&msg).unwrap_or_default()
}

pub fn access_token(topic: &str, token: &str, reference: &str, join_ref: &str) -> String {
    let msg = Msg {
        topic: topic.to_string(),
        event: "access_token".into(),
        payload: json!({ "access_token": token }),
        reference: Some(reference.to_string()),
        join_ref: Some(join_ref.to_string()),
    };
    serde_json::to_string(&msg).unwrap_or_default()
}

pub fn leave(topic: &str, reference: &str, join_ref: &str) -> String {
    let msg = Msg {
        topic: topic.to_string(),
        event: "phx_leave".into(),
        payload: json!({}),
        reference: Some(reference.to_string()),
        join_ref: Some(join_ref.to_string()),
    };
    serde_json::to_string(&msg).unwrap_or_default()
}

#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// Respuesta ok al `phx_join`: el canal está `SUBSCRIBED`.
    Subscribed,
    /// Respuesta de error al `phx_join` o error del canal: `CHANNEL_ERROR`.
    ChannelError(String),
    /// El servidor cerró el canal: `CLOSED`.
    Closed(String),
    HeartbeatReply(String),
    /// Un broadcast con el evento que escuchamos: su `payload`.
    Broadcast(Value),
    Other,
}

pub fn classify(msg: &Msg, topic: &str, join_ref: &str, event: &str) -> Incoming {
    if msg.topic == "phoenix" {
        if msg.event == "phx_reply" {
            return Incoming::HeartbeatReply(msg.reference.clone().unwrap_or_default());
        }
        return Incoming::Other;
    }
    if msg.topic != topic {
        return Incoming::Other;
    }
    let status = msg.payload.get("status").and_then(Value::as_str);
    let reason = || {
        let response = msg.payload.get("response").unwrap_or(&msg.payload);
        ["reason", "message", "error"]
            .iter()
            .find_map(|k| response.get(*k).and_then(Value::as_str))
            .unwrap_or("error del canal")
            .to_string()
    };
    match msg.event.as_str() {
        "phx_reply" if msg.reference.as_deref() == Some(join_ref) => match status {
            Some("ok") => Incoming::Subscribed,
            _ => Incoming::ChannelError(reason()),
        },
        "phx_reply" => match status {
            Some("ok") | None => Incoming::Other,
            _ => Incoming::ChannelError(reason()),
        },
        "phx_error" => Incoming::ChannelError(reason()),
        "phx_close" => Incoming::Closed("el servidor cerró el canal".into()),
        "system" if status == Some("error") => Incoming::ChannelError(reason()),
        "broadcast" => {
            let ev = msg
                .payload
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("");
            if event.is_empty() || ev == event {
                Incoming::Broadcast(msg.payload.get("payload").cloned().unwrap_or(Value::Null))
            } else {
                Incoming::Other
            }
        }
        _ => Incoming::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Msg {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn websocket_url() {
        assert_eq!(
            ws_url("https://abc.supabase.co/", "sb_publishable_x"),
            "wss://abc.supabase.co/realtime/v1/websocket?apikey=sb_publishable_x&vsn=1.0.0"
        );
        assert_eq!(
            ws_url("http://127.0.0.1:54321", "k"),
            "ws://127.0.0.1:54321/realtime/v1/websocket?apikey=k&vsn=1.0.0"
        );
    }

    #[test]
    fn join_carries_token_and_private_flag() {
        let m = parse(&join("realtime:comandas:b1", "tok", "1"));
        assert_eq!(m.event, "phx_join");
        assert_eq!(m.payload["access_token"], "tok");
        assert_eq!(m.payload["config"]["private"], true);
    }

    #[test]
    fn classifies_messages() {
        let t = "realtime:comandas:b1";
        let ok = parse(
            r#"{"topic":"realtime:comandas:b1","event":"phx_reply","payload":{"status":"ok","response":{}},"ref":"1"}"#,
        );
        assert_eq!(classify(&ok, t, "1", "comanda"), Incoming::Subscribed);
        let err = parse(
            r#"{"topic":"realtime:comandas:b1","event":"phx_reply","payload":{"status":"error","response":{"reason":"Unauthorized"}},"ref":"1"}"#,
        );
        assert_eq!(
            classify(&err, t, "1", "comanda"),
            Incoming::ChannelError("Unauthorized".into())
        );
        let bc = parse(
            r#"{"topic":"realtime:comandas:b1","event":"broadcast","payload":{"type":"broadcast","event":"comanda","payload":{"event":"pos.order.created"}},"ref":null}"#,
        );
        assert_eq!(
            classify(&bc, t, "1", "comanda"),
            Incoming::Broadcast(json!({"event":"pos.order.created"}))
        );
        assert_eq!(classify(&bc, t, "1", "otro"), Incoming::Other);
        let hb =
            parse(r#"{"topic":"phoenix","event":"phx_reply","payload":{"status":"ok"},"ref":"7"}"#);
        assert_eq!(
            classify(&hb, t, "1", "comanda"),
            Incoming::HeartbeatReply("7".into())
        );
        let close =
            parse(r#"{"topic":"realtime:comandas:b1","event":"phx_close","payload":{},"ref":"1"}"#);
        assert!(matches!(
            classify(&close, t, "1", "comanda"),
            Incoming::Closed(_)
        ));
        let sys = parse(
            r#"{"topic":"realtime:comandas:b1","event":"system","payload":{"status":"error","message":"token expired"},"ref":null}"#,
        );
        assert_eq!(
            classify(&sys, t, "1", "comanda"),
            Incoming::ChannelError("token expired".into())
        );
    }
}
