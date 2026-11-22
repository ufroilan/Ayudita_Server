//! La ventana: a la izquierda la cuenta de Ayudita y la impresora; a la derecha la comanda actual
//! y, en otra pestaña, la cola con el historial. Los ajustes (y el modo desarrollo) van en un
//! modal que se abre junto al estado de la barra superior.

mod preview;
mod settings;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Local, Utc};
use eframe::egui::{
    self, Align, Color32, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Ui, Vec2,
    ViewportCommand,
};

use crate::config::PrinterSel;
use crate::printer::PState;
use crate::state::{Conn, ConnCmd, Shared};
use crate::store::{AskReason, Job, JobState, Source};
use crate::ticket::Paper;
use crate::tray::{self, Tray};
use crate::worker;
use theme::{Pal, badge, ghost_button, pal, pill, primary_button, section, strong};

pub struct Options {
    pub start_hidden: bool,
    pub screenshot: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Current,
    Queue,
}

pub struct App {
    shared: Arc<Shared>,
    rt: tokio::runtime::Handle,
    email: String,
    password: String,
    tab: Tab,
    net_host: String,
    net_port: String,
    tray: Option<Tray>,
    quitting: bool,
    confirm_logout: bool,
    settings: Option<settings::Draft>,
    /// La pestaña Desarrollo se ha desbloqueado con el atajo (hasta cerrar la app).
    dev_unlocked: bool,
    screenshot: Option<(PathBuf, Instant)>,
    logo: egui::TextureHandle,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Shared>,
        rt: tokio::runtime::Handle,
        opts: Options,
    ) -> App {
        let ctx = &cc.egui_ctx;
        let _ = shared.ctx.set(ctx.clone());
        theme::setup(ctx);
        let tray = Tray::new(ctx);
        let cfg = shared.cfg();
        let (net_host, net_port) = match &cfg.printer {
            PrinterSel::Network { host, port } => (host.clone(), port.to_string()),
            _ => (String::new(), "9100".into()),
        };
        let email = shared.status().session.email.clone().unwrap_or_default();
        if opts.start_hidden {
            ctx.send_viewport_cmd(ViewportCommand::Visible(tray.is_none()));
        }
        let (w, h, rgba) = crate::icon::rgba(64);
        let logo = ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
            egui::TextureOptions::LINEAR,
        );
        App {
            shared,
            rt,
            email,
            password: String::new(),
            tab: Tab::Current,
            net_host,
            net_port,
            tray,
            quitting: false,
            confirm_logout: false,
            settings: None,
            dev_unlocked: false,
            screenshot: opts.screenshot.map(|p| (p, Instant::now())),
            logo,
        }
    }

    fn show_window(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    fn set_paused(&self, paused: bool) {
        self.shared.status().paused = paused;
        self.shared.wake_worker.notify_one();
        self.shared.repaint();
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Bandeja del sistema.
        let actions = self.tray.as_ref().map(|t| t.poll()).unwrap_or_default();
        for action in actions {
            match action {
                tray::Action::Open => self.show_window(ctx),
                tray::Action::TogglePause => {
                    let paused = self.shared.status().paused;
                    self.set_paused(!paused);
                }
                tray::Action::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        let (_, summary) = overall(&self.shared, &theme::LIGHT);
        let paused = self.shared.status().paused;
        if let Some(tray) = &mut self.tray {
            tray.sync(&format!("Ayudita Comandas · {summary}"), paused);
        }

        // Si hay algo que preguntar, se enseña la ventana.
        let show = std::mem::take(&mut self.shared.status().show_window);
        if show {
            self.show_window(ctx);
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }

        // Cerrar la ventana la esconde en la bandeja; «Salir» cierra de verdad.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if self.tray.is_some() && !cfg!(target_os = "linux") {
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            } else {
                ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
            }
        }

        // La pestaña Desarrollo solo aparece con Option + D (Alt + D en Windows y Linux).
        // Se mira la tecla física: en macOS, Option + D escribe «∂».
        let shortcut = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers }
                    if modifiers.alt && !modifiers.command
                        && (*physical_key == Some(egui::Key::D) || *key == egui::Key::D))
            })
        });
        if shortcut {
            self.dev_unlocked = true;
            match &mut self.settings {
                Some(draft) => draft.unlock_dev(),
                None => self.settings = Some(settings::Draft::dev(&self.shared.cfg())),
            }
        }

        // Captura de pantalla para desarrollo (`AYUDITA_CAPTURA=archivo.png`).
        self.screenshot_logic(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // Repinta cada segundo para las cuentas atrás («reintento en 12 s», «hace 3 min»).
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
        let p = pal(ui.ctx());

        egui::Panel::top("top")
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(14, 8))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| self.top_bar(ui, &p));
        if self.shared.dev_mode() {
            egui::Panel::top("dev")
                .frame(
                    Frame::new()
                        .fill(theme::tint(p.bg, p.warning, 0.12))
                        .inner_margin(Margin::symmetric(14, 4))
                        .stroke(Stroke::new(1.0, theme::tint(p.bg, p.warning, 0.35))),
                )
                .show(ui, |ui| self.dev_bar(ui, &p));
        }
        egui::Panel::bottom("bottom")
            .frame(
                Frame::new()
                    .fill(p.muted_bg)
                    .inner_margin(Margin::symmetric(14, 3))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| self.bottom_bar(ui, &p));
        egui::Panel::left("left")
            .resizable(false)
            .exact_size(320.0)
            .frame(
                Frame::new()
                    .fill(p.muted_bg)
                    .inner_margin(Margin::same(12))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt("left_scroll")
                    .show(ui, |ui| self.left(ui, &p));
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin {
                left: 16,
                right: 16,
                top: 10,
                bottom: 12,
            }))
            .show(ui, |ui| self.right(ui, &p));

        self.asks_modal(ui.ctx(), &p);
        self.logout_modal(ui.ctx(), &p);
        if let Some(draft) = &mut self.settings
            && !settings::show(ui.ctx(), &self.shared, &self.rt, draft)
        {
            self.settings = None;
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shared.store().save();
    }
}

// ---------------------------------------------------------------------------------------------
// Barras

impl App {
    fn top_bar(&mut self, ui: &mut Ui, p: &Pal) {
        ui.horizontal(|ui| {
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(22.0)));
            ui.label(strong("Ayudita Comandas").size(15.0).color(p.strong));
            let business = self
                .shared
                .status()
                .session
                .business
                .as_ref()
                .map(|b| b.name.clone());
            if let Some(name) = business {
                ui.label(RichText::new(format!("· {name}")).size(14.0).color(p.muted));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let gear = ghost_button(ui, RichText::new("⚙").size(15.0).color(p.muted))
                    .on_hover_text("Ajustes y modo desarrollo");
                if gear.clicked() {
                    let mut draft = settings::Draft::new(&self.shared.cfg());
                    if self.dev_unlocked {
                        draft.show_dev_tab();
                    }
                    self.settings = Some(draft);
                }
                let (color, text) = overall(&self.shared, p);
                pill(ui, &text, color);
            });
        });
    }

    fn dev_bar(&mut self, ui: &mut Ui, p: &Pal) {
        let cfg = self.shared.cfg();
        ui.horizontal_wrapped(|ui| {
            ui.label(strong("MODO DESARROLLO").size(11.0).color(p.warning));
            ui.label(
                RichText::new(format!("Ayudita: {}", cfg.base_url()))
                    .size(11.5)
                    .color(p.text),
            );
            if self.shared.virtual_printer() {
                ui.label(
                    RichText::new("· Impresora virtual (archivos)")
                        .size(11.5)
                        .color(p.text),
                );
            }
            // **Salir, de un clic, aquí mismo.** Sólo se podía en Ajustes → Desarrollo, una
            // pestaña que se esconde tras Option/Alt + D; y como la configuración sobrevive a
            // reinstalar, quien había probado la app en desarrollo la abría siempre así sin saber
            // cómo volver. Apaga las dos cosas que enciende el modo: la dirección y la impresora
            // virtual.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ghost_button(
                    ui,
                    RichText::new("Salir del modo desarrollo")
                        .size(11.5)
                        .color(p.warning),
                )
                .clicked()
                {
                    let mut next = cfg.clone();
                    next.dev_mode = false;
                    next.dev_virtual_printer = false;
                    let reconnect = next.base_url() != cfg.base_url();
                    self.shared.set_cfg(next);
                    if reconnect {
                        self.shared.send(ConnCmd::Reload);
                    }
                }
            });
        });
    }

    fn bottom_bar(&mut self, ui: &mut Ui, p: &Pal) {
        ui.horizontal(|ui| {
            let notice = self.shared.status().notice.clone();
            if let Some((text, at)) = notice {
                if at.elapsed().as_secs() < 30 {
                    ui.label(RichText::new(text).size(11.5).color(p.warning));
                } else {
                    self.shared.status().notice = None;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .color(p.dimmed),
                );
            });
        });
    }
}

// ---------------------------------------------------------------------------------------------
// Columna izquierda

impl App {
    fn left(&mut self, ui: &mut Ui, p: &Pal) {
        section(ui, "Cuenta de Ayudita");
        ui.add_space(2.0);
        self.account(ui, p);
        ui.add_space(18.0);
        section(ui, "Impresora");
        ui.add_space(2.0);
        self.printer(ui, p);
    }

    fn account(&mut self, ui: &mut Ui, p: &Pal) {
        let conn = self.shared.status().conn.clone();
        let session = self.shared.status().session.clone();
        if conn.needs_login() {
            self.login_form(ui, p, &conn);
            return;
        }

        let (color, state, detail) = conn_text(&conn, p);
        let business = session
            .business
            .as_ref()
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "Ayudita".into());
        let user = session
            .user
            .as_ref()
            .map(|u| u.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| session.email.clone());
        theme::titled_card(
            ui,
            &business,
            user.as_deref(),
            |ui| theme::chip(ui, &state, color),
            |ui| {
                // El detalle solo cuenta algo cuando no está conectado (p. ej. por qué reconecta).
                if let Some(detail) = detail
                    .as_ref()
                    .filter(|_| !matches!(conn, Conn::Connected { .. }))
                {
                    ui.add(
                        egui::Label::new(RichText::new(detail).size(12.0).color(p.muted)).wrap(),
                    );
                }
                if let Conn::Connected { since } = &conn {
                    theme::kv(ui, "Escuchando desde", &local_hm(*since));
                }
                let last = self.shared.store().last_received;
                theme::kv(
                    ui,
                    "Última comanda",
                    &last.map(ago).unwrap_or_else(|| "ninguna todavía".into()),
                );
                if let Some(email) = session
                    .email
                    .as_deref()
                    .filter(|e| user.as_deref() != Some(*e))
                {
                    theme::kv(ui, "Correo", email);
                }
                theme::divider(ui);
                ui.horizontal(|ui| {
                    if matches!(conn, Conn::Reconnecting { .. })
                        && ui.button("Reconectar ahora").clicked()
                    {
                        self.shared.send(ConnCmd::RetryNow);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ghost_button(
                            ui,
                            RichText::new("Cerrar sesión").size(12.5).color(p.muted),
                        )
                        .clicked()
                        {
                            self.confirm_logout = true;
                        }
                    });
                });
            },
        );
    }

    fn login_form(&mut self, ui: &mut Ui, p: &Pal, conn: &Conn) {
        let message = match conn {
            Conn::LoggedOut { message } => message.clone(),
            _ => None,
        };
        let busy = *conn == Conn::LoggingIn;
        theme::titled_card(
            ui,
            "Inicia sesión",
            Some("Con el correo y la contraseña del local"),
            |_| {},
            |ui| {
                if let Some(m) = message {
                    Frame::new()
                        .fill(p.error.gamma_multiply(0.10))
                        .stroke(Stroke::new(1.0, p.error.gamma_multiply(0.3)))
                        .corner_radius(4)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(RichText::new(m).color(p.error).size(12.5));
                        });
                }
                theme::field_label(ui, "Correo");
                let email = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.email)
                        .hint_text("tu@correo.com")
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(10, 7)),
                );
                theme::field_label(ui, "Contraseña");
                let pass = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.password)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(10, 7)),
                );
                ui.add_space(4.0);
                let enter = (email.lost_focus() || pass.lost_focus())
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let can = !busy && self.email.contains('@') && !self.password.is_empty();
                let label = if busy { "Entrando…" } else { "Entrar" };
                let button = egui::Button::new(strong(label).color(p.on_primary))
                    .fill(p.primary)
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(ui.available_width(), 32.0));
                if (ui.add_enabled(can, button).clicked() || enter) && can {
                    self.shared.send(ConnCmd::Login {
                        email: self.email.trim().to_string(),
                        password: std::mem::take(&mut self.password),
                    });
                }
            },
        );
    }

    fn printer(&mut self, ui: &mut Ui, p: &Pal) {
        let (printer, system, usb) = {
            let st = self.shared.status();
            (
                st.printer.clone(),
                st.system_printers.clone(),
                st.usb_printers.clone(),
            )
        };
        let mut cfg = self.shared.cfg();
        let before = cfg.clone();
        let virtual_printer = self.shared.virtual_printer();

        let (title, kind) = if virtual_printer {
            (
                "Impresora virtual".to_string(),
                "Guarda cada comanda en archivos".to_string(),
            )
        } else {
            match &cfg.printer {
                PrinterSel::Unset => ("Sin impresora".to_string(), "Elige una abajo".to_string()),
                PrinterSel::System { name } => {
                    (pretty_name(name), "Impresora del sistema".to_string())
                }
                PrinterSel::Network { host, port } => {
                    ("Impresora de red".to_string(), format!("{host}:{port}"))
                }
                PrinterSel::Usb { label, .. } => (label.clone(), "USB directo".to_string()),
                PrinterSel::File { dir } => ("Archivos".to_string(), dir.clone()),
            }
        };
        let color = pstate_color(printer.state, p);
        theme::titled_card(
            ui,
            &title,
            Some(&kind),
            |ui| theme::chip(ui, printer.state.label(), color),
            |ui| {
                // Un problema se ve como aviso, no como texto suelto.
                if (printer.state.is_problem() || printer.state == PState::PaperNearEnd)
                    && !printer.detail.is_empty()
                    && cfg.printer != PrinterSel::Unset
                {
                    Frame::new()
                        .fill(color.gamma_multiply(0.10))
                        .stroke(Stroke::new(1.0, color.gamma_multiply(0.30)))
                        .corner_radius(4)
                        .inner_margin(Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&printer.detail).size(12.5).color(color),
                                )
                                .wrap(),
                            );
                        });
                }

                if !virtual_printer {
                    // Propuesta: la predeterminada del sistema.
                    if cfg.printer == PrinterSel::Unset
                        && let Some(default) = &system.default
                    {
                        Frame::new()
                            .fill(p.primary.gamma_multiply(0.08))
                            .stroke(Stroke::new(1.0, p.primary.gamma_multiply(0.40)))
                            .corner_radius(4)
                            .inner_margin(Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 4.0;
                                ui.label(
                                    RichText::new("Predeterminada del sistema")
                                        .size(12.0)
                                        .color(p.muted),
                                );
                                ui.add(
                                    egui::Label::new(strong(pretty_name(default)).color(p.strong))
                                        .truncate(),
                                );
                                ui.add_space(2.0);
                                if primary_button(ui, "Usar esta impresora", true).clicked() {
                                    cfg.printer = PrinterSel::System {
                                        name: default.clone(),
                                    };
                                }
                            });
                    }

                    ui.horizontal(|ui| {
                        ui.label(strong("Impresora").size(12.0).color(p.text));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let search = ghost_button(
                                ui,
                                RichText::new("Buscar").size(12.0).color(p.primary_text),
                            )
                            .on_hover_text("Volver a buscar impresoras instaladas y USB");
                            if search.clicked() {
                                self.shared.status().printer =
                                    crate::printer::PrinterStatus::new(PState::Checking, "");
                                self.rt.spawn(worker::refresh_printers(self.shared.clone()));
                            }
                        });
                    });
                    ui.add_space(-4.0);
                    egui::ComboBox::from_id_salt("printer")
                        .width(ui.available_width())
                        .height(300.0)
                        .truncate()
                        .selected_text(match &cfg.printer {
                            PrinterSel::Unset => "Elige la impresora".to_string(),
                            PrinterSel::Network { .. } => "Impresora de red (IP)".to_string(),
                            PrinterSel::System { name } => pretty_name(name),
                            other => other.label(),
                        })
                        .show_ui(ui, |ui| {
                            for name in &system.names {
                                let mut label = pretty_name(name);
                                if system.default.as_ref() == Some(name) {
                                    label.push_str("  · predeterminada");
                                }
                                let sel = PrinterSel::System { name: name.clone() };
                                if ui.selectable_label(cfg.printer == sel, label).clicked() {
                                    cfg.printer = sel;
                                }
                            }
                            for u in &usb {
                                let sel = PrinterSel::Usb {
                                    vendor_id: u.vendor_id,
                                    product_id: u.product_id,
                                    serial: u.serial.clone(),
                                    label: u.label.clone(),
                                };
                                if ui
                                    .selectable_label(
                                        cfg.printer == sel,
                                        format!("USB directo · {}", u.label),
                                    )
                                    .clicked()
                                {
                                    cfg.printer = sel;
                                }
                            }
                            let is_net = matches!(cfg.printer, PrinterSel::Network { .. });
                            if ui
                                .selectable_label(is_net, "Impresora de red (IP)…")
                                .clicked()
                                && !is_net
                            {
                                cfg.printer = PrinterSel::Network {
                                    host: self.net_host.clone(),
                                    port: self.net_port.parse().unwrap_or(9100),
                                };
                            }
                            if system.names.is_empty() && usb.is_empty() {
                                ui.label(
                                    RichText::new("No se encontró ninguna impresora instalada")
                                        .color(p.muted),
                                );
                            }
                        });

                    if let PrinterSel::Network { host, port } = &mut cfg.printer {
                        theme::field_label(ui, "Dirección IP y puerto");
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.add(
                                egui::TextEdit::singleline(&mut self.net_host)
                                    .hint_text("192.168.1.50")
                                    .desired_width(ui.available_width() - 68.0)
                                    .margin(Margin::symmetric(8, 5)),
                            );
                            ui.add(
                                egui::TextEdit::singleline(&mut self.net_port)
                                    .desired_width(ui.available_width())
                                    .margin(Margin::symmetric(8, 5)),
                            );
                        });
                        *host = self.net_host.trim().to_string();
                        *port = self.net_port.trim().parse().unwrap_or(9100);
                    }
                    ui.add_space(2.0);
                }

                theme::field_label(ui, "Ancho del papel");
                theme::segmented(
                    ui,
                    &mut cfg.paper,
                    &[(Paper::Mm80, "80 mm"), (Paper::Mm58, "58 mm")],
                );

                ui.add_space(4.0);
                let test = egui::Button::new(RichText::new("Imprimir página de prueba").size(13.0))
                    .min_size(Vec2::new(ui.available_width(), 30.0));
                if ui.add(test).clicked() {
                    worker::enqueue(&self.shared, None, Source::TestPage);
                    self.tab = Tab::Current;
                }
            },
        );
        if cfg != before {
            self.shared.set_cfg(cfg);
        }
    }
}

/// Nombre de impresora para leer: CUPS usa guiones bajos en vez de espacios.
fn pretty_name(name: &str) -> String {
    name.split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------------------------
// Columna derecha

impl App {
    fn right(&mut self, ui: &mut Ui, p: &Pal) {
        let (jobs, current_id, failed, pending) = {
            let store = self.shared.store();
            (
                store.jobs.clone(),
                store.current().map(|j| j.id),
                store.failed_count(),
                store.pending_count(),
            )
        };
        let queue_label = match (pending, failed) {
            (0, _) => "Cola e historial".to_string(),
            (n, 0) => format!("Cola e historial · {n}"),
            (n, f) => format!("Cola e historial · {n} ({f} con error)"),
        };
        theme::tabs(
            ui,
            &mut self.tab,
            &[
                (Tab::Current, "Comanda actual".into()),
                (Tab::Queue, queue_label),
            ],
        );

        match self.tab {
            Tab::Current => {
                let job = current_id
                    .and_then(|id| jobs.iter().find(|j| j.id == id))
                    .cloned();
                self.current_view(ui, p, job.as_ref());
            }
            Tab::Queue => self.queue_view(ui, p, &jobs, failed, pending),
        }
    }

    fn current_view(&mut self, ui: &mut Ui, p: &Pal, job: Option<&Job>) {
        let Some(job) = job else {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    strong("Todavía no se ha impreso nada")
                        .size(15.0)
                        .color(p.strong),
                );
                ui.label(
                    RichText::new("Las comandas aparecen aquí en cuanto se piden en Ayudita.")
                        .color(p.muted),
                );
            });
            return;
        };
        // Encima de la comanda, solo su estado y los botones: lo demás ya se ve en el papel.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.job_actions(ui, job, true);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let (color, text) = state_text(job, p, true);
                    if job.state == JobState::Printing {
                        ui.spinner();
                    }
                    let text = format!("{text} · recibida a las {}", local_hms(job.received));
                    ui.add(egui::Label::new(strong(&text).color(color)).truncate())
                        .on_hover_text(text);
                });
            });
        });
        ui.add_space(8.0);
        let doc = worker::doc_for(&self.shared, job.comanda.as_ref(), job.is_copy());
        // La comanda entera, lo más grande que quepa a lo ancho y a lo alto.
        let size = ui.available_size();
        ui.vertical_centered(|ui| {
            preview::paper_fit(ui, &doc, size.x, size.y - 4.0);
        });
    }

    fn queue_view(&mut self, ui: &mut Ui, p: &Pal, jobs: &[Job], failed: usize, pending: usize) {
        let paused = self.shared.status().paused;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{pending} en cola · se guardan las {} últimas",
                    crate::store::HISTORY_LEN
                ))
                .color(p.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button(if paused { "▶ Reanudar" } else { "⏸ Pausar" })
                    .clicked()
                {
                    self.set_paused(!paused);
                }
                if failed > 0 && ui.small_button("Reintentar fallidas").clicked() {
                    self.shared.store().retry_all_now(Utc::now());
                    self.shared.wake_worker.notify_one();
                }
            });
        });
        if paused {
            ui.label(
                RichText::new("La impresión está en pausa: las comandas esperan en la cola.")
                    .color(p.warning),
            );
        }
        ui.add_space(6.0);

        // Sin vista previa: cada fila lleva el importe, que basta para reconocer la comanda.
        ScrollArea::vertical()
            .id_salt("jobs")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if jobs.is_empty() {
                    ui.label(RichText::new("No hay nada en la cola.").color(p.muted));
                }
                for job in jobs.iter().rev() {
                    self.job_row(ui, p, job);
                    ui.add_space(4.0);
                }
            });
    }

    /// Botones de una comanda: Reimprimir, Reintentar, Descartar. Para un layout derecha a izquierda.
    fn job_actions(&mut self, ui: &mut Ui, job: &Job, big: bool) {
        let now = Utc::now();
        let button = |ui: &mut Ui, text: &str| {
            if big {
                ui.button(text)
            } else {
                ui.small_button(text)
            }
        };
        if !job.state.is_finished()
            && job.state != JobState::Printing
            && button(ui, "Descartar").clicked()
        {
            let mut store = self.shared.store();
            store.discard(job.id, now);
            store.save();
        }
        if matches!(job.state, JobState::Failed { .. }) && button(ui, "Reintentar").clicked() {
            self.shared.store().retry_now(job.id, now);
            self.shared.wake_worker.notify_one();
        }
        if matches!(
            job.state,
            JobState::Done { .. } | JobState::Discarded { .. }
        ) && (job.comanda.is_some() || job.source == Source::TestPage)
            && button(ui, "Reimprimir").clicked()
        {
            {
                let mut store = self.shared.store();
                store.reprint(job.id, now);
                store.save();
            }
            self.shared.wake_worker.notify_one();
        }
    }

    fn job_row(&mut self, ui: &mut Ui, p: &Pal, job: &Job) {
        Frame::new()
            .fill(p.bg)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(4)
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(local_hm(job.received))
                            .monospace()
                            .size(11.5)
                            .color(p.dimmed),
                    );
                    badges(ui, job, p);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(price) = job.comanda.as_ref().and_then(total_label) {
                            ui.label(strong(price).color(p.strong));
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(strong(job.title()).color(p.strong)).truncate(),
                            );
                        });
                    });
                });
                ui.horizontal(|ui| {
                    // Primero los botones (a la derecha); el estado ocupa lo que queda y se recorta.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.job_actions(ui, job, false);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let (color, text) = state_text(job, p, false);
                            if job.state == JobState::Printing {
                                ui.spinner();
                            }
                            ui.add(
                                egui::Label::new(RichText::new(&text).color(color).size(12.0))
                                    .truncate(),
                            )
                            .on_hover_text(text);
                        });
                    });
                });
            });
    }

    fn asks_modal(&mut self, ctx: &egui::Context, p: &Pal) {
        let asks = self.shared.store().asks.clone();
        if asks.is_empty() {
            return;
        }
        let missed = asks
            .iter()
            .filter(|a| a.reason == AskReason::Missed)
            .count();
        let voids = asks.len() - missed;
        let title = if missed > 0 {
            match asks.len() {
                1 => "Mientras estabas desconectado llegó 1 comanda".to_string(),
                n => format!("Mientras estabas desconectado llegaron {n} comandas"),
            }
        } else if voids == 1 {
            "Llegó una anulación de una comanda que este equipo no imprimió".to_string()
        } else {
            format!("Llegaron {voids} anulaciones de comandas que este equipo no imprimió")
        };
        let mut print = false;
        let mut discard = false;
        let mut toggles = Vec::new();
        let screen = ctx.content_rect();
        egui::Modal::new(egui::Id::new("asks"))
            .frame(theme::modal_frame(ctx))
            .show(ctx, |ui| {
                ui.set_max_width((screen.width() - 80.0).max(400.0));
                ui.label(strong(&title).size(16.0).color(p.strong));
                ui.label(
                    RichText::new("Las marcadas se imprimen; las demás se descartan.")
                        .color(p.muted),
                );
                ui.add_space(8.0);
                let card_w = 280.0;
                ScrollArea::horizontal()
                    .id_salt("asks_scroll")
                    .max_height((screen.height() - 200.0).max(250.0))
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            for (i, ask) in asks.iter().enumerate() {
                                Frame::new()
                                    .fill(p.muted_bg)
                                    .stroke(Stroke::new(1.0, p.border))
                                    .corner_radius(4)
                                    .inner_margin(8)
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width(card_w);
                                            let mut checked = ask.selected;
                                            if ui
                                                .checkbox(&mut checked, strong("Imprimir"))
                                                .changed()
                                            {
                                                toggles.push((i, checked));
                                            }
                                            let why = match ask.reason {
                                                AskReason::Missed => {
                                                    "Llegó mientras la app estaba desconectada"
                                                }
                                                AskReason::VoidWithoutOriginal => {
                                                    "Anulación: la original no salió en este equipo"
                                                }
                                            };
                                            ui.label(RichText::new(why).size(11.5).color(p.muted));
                                            ui.add_space(4.0);
                                            ScrollArea::vertical()
                                                .id_salt(("ask", i))
                                                .max_height((screen.height() - 310.0).max(200.0))
                                                .show(ui, |ui| {
                                                    let doc = worker::doc_for(
                                                        &self.shared,
                                                        Some(&ask.comanda),
                                                        false,
                                                    );
                                                    preview::paper(ui, &doc, card_w - 8.0);
                                                });
                                        });
                                    });
                            }
                        });
                    });
                ui.add_space(10.0);
                let marked = asks.iter().filter(|a| a.selected).count();
                ui.horizontal(|ui| {
                    if primary_button(ui, &format!("Imprimir las marcadas ({marked})"), marked > 0)
                        .clicked()
                    {
                        print = true;
                    }
                    if ui.button("Descartar").clicked() {
                        discard = true;
                    }
                });
            });
        let now = Utc::now();
        let mut store = self.shared.store();
        for (i, checked) in toggles {
            if let Some(a) = store.asks.get_mut(i) {
                a.selected = checked;
            }
        }
        if print {
            store.resolve_asks(now);
        } else if discard {
            store.discard_asks(now);
        }
        if print || discard {
            store.save();
            drop(store);
            self.shared.wake_worker.notify_one();
        }
    }

    fn logout_modal(&mut self, ctx: &egui::Context, p: &Pal) {
        if !self.confirm_logout {
            return;
        }
        egui::Modal::new(egui::Id::new("logout")).frame(theme::modal_frame(ctx)).show(ctx, |ui| {
            ui.set_max_width(340.0);
            ui.label(strong("¿Cerrar sesión?").size(16.0).color(p.strong));
            ui.label(
                RichText::new(
                    "Este equipo dejará de recibir comandas hasta que vuelvas a entrar. Lo que ya está en la cola se sigue imprimiendo.",
                )
                .color(p.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if theme::danger_button(ui, "Cerrar sesión").clicked() {
                    self.shared.send(ConnCmd::Logout);
                    self.confirm_logout = false;
                }
                if ui.button("Cancelar").clicked() {
                    self.confirm_logout = false;
                }
            });
        });
    }

    fn screenshot_logic(&mut self, ctx: &egui::Context) {
        let Some((path, started)) = &self.screenshot else {
            return;
        };
        // Desarrollo: abrir una pestaña o los ajustes antes de la captura.
        if let Ok(view) = std::env::var("AYUDITA_CAPTURA_VISTA") {
            match view.as_str() {
                "cola" => self.tab = Tab::Queue,
                "ajustes" if self.settings.is_none() => {
                    self.settings = Some(settings::Draft::new(&self.shared.cfg()))
                }
                "desarrollo" if self.settings.is_none() => {
                    self.settings = Some(settings::Draft::dev(&self.shared.cfg()))
                }
                _ => {}
            }
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let path = path.clone();
            match save_png(&path, &image) {
                Ok(()) => log::info!("Captura guardada en {}", path.display()),
                Err(e) => log::error!("No se pudo guardar la captura: {e}"),
            }
            self.screenshot = None;
            if std::env::var("AYUDITA_CAPTURA_SALIR").is_ok_and(|v| v == "1") {
                self.quitting = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return;
        }
        let delay = std::env::var("AYUDITA_CAPTURA_ESPERA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3.0);
        if started.elapsed().as_secs_f32() > delay {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            self.screenshot = Some((
                path.clone(),
                Instant::now() + std::time::Duration::from_secs(3600),
            ));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Textos y colores de estado

fn badges(ui: &mut Ui, job: &Job, p: &Pal) {
    let (text, color) = if job.comanda.is_none() {
        ("PRUEBA", p.neutral)
    } else if job.is_voided() {
        ("ANULADA", p.error)
    } else {
        ("NUEVA", p.primary_text)
    };
    badge(ui, text, color);
    match job.source {
        Source::Reprint => badge(ui, "COPIA", p.info),
        Source::Recovered => badge(ui, "RECUPERADA", p.info),
        Source::Demo => badge(ui, "DEMO", p.neutral),
        _ => {}
    }
    if let Some(c) = &job.comanda {
        match c.order.kind.as_str() {
            "cortesia" => badge(ui, "CORTESÍA", p.warning),
            "cuenta" => badge(ui, "CUENTA", p.warning),
            _ => {}
        }
    }
}

/// El importe de la orden con su moneda (el total, o la suma de las líneas si no viene).
fn total_label(c: &crate::model::Comanda) -> Option<String> {
    let total = c.order.total.or_else(|| {
        let lines: Vec<f64> = c.order.lines.iter().filter_map(|l| l.line_total).collect();
        (!lines.is_empty()).then(|| lines.iter().sum())
    })?;
    Some(crate::ticket::fmt_money(total, c.business.currency.trim()))
}

fn state_text(job: &Job, p: &Pal, long: bool) -> (Color32, String) {
    let now = Utc::now();
    match &job.state {
        JobState::Queued => (p.muted, "En cola".into()),
        JobState::Printing => (p.info, "Imprimiendo…".into()),
        JobState::Done { at, .. } => (p.success, format!("✔ Impresa a las {}", local_hms(*at))),
        JobState::Failed {
            error,
            attempts,
            next_retry,
        } => {
            let secs = (*next_retry - now).num_seconds().max(0);
            let text = if long {
                format!(
                    "✖ {error} · se reintenta sola en {secs} s (intento {attempts}); no se pierde"
                )
            } else {
                format!("✖ {error} · reintento en {secs} s")
            };
            (p.error, text)
        }
        JobState::Discarded { reason, .. } => (p.muted, reason.clone()),
    }
}

fn pstate_color(state: PState, p: &Pal) -> Color32 {
    match state {
        PState::Ready | PState::Unknown | PState::Busy => p.success,
        PState::PaperNearEnd | PState::Checking => p.warning,
        _ => p.error,
    }
}

fn conn_text(conn: &Conn, p: &Pal) -> (Color32, String, Option<String>) {
    match conn {
        Conn::Connected { since } => (
            p.success,
            "Conectado".into(),
            Some(format!(
                "Escuchando comandas desde las {}",
                local_hm(*since)
            )),
        ),
        Conn::Connecting => (p.warning, "Conectando…".into(), None),
        Conn::Reconnecting {
            at,
            attempt,
            reason,
        } => {
            let secs = at.saturating_duration_since(Instant::now()).as_secs();
            (
                p.warning,
                format!("Reconectando en {secs} s"),
                Some(format!("Intento {attempt}. {reason}")),
            )
        }
        Conn::LoggingIn => (p.warning, "Entrando…".into(), None),
        Conn::LoggedOut { .. } => (p.error, "Sin sesión".into(), None),
    }
}

/// El peor estado de todos, para la barra superior y la bandeja.
pub fn overall(shared: &Shared, p: &Pal) -> (Color32, String) {
    let (conn, printer, paused) = {
        let st = shared.status();
        (st.conn.clone(), st.printer.clone(), st.paused)
    };
    let (failed, asks) = {
        let store = shared.store();
        (store.failed_count(), store.asks.len())
    };
    match conn {
        Conn::LoggedOut { message: Some(_) } => {
            return (p.error, "Sesión caducada: vuelve a entrar".into());
        }
        Conn::LoggedOut { .. } | Conn::LoggingIn => return (p.error, "Sin sesión".into()),
        _ => {}
    }
    if printer.state.is_problem() {
        return (p.error, format!("Impresora: {}", printer.state.label()));
    }
    if failed > 0 {
        return (
            p.error,
            if failed == 1 {
                "1 comanda sin imprimir".into()
            } else {
                format!("{failed} comandas sin imprimir")
            },
        );
    }
    match conn {
        Conn::Reconnecting { .. } => return (p.warning, "Reconectando…".into()),
        Conn::Connecting => return (p.warning, "Conectando…".into()),
        _ => {}
    }
    if asks > 0 {
        return (p.warning, "Comandas esperando confirmación".into());
    }
    if paused {
        return (p.warning, "Impresión en pausa".into());
    }
    if printer.state == PState::PaperNearEnd {
        return (p.warning, "Queda poco papel".into());
    }
    (p.success, "Conectado y listo".into())
}

fn ago(t: DateTime<Utc>) -> String {
    let secs = (Utc::now() - t).num_seconds().max(0);
    match secs {
        0..=59 => format!("hace {secs} s"),
        60..=3599 => format!("hace {} min", secs / 60),
        _ => format!("el {}", t.with_timezone(&Local).format("%d/%m a las %H:%M")),
    }
}

fn local_hm(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M").to_string()
}

fn local_hms(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M:%S").to_string()
}

pub(crate) fn open_path(path: &std::path::Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.size[0] as u32,
        image.size[1] as u32,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(|e| e.to_string())
}
