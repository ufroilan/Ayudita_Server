//! Icono en la bandeja del sistema: abrir la ventana, pausar la impresión y salir.

use std::sync::mpsc;

use eframe::egui;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum Action {
    Open,
    TogglePause,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem,
    pause: CheckMenuItem,
    quit: MenuItem,
    rx: mpsc::Receiver<Event>,
    last_tooltip: String,
}

enum Event {
    Menu(MenuEvent),
    Icon(TrayIconEvent),
}

impl Tray {
    pub fn new(ctx: &egui::Context) -> Option<Tray> {
        let open = MenuItem::new("Abrir Ayudita Comandas", true, None);
        let pause = CheckMenuItem::new("Pausar la impresión", true, false, None);
        let quit = MenuItem::new("Salir", true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &pause, &PredefinedMenuItem::separator(), &quit])
            .ok()?;
        let (w, h, rgba) = crate::icon::rgba(64);
        let icon = tray_icon::Icon::from_rgba(rgba, w, h).ok()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Ayudita Comandas")
            .with_icon(icon)
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
            .map_err(|e| log::warn!("Sin icono en la bandeja: {e}"))
            .ok()?;

        // Los eventos llegan aunque la ventana esté oculta: se despierta a la app para tratarlos.
        let (tx, rx) = mpsc::channel();
        let (tx2, ctx2) = (tx.clone(), ctx.clone());
        let ctx1 = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = tx.send(Event::Menu(e));
            ctx1.request_repaint();
        }));
        TrayIconEvent::set_event_handler(Some(move |e| {
            let _ = tx2.send(Event::Icon(e));
            ctx2.request_repaint();
        }));
        Some(Tray {
            icon: tray,
            open,
            pause,
            quit,
            rx,
            last_tooltip: String::new(),
        })
    }

    pub fn poll(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Menu(e) if e.id == *self.open.id() => actions.push(Action::Open),
                Event::Menu(e) if e.id == *self.pause.id() => actions.push(Action::TogglePause),
                Event::Menu(e) if e.id == *self.quit.id() => actions.push(Action::Quit),
                Event::Icon(TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                })
                | Event::Icon(TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }) if !cfg!(target_os = "macos") => actions.push(Action::Open),
                _ => {}
            }
        }
        actions
    }

    pub fn sync(&mut self, tooltip: &str, paused: bool) {
        if self.pause.is_checked() != paused {
            self.pause.set_checked(paused);
        }
        if self.last_tooltip != tooltip {
            let _ = self.icon.set_tooltip(Some(tooltip));
            self.last_tooltip = tooltip.to_string();
        }
    }
}
