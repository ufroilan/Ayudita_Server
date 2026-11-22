//! Modal de ajustes, en pestañas: General, Impresión y, solo tras pulsar Option/Alt + D
//! (o si ya está activo), Desarrollo.

use std::sync::Arc;

use eframe::egui::{self, Margin, RichText};

use super::theme::{self, close_button, pal, primary_button, setting_row, strong, toggle};
use crate::config::{Config, DEFAULT_BASE_URL, DEV_BASE_URL};
use crate::printer::SimFail;
use crate::state::{ConnCmd, Shared};
use crate::{autostart, dev};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Printing,
    Dev,
}

/// El estado del modal mientras está abierto.
pub struct Draft {
    tab: Tab,
    /// La pestaña de desarrollo se ha desbloqueado con el atajo.
    dev_unlocked: bool,
    /// La dirección de Ayudita que se está escribiendo y aún no se ha guardado.
    base_url: String,
}

impl Draft {
    pub fn new(cfg: &Config) -> Draft {
        Draft {
            tab: Tab::General,
            dev_unlocked: false,
            base_url: cfg.dev_base_url.clone(),
        }
    }

    /// Abierto con el atajo de teclado: directamente en Desarrollo.
    pub fn dev(cfg: &Config) -> Draft {
        Draft {
            tab: Tab::Dev,
            dev_unlocked: true,
            ..Draft::new(cfg)
        }
    }

    /// Enseña la pestaña Desarrollo sin ir a ella.
    pub fn show_dev_tab(&mut self) {
        self.dev_unlocked = true;
    }

    pub fn unlock_dev(&mut self) {
        self.dev_unlocked = true;
        self.tab = Tab::Dev;
    }
}

/// Dibuja el modal. Devuelve `false` cuando se cierra.
pub fn show(
    ctx: &egui::Context,
    shared: &Arc<Shared>,
    rt: &tokio::runtime::Handle,
    draft: &mut Draft,
) -> bool {
    let p = pal(ctx);
    let mut cfg = shared.cfg();
    let before = cfg.clone();
    let mut open = true;

    let resp = egui::Modal::new(egui::Id::new("ajustes"))
        .frame(theme::modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(600.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(strong("Ajustes").size(19.0).color(p.strong));
                    ui.label(
                        RichText::new("Cómo trabaja la app en este equipo.")
                            .size(12.5)
                            .color(p.muted),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if close_button(ui).clicked() {
                        open = false;
                    }
                });
            });
            ui.add_space(14.0);

            let mut tabs = vec![
                (Tab::General, "General".to_string()),
                (Tab::Printing, "Impresión".to_string()),
            ];
            if draft.dev_unlocked {
                tabs.push((Tab::Dev, "Desarrollo".to_string()));
            }
            theme::tabs(ui, &mut draft.tab, &tabs);
            ui.add_space(6.0);

            ui.allocate_ui(egui::vec2(ui.available_width(), 120.0), |ui| {
                ui.set_min_height(120.0);
                match draft.tab {
                    Tab::General => general(ui, &mut cfg),
                    Tab::Printing => printing(ui, &mut cfg),
                    Tab::Dev => development(ui, &p, &mut cfg, shared, rt, draft),
                }
            });
        });
    if resp.should_close() {
        open = false;
    }

    if cfg.autostart != before.autostart
        && let Err(e) = autostart::apply(cfg.autostart)
    {
        shared.notice(format!("No se pudo cambiar el arranque automático: {e}"));
    }
    let reconnect = cfg.base_url() != before.base_url();
    if cfg != before {
        shared.set_cfg(cfg);
    }
    if reconnect {
        shared.send(ConnCmd::Reload);
    }
    open
}

fn general(ui: &mut egui::Ui, cfg: &mut Config) {
    setting_row(
        ui,
        "Abrir al encender el ordenador",
        "La app arranca escondida en la bandeja y empieza a imprimir sola.",
        |ui| {
            toggle(ui, &mut cfg.autostart);
        },
    );
}

fn printing(ui: &mut egui::Ui, cfg: &mut Config) {
    setting_row(
        ui,
        "Copias de cada comanda",
        "Por ejemplo, 2 si una va a la cocina y otra a la barra.",
        |ui| {
            ui.add(
                egui::DragValue::new(&mut cfg.copies)
                    .range(1..=5)
                    .speed(0.05),
            );
        },
    );
    theme::divider(ui);
    setting_row(
        ui,
        "Imprimir precios",
        "El importe de cada producto y el total al pie de la comanda.",
        |ui| {
            toggle(ui, &mut cfg.show_prices);
        },
    );
}

fn development(
    ui: &mut egui::Ui,
    p: &theme::Pal,
    cfg: &mut Config,
    shared: &Arc<Shared>,
    rt: &tokio::runtime::Handle,
    draft: &mut Draft,
) {
    setting_row(
        ui,
        "Modo desarrollo",
        "Usa la dirección de Ayudita de abajo en vez de la de producción. Al desactivarlo vuelve a producción.",
        |ui| {
            toggle(ui, &mut cfg.dev_mode);
        },
    );
    if cfg.dev_mode {
        dev_url(ui, p, cfg, draft);
    }

    theme::divider(ui);
    setting_row(
        ui,
        "Impresora virtual",
        "Guarda cada comanda en archivos en vez de imprimirla. Se borran al salir.",
        |ui| {
            toggle(ui, &mut cfg.dev_virtual_printer);
        },
    );
    if cfg.dev_virtual_printer {
        let mut fail = *shared.dev.fail.lock().unwrap_or_else(|e| e.into_inner());
        let old = fail;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Simular fallo").size(12.5).color(p.muted));
            theme::segmented(
                ui,
                &mut fail,
                &[
                    (None, "Ninguno"),
                    (Some(SimFail::PaperOut), "Sin papel"),
                    (Some(SimFail::CoverOpen), "Tapa abierta"),
                    (Some(SimFail::Disconnected), "Desconectada"),
                ],
            );
        });
        if fail != old {
            *shared.dev.fail.lock().unwrap_or_else(|e| e.into_inner()) = fail;
            shared.wake_poller.notify_one();
        }
    }

    theme::divider(ui);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Comandas de ejemplo").clicked() {
            let shared = shared.clone();
            rt.spawn(async move { dev::inject(shared) });
        }
        if ui.button("Simular comandas perdidas").clicked() {
            dev::simulate_missed(shared);
        }
        if ui.button("Abrir carpeta de datos").clicked() {
            super::open_path(&shared.paths.data_dir);
        }
    });
}

fn dev_url(ui: &mut egui::Ui, p: &theme::Pal, cfg: &mut Config, draft: &mut Draft) {
    theme::divider(ui);
    ui.label(strong("Dirección de Ayudita").size(13.5).color(p.strong));
    ui.label(
        RichText::new(format!("En producción es {DEFAULT_BASE_URL}"))
            .size(12.0)
            .color(p.muted),
    );
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut draft.base_url)
                .hint_text(DEV_BASE_URL)
                .desired_width(ui.available_width() - 90.0)
                .margin(Margin::symmetric(10, 6)),
        );
        let changed = draft.base_url.trim().trim_end_matches('/') != cfg.dev_base_url;
        if primary_button(ui, "Guardar", changed).clicked() {
            let url = draft.base_url.trim().trim_end_matches('/');
            cfg.dev_base_url = if url.is_empty() {
                DEV_BASE_URL.into()
            } else {
                url.into()
            };
            draft.base_url = cfg.dev_base_url.clone();
        }
    });
}
