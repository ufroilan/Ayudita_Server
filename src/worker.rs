//! Tareas de fondo de la impresora: el worker que vacía la cola y el sondeo de estado.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use crate::escpos;
use crate::model::Comanda;
use crate::printer::{self, PrintJob};
use crate::state::Shared;
use crate::store::{Next, Outcome, Source};
use crate::ticket::{self, Doc, Mark, Options};

const PRINT_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_EVERY: Duration = Duration::from_secs(5);

pub fn ticket_options(shared: &Shared) -> Options {
    let cfg = shared.cfg();
    Options {
        paper: cfg.paper,
        show_prices: cfg.show_prices,
    }
}

/// El documento que saldría por la impresora para una comanda.
pub fn doc_for(shared: &Shared, comanda: Option<&Comanda>, copy: bool) -> Doc {
    let opts = ticket_options(shared);
    match comanda {
        Some(c) => ticket::comanda(c, if copy { Mark::Copy } else { Mark::Original }, &opts),
        None => ticket::test_page(&opts, &shared.printer_label(), Utc::now()),
    }
}

/// Mete en la cola una comanda llegada por Realtime.
pub fn receive(shared: &Shared, comanda: Comanda) {
    let cfg = shared.cfg();
    let title = format!("{} {}", comanda.event, comanda.seq_label());
    let outcome = {
        let mut store = shared.store();
        let outcome = store.receive(comanda, |t| cfg.accepts_place(t), Utc::now());
        store.save();
        outcome
    };
    log::info!("Comanda recibida ({title}): {outcome:?}");
    match outcome {
        Outcome::Queued(_) => shared.wake_worker.notify_one(),
        Outcome::Ask => {
            shared.status().show_window = true;
        }
        _ => {}
    }
    shared.repaint();
}

pub fn enqueue(shared: &Shared, comanda: Option<Comanda>, source: Source) {
    {
        let mut store = shared.store();
        store.push_job(comanda, source, Utc::now());
        store.save();
    }
    shared.wake_worker.notify_one();
    shared.repaint();
}

pub async fn run_printer(shared: Arc<Shared>) {
    loop {
        let now = Utc::now();
        let paused = shared.status().paused;
        let next = if paused {
            Next::Idle
        } else {
            shared.store().next(now)
        };
        match next {
            Next::Idle => shared.wake_worker.notified().await,
            Next::WaitUntil(t) => {
                let wait = (t - now).to_std().unwrap_or_default();
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = shared.wake_worker.notified() => {}
                }
            }
            Next::Ready(id) => print_one(&shared, id).await,
        }
    }
}

async fn print_one(shared: &Shared, id: u64) {
    let job = {
        let mut store = shared.store();
        store.start(id, Utc::now());
        store.save();
        store.job(id).cloned()
    };
    shared.repaint();
    let Some(job) = job else { return };

    let cfg = shared.cfg();
    let doc = doc_for(shared, job.comanda.as_ref(), job.is_copy());
    let copies = if job.comanda.is_some() { cfg.copies } else { 1 };
    let bytes = escpos::render(&doc, copies);
    let preview = ticket::plain_text(&doc);
    let title = format!("Comanda {}", job.title());
    let target = shared.target();

    let result = {
        let _lock = shared.printer_lock.lock().await;
        let job = PrintJob {
            bytes: &bytes,
            title: &title,
            preview: &preview,
        };
        tokio::time::timeout(PRINT_TIMEOUT, printer::print(&target, job))
            .await
            .unwrap_or_else(|_| Err("La impresora tardó demasiado en responder".into()))
    };
    if let Err(e) = &result {
        log::warn!("Falló la impresión de {title}: {e}");
        shared.wake_poller.notify_one();
    }
    {
        let mut store = shared.store();
        store.finish(id, result, Utc::now());
        store.save();
    }
    shared.repaint();
}

/// Consulta el estado de la impresora cada pocos segundos. Si vuelve a estar bien después de un
/// problema, reintenta enseguida lo que había fallado.
pub async fn run_poller(shared: Arc<Shared>) {
    let mut had_problem = false;
    loop {
        let target = shared.target();
        let st = {
            let _lock = shared.printer_lock.lock().await;
            printer::status(&target).await
        };
        let problem = st.state.is_problem();
        shared.status().printer = st;
        if had_problem && !problem {
            shared.store().retry_all_now(Utc::now());
            shared.wake_worker.notify_one();
        }
        had_problem = problem;
        shared.repaint();
        tokio::select! {
            _ = tokio::time::sleep(POLL_EVERY) => {}
            _ = shared.wake_poller.notified() => {}
        }
    }
}

/// Vuelve a leer la lista de impresoras del sistema y USB.
pub async fn refresh_printers(shared: Arc<Shared>) {
    let found = tokio::task::spawn_blocking(|| (printer::list_system(), printer::list_usb())).await;
    if let Ok((system, usb)) = found {
        let mut st = shared.status();
        st.system_printers = system;
        st.usb_printers = usb;
    }
    shared.wake_poller.notify_one();
    shared.repaint();
}
