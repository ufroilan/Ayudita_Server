//! El aspecto de Ayudita (Nuxt UI): Public Sans, grises pizarra, teal de marca, radios de 4 px,
//! y el tema oscuro azul marino de Ayudita. Sigue el tema claro/oscuro del sistema.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin,
    Response, RichText, Sense, Stroke, TextStyle, Theme, Ui, Vec2,
};

#[derive(Clone, Copy)]
pub struct Pal {
    /// `--ui-bg`: el fondo del contenido.
    pub bg: Color32,
    /// `--ui-bg-muted`: la barra lateral.
    pub muted_bg: Color32,
    /// `--ui-bg-elevated`: hover, pestañas, campos.
    pub elevated: Color32,
    pub accented: Color32,
    pub border: Color32,
    pub border_accented: Color32,
    pub text: Color32,
    /// `--ui-text-highlighted`.
    pub strong: Color32,
    pub muted: Color32,
    pub dimmed: Color32,
    /// Teal de marca como relleno.
    pub primary: Color32,
    /// Texto sobre el teal.
    pub on_primary: Color32,
    /// Teal legible como texto sobre el fondo.
    pub primary_text: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub info: Color32,
    pub neutral: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Tema claro: Nuxt UI con neutros «slate» y el teal `#00FFD9` de Ayudita.
pub const LIGHT: Pal = Pal {
    bg: hex(0xFFFFFF),
    muted_bg: hex(0xF8FAFC),
    elevated: hex(0xF1F5F9),
    accented: hex(0xE2E8F0),
    border: hex(0xE2E8F0),
    border_accented: hex(0xCBD5E1),
    text: hex(0x334155),
    strong: hex(0x0F172A),
    muted: hex(0x64748B),
    dimmed: hex(0x94A3B8),
    primary: hex(0x00FFD9),
    on_primary: hex(0x00332B),
    primary_text: hex(0x00A38B),
    success: hex(0x16A34A),
    warning: hex(0xCA8A04),
    error: hex(0xDC2626),
    info: hex(0x2563EB),
    neutral: hex(0x64748B),
};

/// Tema oscuro: el azul marino y teal de `.dark` en `main.css` de Ayudita.
pub const DARK: Pal = Pal {
    bg: hex(0x010D17),
    muted_bg: hex(0x091820),
    elevated: hex(0x182D2F),
    accented: hex(0x1C3D45),
    border: hex(0x172E2C),
    border_accented: hex(0x24474A),
    text: hex(0xF0F7F5),
    strong: hex(0xFFFFFF),
    muted: hex(0x7BB1B0),
    dimmed: hex(0x4F828D),
    primary: hex(0x33FFE1),
    on_primary: hex(0x020D13),
    primary_text: hex(0x33FFE1),
    success: hex(0x4ADE80),
    warning: hex(0xFACC15),
    error: hex(0xF87171),
    info: hex(0x60A5FA),
    neutral: hex(0x7BB1B0),
};

/// Mezcla opaca de `color` sobre `bg` (para tintes que no se pintan encima de nada).
pub fn tint(bg: Color32, color: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        mix(bg.r(), color.r()),
        mix(bg.g(), color.g()),
        mix(bg.b(), color.b()),
    )
}

pub fn pal(ctx: &egui::Context) -> Pal {
    if ctx.global_style().visuals.dark_mode {
        DARK
    } else {
        LIGHT
    }
}

/// El marco de los modales: fondo, borde, sombra y margen generoso.
pub fn modal_frame(ctx: &egui::Context) -> Frame {
    let p = pal(ctx);
    Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(Margin::same(24))
        .shadow(ctx.global_style().visuals.window_shadow)
}

/// La familia seminegrita de Public Sans (egui no tiene pesos: es otra familia).
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn setup(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "PublicSans".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/PublicSans-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "PublicSans-SemiBold".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/PublicSans-SemiBold.ttf")).into(),
    );
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut regular = vec!["PublicSans".to_string()];
    regular.extend(fallback.iter().cloned());
    let mut bold = vec!["PublicSans-SemiBold".to_string(), "PublicSans".to_string()];
    bold.extend(fallback);
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(semibold(), bold);
    ctx.set_fonts(fonts);

    ctx.set_style_of(Theme::Light, style(LIGHT, false));
    ctx.set_style_of(Theme::Dark, style(DARK, true));
    // Sigue al sistema; `AYUDITA_TEMA=claro|oscuro` lo fuerza (para revisar los dos en desarrollo).
    let theme = match std::env::var("AYUDITA_TEMA").as_deref() {
        Ok("claro") => egui::ThemePreference::Light,
        Ok("oscuro") => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    };
    ctx.set_theme(theme);
}

fn style(p: Pal, dark: bool) -> egui::Style {
    let mut style = egui::Style {
        text_styles: [
            (TextStyle::Heading, FontId::new(17.0, semibold())),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into(),
        ..Default::default()
    };
    style.spacing.item_spacing = Vec2::new(6.0, 5.0);
    style.spacing.button_padding = Vec2::new(8.0, 3.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.window_margin = Margin::same(16);

    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let r = CornerRadius::same(4);
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(8);
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.muted_bg;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.primary_text;
    v.selection.bg_fill = p.primary.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.primary_text);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.corner_radius = r;
    for (w, fill, stroke) in [
        (&mut v.widgets.inactive, p.bg, p.border_accented),
        (&mut v.widgets.hovered, p.elevated, p.border_accented),
        (&mut v.widgets.active, p.accented, p.border_accented),
        (&mut v.widgets.open, p.elevated, p.border_accented),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, stroke);
        w.fg_stroke = Stroke::new(1.0, p.text);
        w.corner_radius = r;
        w.expansion = 0.0;
    }
    // Las casillas y radios marcados, en teal.
    v.widgets.inactive.fg_stroke = Stroke::new(1.5, p.text);
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(30),
    };
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(40),
    };
    style.visuals = v;
    style
}

// ---------------------------------------------------------------------------------------------
// Piezas

/// Tarjeta con cabecera: título a la izquierda y algo (un estado) a la derecha, una línea, y el
/// cuerpo con más aire.
pub fn titled_card(
    ui: &mut Ui,
    title: &str,
    subtitle: Option<&str>,
    right: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
) {
    let p = pal(ui.ctx());
    Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::new()
                .inner_margin(Margin {
                    left: 16,
                    right: 14,
                    top: 12,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // Primero el estado (a la derecha); el título ocupa lo que queda y se recorta,
                    // así un nombre largo nunca ensancha la tarjeta.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        right(ui);
                        ui.add_space(8.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_max_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(
                                egui::Label::new(strong(title).size(15.0).color(p.strong))
                                    .truncate(),
                            )
                            .on_hover_text(title);
                            if let Some(sub) = subtitle {
                                ui.add(
                                    egui::Label::new(RichText::new(sub).size(12.0).color(p.muted))
                                        .truncate(),
                                )
                                .on_hover_text(sub);
                            }
                        });
                    });
                });
            let rect = ui.min_rect();
            ui.painter().hline(
                rect.left()..=rect.right(),
                ui.cursor().top(),
                Stroke::new(1.0, p.border),
            );
            Frame::new()
                .inner_margin(Margin {
                    left: 16,
                    right: 14,
                    top: 12,
                    bottom: 14,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 8.0;
                    body(ui);
                });
        });
}

/// Estado pequeño con su punto, para las cabeceras de las tarjetas.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) {
    let frame = Frame::new()
        .fill(color.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.30)))
        .corner_radius(10)
        .inner_margin(Margin {
            left: 18,
            right: 8,
            top: 1,
            bottom: 1,
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(11.5)
                    .color(color)
                    .family(semibold()),
            )
        });
    let rect = frame.response.rect;
    ui.painter()
        .circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 3.0, color);
}

/// Etiqueta de un campo de formulario.
pub fn field_label(ui: &mut Ui, text: &str) {
    let p = pal(ui.ctx());
    ui.add_space(2.0);
    ui.label(strong(text).size(12.0).color(p.text));
    ui.add_space(-4.0);
}

/// Dato «etiqueta … valor», con el valor alineado a la derecha.
pub fn kv(ui: &mut Ui, key: &str, value: &str) {
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(12.5).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).size(12.5).color(p.strong)).truncate());
        });
    });
}

/// Control segmentado (como un `UTabs` pequeño): una opción de varias.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, items: &[(T, &str)]) -> bool {
    let p = pal(ui.ctx());
    let mut changed = false;
    Frame::new()
        .fill(p.elevated)
        .corner_radius(6)
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // Las opciones se reparten todo el ancho disponible.
                let n = items.len().max(1) as f32;
                let width = ((ui.available_width() - 2.0 * (n - 1.0)) / n).max(56.0);
                for (v, label) in items {
                    let active = *value == *v;
                    let text =
                        strong(*label)
                            .size(12.5)
                            .color(if active { p.strong } else { p.muted });
                    let button = egui::Button::new(text)
                        .fill(if active { p.bg } else { Color32::TRANSPARENT })
                        .stroke(if active {
                            Stroke::new(1.0, p.border)
                        } else {
                            Stroke::NONE
                        })
                        .corner_radius(4)
                        .min_size(Vec2::new(width, 26.0));
                    if ui.add(button).clicked() && !active {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Fila de ajuste: título y explicación a la izquierda, el control a la derecha.
pub fn setting_row(ui: &mut Ui, title: &str, description: &str, control: impl FnOnce(&mut Ui)) {
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        let text_w = (ui.available_width() - 140.0).max(200.0);
        ui.allocate_ui_with_layout(
            Vec2::new(text_w, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(strong(title).size(13.5).color(p.strong));
                ui.label(RichText::new(description).size(12.0).color(p.muted));
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

/// Línea separadora con aire.
pub fn divider(ui: &mut Ui) {
    let p = pal(ui.ctx());
    ui.add_space(6.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(
        rect.left()..=rect.right(),
        rect.top(),
        Stroke::new(1.0, p.border),
    );
    ui.add_space(8.0);
}

/// Interruptor solo, sin texto (para las filas de ajustes).
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let p = pal(ui.ctx());
    let size = Vec2::new(34.0, 19.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = if *on { p.primary } else { p.accented };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(10), track);
    let x = egui::lerp((rect.left() + 9.5)..=(rect.right() - 9.5), t);
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 7.5, p.bg);
    resp
}

/// Rótulo de sección: pequeño, en mayúsculas y apagado, como los grupos de Ayudita.
pub fn section(ui: &mut Ui, text: &str) {
    let p = pal(ui.ctx());
    ui.label(
        RichText::new(text.to_uppercase())
            .size(10.5)
            .color(p.dimmed)
            .family(semibold())
            .extra_letter_spacing(0.8),
    );
}

pub fn strong(text: impl Into<String>) -> RichText {
    RichText::new(text).family(semibold())
}

/// Insignia «subtle» de Nuxt UI: fondo tenue, borde tenue y texto de color.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(10.5)
                    .color(color)
                    .family(semibold()),
            );
        });
}

/// Estado en forma de píldora con su punto.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let frame = Frame::new()
        .fill(color.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(4)
        .inner_margin(Margin {
            left: 22,
            right: 8,
            top: 2,
            bottom: 2,
        })
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(12.5)
                    .color(color)
                    .family(semibold()),
            )
        });
    // El punto se pinta en el margen: no depende del orden del layout (derecha a izquierda).
    let rect = frame.response.rect;
    ui.painter()
        .circle_filled(egui::pos2(rect.left() + 11.0, rect.center().y), 3.5, color);
    frame.response
}

/// Botón principal: relleno teal.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let p = pal(ui.ctx());
    let button = egui::Button::new(RichText::new(text).color(p.on_primary).family(semibold()))
        .fill(p.primary)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(0.0, 26.0));
    ui.add_enabled(enabled, button)
}

/// Botón de peligro: rojo.
pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    let p = pal(ui.ctx());
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).family(semibold()))
            .fill(p.error)
            .stroke(Stroke::NONE),
    )
}

/// Botón discreto sin borde (como `variant="ghost"`).
pub fn ghost_button(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(egui::Button::new(text).frame_when_inactive(false))
}

/// Botón de cerrar: una × dibujada (la fuente no trae el carácter).
pub fn close_button(ui: &mut Ui) -> Response {
    let p = pal(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), p.elevated);
    }
    let c = rect.center();
    let d = 4.5;
    let stroke = Stroke::new(1.5, if resp.hovered() { p.strong } else { p.muted });
    ui.painter()
        .line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
    ui.painter()
        .line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
    resp.on_hover_text("Cerrar")
}

/// Pestañas subrayadas (como `UTabs variant="link"`). Devuelve la elegida.
pub fn tabs<T: PartialEq + Copy>(ui: &mut Ui, current: &mut T, items: &[(T, String)]) {
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (value, label) in items {
            let active = *current == *value;
            let color = if active { p.strong } else { p.muted };
            let text = RichText::new(label)
                .size(13.5)
                .color(color)
                .family(semibold());
            let resp = ui.add(egui::Label::new(text).sense(Sense::click()));
            let underline = egui::Rect::from_min_max(
                egui::pos2(resp.rect.left(), resp.rect.bottom() + 5.0),
                egui::pos2(resp.rect.right(), resp.rect.bottom() + 7.0),
            );
            if active {
                ui.painter().rect_filled(underline, 1.0, p.primary_text);
            } else if resp.hovered() {
                ui.painter().rect_filled(underline, 1.0, p.border_accented);
            }
            if resp.clicked() {
                *current = *value;
            }
        }
    });
    ui.add_space(7.0);
    let y = ui.cursor().top();
    let rect = ui.max_rect();
    ui.painter()
        .hline(rect.left()..=rect.right(), y, Stroke::new(1.0, p.border));
    ui.add_space(8.0);
}
