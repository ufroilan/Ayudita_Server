//! Lo que llega de Ayudita: comandas por Realtime y por `/api/pos/print/pending`.
//!
//! Todo es indulgente: los campos que faltan o vienen a `null` toman su valor por defecto y los
//! campos desconocidos se ignoran (el formato «solo crece»).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

pub const EV_CREATED: &str = "pos.order.created";
pub const EV_VOIDED: &str = "pos.order.voided";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Comanda {
    #[serde(deserialize_with = "null_default")]
    pub event: String,
    #[serde(deserialize_with = "null_default")]
    pub sent_at: String,
    #[serde(deserialize_with = "null_default")]
    pub business: Business,
    #[serde(deserialize_with = "null_default")]
    pub order: Order,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Business {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub currency: String,
    #[serde(deserialize_with = "null_default")]
    pub timezone: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Order {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub code: String,
    #[serde(deserialize_with = "lenient_i64")]
    pub number: Option<i64>,
    #[serde(deserialize_with = "lenient_i64")]
    pub seq: Option<i64>,
    #[serde(deserialize_with = "null_default")]
    pub kind: String,
    #[serde(deserialize_with = "null_default")]
    pub status: String,
    pub created_at: Option<String>,
    pub voided_at: Option<String>,
    pub place: Option<Place>,
    #[serde(deserialize_with = "null_default")]
    pub note: String,
    pub customer_name: Option<String>,
    pub waiter: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub total: Option<f64>,
    #[serde(deserialize_with = "null_default")]
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Place {
    #[serde(rename = "type", deserialize_with = "null_default")]
    pub kind: String,
    #[serde(deserialize_with = "null_default")]
    pub label: String,
    pub floor: Option<String>,
    #[serde(deserialize_with = "lenient_i64")]
    pub table_number: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Line {
    #[serde(deserialize_with = "null_default")]
    pub product: String,
    pub variant: Option<String>,
    #[serde(deserialize_with = "lenient_f64")]
    pub qty: Option<f64>,
    #[serde(deserialize_with = "null_default")]
    pub note: String,
    #[serde(deserialize_with = "lenient_f64")]
    pub unit_price: Option<f64>,
    #[serde(deserialize_with = "lenient_f64")]
    pub line_total: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Created,
    Voided,
    Other,
}

impl Comanda {
    pub fn kind(&self) -> EventKind {
        match self.event.as_str() {
            EV_CREATED => EventKind::Created,
            EV_VOIDED => EventKind::Voided,
            _ => EventKind::Other,
        }
    }

    /// Clave de deduplicación: la pareja (`order.id`, `event`).
    pub fn dedupe_key(&self) -> String {
        dedupe_key(&self.order.id, &self.event)
    }

    pub fn place_label(&self) -> &str {
        self.order
            .place
            .as_ref()
            .map(|p| p.label.as_str())
            .unwrap_or("")
    }

    pub fn place_type(&self) -> &str {
        self.order
            .place
            .as_ref()
            .map(|p| p.kind.as_str())
            .unwrap_or("")
    }

    /// `#42`, o el correlativo/código si no viene el número del día.
    pub fn seq_label(&self) -> String {
        match (self.order.seq, self.order.number) {
            (Some(seq), _) => format!("#{seq}"),
            (None, Some(n)) => format!("#{n}"),
            _ => self.order.code.clone(),
        }
    }
}

pub fn dedupe_key(order_id: &str, event: &str) -> String {
    format!("{order_id}|{event}")
}

pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Interpreta la respuesta de `/api/pos/print/pending`: una lista, o un objeto que la envuelve.
pub fn parse_pending(value: serde_json::Value) -> Vec<Comanda> {
    let list = match value {
        serde_json::Value::Array(list) => list,
        serde_json::Value::Object(map) => ["comandas", "items", "pending", "data", "results"]
            .iter()
            .find_map(|k| map.get(*k).and_then(|v| v.as_array().cloned()))
            .or_else(|| map.values().find_map(|v| v.as_array().cloned()))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    list.into_iter()
        .filter_map(|v| serde_json::from_value::<Comanda>(v).ok())
        .filter(|c| c.kind() != EventKind::Other && !c.order.id.is_empty())
        .collect()
}

fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

fn lenient_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().replace(',', ".").parse().ok(),
        _ => None,
    })
}

fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const SAMPLE: &str = r#"{
      "id": "1d850ebd-0000",
      "event": "pos.order.created",
      "sent_at": "2026-10-15T20:01:02.123+00:00",
      "business": { "id": "b1", "name": "Malabar", "currency": "BOB", "timezone": "America/La_Paz" },
      "order": {
        "id": "o1", "code": "20261015MAL0042VTA", "number": 812, "seq": 42,
        "kind": "venta", "status": "pedida",
        "created_at": "2026-10-15T20:01:01.900+00:00", "voided_at": null,
        "place": { "type": "mesa", "label": "Terraza · Mesa 4", "floor": "Terraza", "table_number": 4 },
        "note": "", "customer_name": null, "waiter": "Ana Pérez", "total": 30,
        "lines": [
          { "product": "Mojito", "variant": "Normal", "qty": 2, "note": "Sin hielo", "unit_price": 10, "line_total": 20 }
        ],
        "campo_nuevo": { "lo": "que sea" }
      },
      "otro_campo_nuevo": 1
    }"#;

    pub fn sample() -> Comanda {
        serde_json::from_str(SAMPLE).unwrap()
    }

    #[test]
    fn parses_sample_and_ignores_unknown_fields() {
        let c = sample();
        assert_eq!(c.kind(), EventKind::Created);
        assert_eq!(c.order.seq, Some(42));
        assert_eq!(c.place_label(), "Terraza · Mesa 4");
        assert_eq!(c.place_type(), "mesa");
        assert_eq!(c.order.lines[0].qty, Some(2.0));
        assert_eq!(c.dedupe_key(), "o1|pos.order.created");
        assert!(parse_time(&c.sent_at).is_some());
    }

    #[test]
    fn tolerates_nulls_and_missing_fields() {
        let c: Comanda = serde_json::from_str(
            r#"{"event":"pos.order.voided","order":{"id":"x","note":null,"lines":null,"place":null,"seq":"7"}}"#,
        )
        .unwrap();
        assert_eq!(c.kind(), EventKind::Voided);
        assert_eq!(c.order.note, "");
        assert!(c.order.lines.is_empty());
        assert_eq!(c.order.seq, Some(7));
        assert_eq!(c.place_label(), "");
    }

    #[test]
    fn pending_accepts_array_or_wrapped() {
        let one: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        let arr = serde_json::Value::Array(vec![one.clone()]);
        assert_eq!(parse_pending(arr).len(), 1);
        let wrapped = serde_json::json!({ "comandas": [one.clone()] });
        assert_eq!(parse_pending(wrapped).len(), 1);
        let other = serde_json::json!({ "data": [one] });
        assert_eq!(parse_pending(other).len(), 1);
    }
}
