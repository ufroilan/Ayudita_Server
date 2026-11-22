//! Vista previa de la comanda «en papel», a partir de las mismas líneas que van a la impresora.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::ticket::{Doc, Out, layout, lead};

const PAPER: Color32 = Color32::from_rgb(0xFF, 0xFE, 0xFA);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MARGIN: f32 = 12.0;

/// Dibuja el ticket ocupando como mucho `max_width` de ancho.
pub fn paper(ui: &mut egui::Ui, doc: &Doc, max_width: f32) -> egui::Response {
    paper_fit(ui, doc, max_width, f32::INFINITY)
}

/// Dibuja el ticket entero dentro de `max_width` × `max_height`, lo más grande que quepa.
pub fn paper_fit(ui: &mut egui::Ui, doc: &Doc, max_width: f32, max_height: f32) -> egui::Response {
    let lines = layout(doc);
    let probe = FontId::monospace(10.0);
    let unit = ui.painter().layout_no_wrap("M".into(), probe, INK).size().x / 10.0;
    // Alto del ticket por cada punto de letra (todo escala con el tamaño de letra).
    let rows: f32 = lines
        .iter()
        .map(|l| match l {
            Out::Line { h, .. } => *h as f32,
            Out::Feed(n) => 0.5 * (*n).min(2) as f32,
            Out::Cut => 1.2,
        })
        .sum();
    let by_width = (max_width - 2.0 * MARGIN) / (doc.cols as f32 * unit);
    let by_height = (max_height - MARGIN) / (rows * 1.3);
    let base = by_width.min(by_height).clamp(4.0, 30.0);
    let cw = unit * base;
    let row = base * 1.3;
    let height = MARGIN + row * rows;
    let width = doc.cols as f32 * cw + 2.0 * MARGIN;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect.expand(6.0));

    // Sombra suave y papel.
    painter.rect_filled(
        rect.translate(Vec2::new(0.0, 2.0)).expand(1.0),
        2.0,
        Color32::from_black_alpha(28),
    );
    painter.rect_filled(rect, 2.0, PAPER);

    let left = rect.left() + MARGIN;
    let mut y = rect.top() + MARGIN;
    for l in &lines {
        match l {
            Out::Line {
                text,
                w,
                h,
                bold,
                invert,
            } => {
                let line_h = row * *h as f32;
                let x0 = left + lead(doc.cols, text, *w) as f32 * cw;
                let font = FontId::monospace(base * *w as f32);
                if *invert {
                    let band_w = text.chars().count() as f32 * cw * *w as f32;
                    let band = Rect::from_min_size(
                        Pos2::new(x0, y + 1.0),
                        Vec2::new(band_w, line_h - 2.0),
                    );
                    painter.rect_filled(band, 1.0, INK);
                }
                let color = if *invert { PAPER } else { INK };
                if h > w {
                    // Doble alto: el texto se estira en vertical, como en el papel.
                    let sy = *h as f32 / *w as f32;
                    stretched(
                        ui,
                        &painter,
                        Pos2::new(x0, y),
                        line_h,
                        text,
                        &font,
                        color,
                        sy,
                        *bold,
                    );
                    y += line_h;
                    continue;
                }
                let pos = Pos2::new(x0, y + line_h / 2.0);
                painter.text(pos, Align2::LEFT_CENTER, text, font.clone(), color);
                if *bold {
                    // La fuente monoespaciada no tiene negrita: se simula con un segundo trazo.
                    painter.text(
                        pos + Vec2::new(0.6, 0.0),
                        Align2::LEFT_CENTER,
                        text,
                        font,
                        color,
                    );
                }
                y += line_h;
            }
            Out::Feed(n) => y += row * 0.5 * (*n).min(2) as f32,
            Out::Cut => {
                let mid = y + row * 0.6;
                let mut x = rect.left();
                while x < rect.right() {
                    painter.line_segment(
                        [
                            Pos2::new(x, mid),
                            Pos2::new((x + 4.0).min(rect.right()), mid),
                        ],
                        Stroke::new(1.0, Color32::from_gray(150)),
                    );
                    x += 8.0;
                }
                painter.text(
                    Pos2::new(rect.left() + 4.0, mid),
                    Align2::LEFT_CENTER,
                    "✂",
                    FontId::proportional(12.0),
                    Color32::from_gray(120),
                );
                y += row * 1.2;
            }
        }
    }
    response
}

/// Dibuja `text` estirado `sy` veces en vertical, centrado en una fila de alto `line_h`.
#[allow(clippy::too_many_arguments)]
fn stretched(
    ui: &egui::Ui,
    painter: &egui::Painter,
    top_left: Pos2,
    line_h: f32,
    text: &str,
    font: &FontId,
    color: Color32,
    sy: f32,
    bold: bool,
) {
    let galley = painter.layout_no_wrap(text.to_string(), font.clone(), color);
    let top = top_left.y + (line_h - galley.size().y * sy) / 2.0;
    let mut shapes = vec![egui::epaint::ClippedShape {
        clip_rect: Rect::EVERYTHING,
        shape: egui::Shape::galley(Pos2::new(top_left.x, top), galley.clone(), color),
    }];
    if bold {
        shapes.push(egui::epaint::ClippedShape {
            clip_rect: Rect::EVERYTHING,
            shape: egui::Shape::galley(Pos2::new(top_left.x + 0.6, top), galley, color),
        });
    }
    let ppp = ui.ctx().pixels_per_point();
    for prim in ui.ctx().tessellate(shapes, ppp) {
        if let egui::epaint::Primitive::Mesh(mut mesh) = prim.primitive {
            for v in &mut mesh.vertices {
                v.pos.y = top + (v.pos.y - top) * sy;
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }
}
