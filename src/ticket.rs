//! Diseño de la comanda en papel.
//!
//! Se construye un documento abstracto (`Doc`) y se maqueta en líneas ya ajustadas al ancho
//! (`layout`). De esas mismas líneas salen los bytes ESC/POS y la vista previa en pantalla, así
//! que lo que se ve es exactamente lo que sale.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::model::{Comanda, EventKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paper {
    #[default]
    Mm80,
    Mm58,
}

impl Paper {
    /// Columnas en la fuente A a tamaño normal.
    pub fn cols(self) -> usize {
        match self {
            Paper::Mm80 => 48,
            Paper::Mm58 => 32,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Paper::Mm80 => "80 mm",
            Paper::Mm58 => "58 mm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    /// Multiplicador de ancho (1..=8).
    pub w: u8,
    /// Multiplicador de alto (1..=8).
    pub h: u8,
    pub bold: bool,
    pub invert: bool,
    pub align: Align,
}

impl Style {
    pub const NORMAL: Style = Style {
        w: 1,
        h: 1,
        bold: false,
        invert: false,
        align: Align::Left,
    };

    const fn sized(w: u8, h: u8) -> Style {
        Style {
            w,
            h,
            bold: true,
            invert: false,
            align: Align::Center,
        }
    }
    fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
    fn center(mut self) -> Self {
        self.align = Align::Center;
        self
    }
    fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Texto que se ajusta a varias líneas; `indent` en columnas de ese tamaño.
    Text {
        text: String,
        style: Style,
        indent: usize,
    },
    /// Banda invertida (blanco sobre negro) a todo el ancho.
    Band {
        text: String,
        w: u8,
        h: u8,
    },
    /// Línea separadora del carácter dado.
    Rule(char),
    Feed(u8),
    Cut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub cols: usize,
    pub blocks: Vec<Block>,
}

/// Una línea ya maquetada: `text` tiene exactamente `cols / w` caracteres (rellena con espacios).
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Line {
        text: String,
        w: u8,
        h: u8,
        bold: bool,
        invert: bool,
    },
    Feed(u8),
    Cut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Llegó de Ayudita y sale por primera vez.
    Original,
    /// Reimpresión pedida desde la app.
    Copy,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub paper: Paper,
    pub show_prices: bool,
}

impl Doc {
    fn new(paper: Paper) -> Self {
        Doc {
            cols: paper.cols(),
            blocks: Vec::new(),
        }
    }
    fn text(&mut self, text: impl Into<String>, style: Style) {
        self.blocks.push(Block::Text {
            text: text.into(),
            style,
            indent: 0,
        });
    }
    fn indented(&mut self, text: impl Into<String>, style: Style, indent: usize) {
        self.blocks.push(Block::Text {
            text: text.into(),
            style,
            indent,
        });
    }
    fn band(&mut self, text: impl Into<String>, size: u8) {
        self.blocks.push(Block::Band {
            text: text.into(),
            w: size,
            h: size,
        });
    }
    fn rule(&mut self, c: char) {
        self.blocks.push(Block::Rule(c));
    }
    fn feed(&mut self, n: u8) {
        self.blocks.push(Block::Feed(n));
    }
}

/// La comanda de una orden.
pub fn comanda(c: &Comanda, mark: Mark, opts: &Options) -> Doc {
    let mut d = Doc::new(opts.paper);
    let voided = c.kind() == EventKind::Voided;
    let tz = parse_tz(&c.business.timezone);
    let o = &c.order;

    if voided {
        d.band("ANULADA", 2);
        d.text("NO PREPARAR", Style::sized(2, 2));
    }
    if mark == Mark::Copy {
        d.band("COPIA", 2);
        d.text("Reimpresión", Style::NORMAL.center());
    }
    match o.kind.as_str() {
        "cortesia" => {
            d.band("CORTESÍA", 2);
            d.text("Se prepara pero NO SE COBRA", Style::NORMAL.bold().center());
        }
        "cuenta" => {
            d.band("CUENTA", 2);
            d.text(
                "A la cuenta de la mesa, se paga al final",
                Style::NORMAL.center(),
            );
        }
        _ => {}
    }
    if !d.blocks.is_empty() {
        d.feed(1);
    }

    // Arriba y en grande: el número del día y a dónde va.
    d.text(c.seq_label(), Style::sized(3, 3));
    let place = c.place_label();
    if !place.is_empty() {
        d.text(place, Style::sized(2, 2));
    }
    d.rule('=');

    // La hora y quién atiende.
    let created = o.created_at.as_deref().and_then(|s| fmt_time(s, tz));
    if voided {
        let voided_at = o.voided_at.as_deref().and_then(|s| fmt_time(s, tz));
        if let Some((hm, date)) = &voided_at {
            d.text(format!("Anulada: {hm} · {date}"), Style::NORMAL.bold());
        }
        if let Some((hm, _)) = &created {
            d.text(format!("Pedida: {hm}"), Style::NORMAL);
        }
    } else if let Some((hm, date)) = &created {
        d.text(format!("Hora: {hm} · {date}"), Style::NORMAL.bold());
    }
    if let Some(w) = o.waiter.as_deref().filter(|w| !w.trim().is_empty()) {
        d.text(w.trim(), Style::NORMAL);
    }
    if let Some(name) = o.customer_name.as_deref().filter(|w| !w.trim().is_empty()) {
        d.text(format!("Cliente: {}", name.trim()), Style::NORMAL);
    }
    d.rule('-');

    if voided {
        d.text("RETIRAR / NO PREPARAR:", Style::NORMAL.bold());
    }
    let currency = c.business.currency.trim();
    for line in &o.lines {
        let mut text = format!(
            "{}× {}",
            fmt_qty(line.qty.unwrap_or(1.0)),
            line.product.trim()
        );
        if let Some(v) = line
            .variant
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            text.push_str(&format!(" ({v})"));
        }
        if voided {
            text = format!("X {text}");
        }
        // Los productos, en doble alto: es lo que se lee de un vistazo en la cocina.
        d.text(
            text,
            Style {
                h: 2,
                ..Style::NORMAL.bold()
            },
        );
        let note = line.note.trim();
        if !note.is_empty() {
            d.indented(format!("» {note}"), Style::NORMAL.bold(), 4);
        }
        if opts.show_prices
            && let Some(total) = line.line_total
        {
            d.text(fmt_money(total, currency), Style::NORMAL.right());
        }
    }
    if o.lines.is_empty() {
        d.text("(sin productos)", Style::NORMAL.center());
    }
    d.rule('-');

    let note = o.note.trim();
    if !note.is_empty() {
        d.text("NOTA DE LA ORDEN:", Style::NORMAL.bold());
        d.text(
            note,
            Style {
                h: 2,
                ..Style::NORMAL.bold()
            },
        );
        d.rule('-');
    }
    if opts.show_prices
        && let Some(total) = o.total
    {
        d.text(
            format!("TOTAL {}", fmt_money(total, currency)),
            Style::NORMAL.bold().right(),
        );
    }
    if voided {
        d.band("ANULADA", 2);
    }

    // Pie pequeño.
    let mut ids = Vec::new();
    if let Some(n) = o.number {
        ids.push(format!("Orden {n}"));
    }
    if !o.code.is_empty() {
        ids.push(o.code.clone());
    }
    if !ids.is_empty() {
        d.text(ids.join(" · "), Style::NORMAL.center());
    }
    if !c.business.name.is_empty() {
        d.text(c.business.name.clone(), Style::NORMAL.center());
    }
    d.feed(4);
    d.blocks.push(Block::Cut);
    d
}

/// Página de prueba local: no llama a Ayudita.
pub fn test_page(opts: &Options, printer: &str, now: DateTime<Utc>) -> Doc {
    let mut d = Doc::new(opts.paper);
    d.band("PRUEBA", 2);
    d.feed(1);
    d.text("Ayudita Comandas", Style::sized(2, 2));
    d.text("La impresora funciona", Style::NORMAL.bold().center());
    d.rule('=');
    d.text(format!("Impresora: {printer}"), Style::NORMAL);
    d.text(
        format!(
            "Papel: {} ({} columnas)",
            opts.paper.label(),
            opts.paper.cols()
        ),
        Style::NORMAL,
    );
    let local = now.with_timezone(&chrono::Local);
    d.text(
        format!("Fecha: {}", local.format("%d/%m/%Y %H:%M")),
        Style::NORMAL,
    );
    d.rule('-');
    d.text("Acentos: á é í ó ú ñ Ñ ü ¿? ¡!", Style::NORMAL);
    d.text(
        "2× Mojito (Normal)",
        Style {
            h: 2,
            ..Style::NORMAL.bold()
        },
    );
    d.indented("» Sin hielo", Style::NORMAL.bold(), 4);
    d.text("Doble alto y ancho", Style::sized(2, 2));
    d.text("#42", Style::sized(3, 3));
    let ruler: String = (0..opts.paper.cols())
        .map(|i| char::from(b'0' + (i % 10) as u8))
        .collect();
    d.text(ruler, Style::NORMAL);
    d.rule('-');
    d.text(
        format!("v{}", env!("CARGO_PKG_VERSION")),
        Style::NORMAL.center(),
    );
    d.feed(4);
    d.blocks.push(Block::Cut);
    d
}

pub fn layout(doc: &Doc) -> Vec<Out> {
    let mut out = Vec::new();
    for block in &doc.blocks {
        match block {
            Block::Text {
                text,
                style,
                indent,
            } => {
                let width = (doc.cols / style.w.max(1) as usize).max(1);
                let indent = (*indent).min(width.saturating_sub(4));
                for (i, piece) in wrap(text, width - indent).into_iter().enumerate() {
                    // Las líneas de continuación de un producto también van sangradas.
                    let pad = if i == 0 {
                        indent
                    } else {
                        indent.max(continuation_indent(text))
                    };
                    let pad = pad.min(width.saturating_sub(1));
                    let piece = truncate(&piece, width - pad);
                    let body = format!("{}{}", " ".repeat(pad), piece);
                    out.push(Out::Line {
                        text: align(&body, width, style.align),
                        w: style.w,
                        h: style.h,
                        bold: style.bold,
                        invert: style.invert,
                    });
                }
            }
            Block::Band { text, w, h } => {
                let width = (doc.cols / (*w).max(1) as usize).max(1);
                for piece in wrap(text, width) {
                    out.push(Out::Line {
                        text: align(&piece, width, Align::Center),
                        w: *w,
                        h: *h,
                        bold: true,
                        invert: true,
                    });
                }
            }
            Block::Rule(c) => out.push(Out::Line {
                text: c.to_string().repeat(doc.cols),
                w: 1,
                h: 1,
                bold: false,
                invert: false,
            }),
            Block::Feed(n) => out.push(Out::Feed(*n)),
            Block::Cut => out.push(Out::Cut),
        }
    }
    out
}

/// Texto plano de la comanda (para los `.txt` del modo sin impresora y los registros).
pub fn plain_text(doc: &Doc) -> String {
    let mut s = String::new();
    for out in layout(doc) {
        match out {
            Out::Line {
                text, w, invert, ..
            } => {
                s.push_str(&" ".repeat(lead(doc.cols, &text, w)));
                let mut line: String = if w > 1 {
                    // Los caracteres anchos se separan con espacios para respetar el ancho real.
                    text.chars()
                        .map(|c| format!("{c}{}", " ".repeat(w as usize - 1)))
                        .collect()
                } else {
                    text
                };
                if invert {
                    line = line.replace(' ', "█");
                }
                s.push_str(line.trim_end_matches(' '));
                s.push('\n');
            }
            Out::Feed(n) => s.push_str(&"\n".repeat(n as usize)),
            Out::Cut => s.push_str(&"✂".repeat(doc.cols / 2)),
        }
    }
    s
}

/// Espacios de tamaño normal que centran una línea cuando el ancho no es múltiplo del tamaño
/// (p. ej. letra triple en 58 mm: 10 × 3 = 30 de 32 columnas).
pub fn lead(cols: usize, text: &str, w: u8) -> usize {
    cols.saturating_sub(text.chars().count() * w.max(1) as usize) / 2
}

fn continuation_indent(text: &str) -> usize {
    // «2× Mojito…»: las líneas siguientes empiezan debajo del nombre del producto.
    if let Some(pos) = text.find("× ") {
        let prefix = &text[..pos];
        if prefix.chars().count() <= 8 {
            return prefix.chars().count() + 2;
        }
    }
    0
}

/// Ajuste de línea por palabras, contando caracteres. Las palabras más largas que el ancho se
/// parten.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut current = String::new();
        let mut len = 0;
        for word in para.split_whitespace() {
            let mut word: Vec<char> = word.chars().collect();
            loop {
                let wl = word.len();
                let needed = if len == 0 { wl } else { len + 1 + wl };
                if needed <= width {
                    if len > 0 {
                        current.push(' ');
                        len += 1;
                    }
                    current.extend(word.iter());
                    len += wl;
                    break;
                }
                if len > 0 {
                    lines.push(std::mem::take(&mut current));
                    len = 0;
                    continue;
                }
                // Palabra sola más larga que la línea.
                let rest = word.split_off(width);
                lines.push(word.iter().collect());
                word = rest;
            }
        }
        lines.push(current);
    }
    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

fn truncate(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

fn align(s: &str, width: usize, align: Align) -> String {
    let len = s.chars().count().min(width);
    let s = truncate(s, width);
    let free = width - len;
    let (left, right) = match align {
        Align::Left => (0, free),
        Align::Right => (free, 0),
        Align::Center => (free / 2, free - free / 2),
    };
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(right))
}

pub fn parse_tz(name: &str) -> Option<Tz> {
    name.trim().parse::<Tz>().ok()
}

/// (`HH:MM`, `dd/mm/aaaa`) en la zona del negocio, o en la hora local si no se conoce.
pub fn fmt_time(iso: &str, tz: Option<Tz>) -> Option<(String, String)> {
    let t = chrono::DateTime::parse_from_rfc3339(iso.trim()).ok()?;
    Some(match tz {
        Some(tz) => {
            let t = t.with_timezone(&tz);
            (
                t.format("%H:%M").to_string(),
                t.format("%d/%m/%Y").to_string(),
            )
        }
        None => {
            let t = t.with_timezone(&chrono::Local);
            (
                t.format("%H:%M").to_string(),
                t.format("%d/%m/%Y").to_string(),
            )
        }
    })
}

pub fn fmt_qty(q: f64) -> String {
    if (q - q.round()).abs() < 1e-9 {
        format!("{}", q.round() as i64)
    } else {
        let s = format!("{q:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub fn fmt_money(v: f64, currency: &str) -> String {
    if currency.is_empty() {
        format!("{v:.2}")
    } else {
        format!("{v:.2} {currency}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::sample;

    fn opts(paper: Paper) -> Options {
        Options {
            paper,
            show_prices: false,
        }
    }

    fn lines(doc: &Doc) -> Vec<(String, u8)> {
        layout(doc)
            .into_iter()
            .filter_map(|o| match o {
                Out::Line { text, w, .. } => Some((text, w)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_line_fills_the_paper_exactly() {
        for paper in [Paper::Mm80, Paper::Mm58] {
            let mut c = sample();
            c.order.note =
                "Que salga todo junto con el postre de la mesa de al lado por favor".into();
            c.order.lines[0].product =
                "Hamburguesa doble con queso cheddar y tocino ahumado".into();
            for (text, w) in lines(&comanda(&c, Mark::Copy, &opts(paper))) {
                let used = text.chars().count() * w as usize;
                assert!(
                    used <= paper.cols() && paper.cols() - used < w as usize,
                    "{text:?}"
                );
            }
        }
    }

    #[test]
    fn created_ticket_has_seq_place_time_and_line() {
        let c = sample();
        let txt = plain_text(&comanda(&c, Mark::Original, &opts(Paper::Mm80)));
        assert!(txt.contains("#  4  2"), "{txt}");
        assert!(txt.contains("T e r r a z a   ·   M e s a   4"), "{txt}");
        // 20:01 UTC son las 16:01 en La Paz.
        assert!(txt.contains("Hora: 16:01 · 15/10/2026"), "{txt}");
        assert!(txt.contains("\nAna Pérez\n"), "{txt}");
        assert!(!txt.contains("Atiende"));
        assert!(txt.contains("2× Mojito (Normal)"), "{txt}");
        assert!(txt.contains("» Sin hielo"));
        assert!(!txt.contains("ANULADA"));
        assert!(!txt.contains("COPIA"));
    }

    #[test]
    fn voided_ticket_reads_as_cancellation() {
        let mut c = sample();
        c.event = crate::model::EV_VOIDED.into();
        c.order.voided_at = Some("2026-10-15T20:15:00+00:00".into());
        let txt = plain_text(&comanda(&c, Mark::Original, &opts(Paper::Mm58)));
        assert_eq!(txt.matches("A█N█U█L█A█D█A").count(), 2, "{txt}");
        assert!(txt.contains("N O   P R E P A R A R"));
        assert!(txt.contains("X 2× Mojito (Normal)"), "{txt}");
        assert!(txt.contains("Anulada: 16:15"));
    }

    #[test]
    fn bands_for_kind_and_copy() {
        let mut c = sample();
        c.order.kind = "cortesia".into();
        let txt = plain_text(&comanda(&c, Mark::Copy, &opts(Paper::Mm80)));
        assert!(txt.contains("C█O█P█I█A"));
        assert!(txt.contains("C█O█R█T█E█S█Í█A"));
        assert!(txt.contains("NO SE COBRA"));
        c.order.kind = "cuenta".into();
        let txt = plain_text(&comanda(&c, Mark::Original, &opts(Paper::Mm80)));
        assert!(txt.contains("C█U█E█N█T█A"));
    }

    #[test]
    fn wrap_breaks_words_and_long_tokens() {
        assert_eq!(wrap("hola que tal", 8), vec!["hola que", "tal"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), vec![""]);
    }

    #[test]
    fn quantities() {
        assert_eq!(fmt_qty(2.0), "2");
        assert_eq!(fmt_qty(1.5), "1.5");
        assert_eq!(fmt_qty(0.25), "0.25");
    }
}
