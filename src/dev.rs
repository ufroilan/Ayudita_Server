//! Comandas de ejemplo para el modo desarrollador (no llaman a Ayudita).

use std::sync::Arc;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};

use crate::model::{Business, Comanda, EV_CREATED, EV_VOIDED, Line, Order, Place};
use crate::state::Shared;
use crate::worker;

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, false)
}

fn business() -> Business {
    Business {
        id: "demo".into(),
        name: "Local de prueba".into(),
        currency: "BOB".into(),
        timezone: "America/La_Paz".into(),
    }
}

fn line(product: &str, variant: Option<&str>, qty: f64, note: &str, price: f64) -> Line {
    Line {
        product: product.into(),
        variant: variant.map(Into::into),
        qty: Some(qty),
        note: note.into(),
        unit_price: Some(price),
        line_total: Some(price * qty),
    }
}

pub fn order(
    seq: i64,
    kind: &str,
    place_type: &str,
    place: &str,
    lines: Vec<Line>,
    note: &str,
) -> Comanda {
    let total = lines.iter().filter_map(|l| l.line_total).sum();
    Comanda {
        event: EV_CREATED.into(),
        sent_at: now_iso(),
        business: business(),
        order: Order {
            id: uuid::Uuid::new_v4().to_string(),
            code: format!("{}DEMO{seq:04}VTA", Utc::now().format("%Y%m%d")),
            number: Some(800 + seq),
            seq: Some(seq),
            kind: kind.into(),
            status: "pedida".into(),
            created_at: Some(now_iso()),
            voided_at: None,
            place: Some(Place {
                kind: place_type.into(),
                label: place.into(),
                floor: None,
                table_number: None,
            }),
            note: note.into(),
            customer_name: None,
            waiter: Some("Ana Pérez".into()),
            total: Some(total),
            lines,
        },
    }
}

pub fn voided(c: &Comanda) -> Comanda {
    let mut v = c.clone();
    v.event = EV_VOIDED.into();
    v.sent_at = now_iso();
    v.order.status = "anulada".into();
    v.order.voided_at = Some(now_iso());
    v
}

/// Nueva, cortesía y cuenta; a los pocos segundos, la anulación de la primera.
pub fn inject(shared: Arc<Shared>) {
    let a = order(
        41,
        "venta",
        "mesa",
        "Terraza · Mesa 4",
        vec![
            line("Mojito", Some("Normal"), 2.0, "Sin hielo", 10.0),
            line(
                "Hamburguesa doble con queso",
                Some("Grande"),
                1.0,
                "Sin cebolla, carne bien cocida",
                35.0,
            ),
            line("Papas fritas", None, 1.0, "", 12.0),
        ],
        "Traer todo junto",
    );
    let b = order(
        42,
        "cortesia",
        "barra",
        "Barra 7",
        vec![line("Café americano", None, 1.0, "", 8.0)],
        "",
    );
    let c = order(
        43,
        "cuenta",
        "mesa",
        "Salón · Mesa 12",
        vec![
            line("Pique macho", Some("Para 2"), 1.0, "Poco picante", 70.0),
            line("Limonada", None, 3.0, "", 7.0),
        ],
        "",
    );
    let void = voided(&a);
    for comanda in [a, b, c] {
        worker::receive(&shared, comanda);
    }
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        worker::receive(&shared, void);
    });
}

/// Simula que mientras la app estaba cerrada llegaron dos comandas.
pub fn simulate_missed(shared: &Shared) {
    let list = vec![
        order(
            51,
            "venta",
            "llevar",
            "Para llevar",
            vec![line("Salteña", Some("Carne"), 4.0, "", 6.0)],
            "Cliente espera en caja",
        ),
        order(
            52,
            "venta",
            "autoservicio",
            "Autoservicio en barra",
            vec![line("Té", None, 1.0, "Con miel", 5.0)],
            "",
        ),
    ];
    let cfg = shared.cfg();
    {
        let mut store = shared.store();
        store.absorb_pending(list, |t| cfg.accepts_place(t), Utc::now());
        store.save();
    }
    shared.status().show_window = true;
    shared.repaint();
}
