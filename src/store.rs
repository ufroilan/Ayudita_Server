//! Cola de impresión persistente, historial, deduplicación y `last_sent_at`.
//!
//! Todo vive en un único JSON en el directorio de datos. Una comanda pendiente nunca se borra:
//! del historial solo se recortan las terminadas (se guardan las 10 últimas).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{self, Comanda, EventKind};

pub const HISTORY_LEN: usize = 10;
const SEEN_DAYS: i64 = 14;
const BACKOFF_SECS: [i64; 5] = [3, 6, 12, 24, 30];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    /// Llegó por Realtime.
    Live,
    /// Llegó mientras la app estaba desconectada y el usuario la mandó imprimir.
    Recovered,
    /// «Reimprimir» desde la app.
    Reprint,
    /// «Página de prueba».
    TestPage,
    /// Comanda de ejemplo del modo desarrollador.
    Demo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Printing,
    Done {
        at: DateTime<Utc>,
        note: Option<String>,
    },
    Failed {
        error: String,
        attempts: u32,
        next_retry: DateTime<Utc>,
    },
    Discarded {
        at: DateTime<Utc>,
        reason: String,
    },
}

impl JobState {
    pub fn is_finished(&self) -> bool {
        matches!(self, JobState::Done { .. } | JobState::Discarded { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub received: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub source: Source,
    /// `None` en la página de prueba.
    pub comanda: Option<Comanda>,
    pub state: JobState,
    #[serde(default)]
    pub attempts: u32,
}

impl Job {
    pub fn is_copy(&self) -> bool {
        self.source == Source::Reprint
    }

    pub fn is_voided(&self) -> bool {
        self.comanda
            .as_ref()
            .is_some_and(|c| c.kind() == EventKind::Voided)
    }

    /// Título corto: `#42 · Terraza · Mesa 4`.
    pub fn title(&self) -> String {
        match &self.comanda {
            None => "Página de prueba".into(),
            Some(c) => {
                let place = c.place_label();
                if place.is_empty() {
                    c.seq_label()
                } else {
                    format!("{} · {}", c.seq_label(), place)
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskReason {
    /// Llegó mientras la app estaba desconectada.
    Missed,
    /// Anulación de una comanda que este PC no imprimió.
    VoidWithoutOriginal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ask {
    pub comanda: Comanda,
    pub reason: AskReason,
    pub selected: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Queued(u64),
    Duplicate,
    /// Este PC no imprime ese tipo de lugar.
    Filtered,
    /// Hay que preguntar antes de imprimir.
    Ask,
    /// Anulación de una comanda que seguía en la cola: no se imprime ninguna de las dos.
    CancelledBeforePrinting,
    Ignored,
}

pub enum Next {
    Ready(u64),
    WaitUntil(DateTime<Utc>),
    Idle,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub jobs: Vec<Job>,
    next_id: u64,
    /// Parejas (`order.id`, `event`) ya tratadas, con la hora en que se vieron.
    seen: HashMap<String, DateTime<Utc>>,
    /// Órdenes cuya comanda normal salió por la impresora de este PC.
    printed_orders: HashMap<String, DateTime<Utc>>,
    /// El `sent_at` más reciente recibido (texto tal cual, se devuelve en `?since=`).
    pub last_sent_at: Option<String>,
    pub last_received: Option<DateTime<Utc>>,
    /// Lo que espera confirmación del usuario.
    pub asks: Vec<Ask>,
    #[serde(skip)]
    path: PathBuf,
}

impl Store {
    pub fn load(path: &Path) -> Store {
        let mut store: Store = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        store.path = path.to_path_buf();
        // Si la app se cerró a mitad de una impresión, se vuelve a intentar; y lo que había fallado
        // se reintenta nada más arrancar.
        let now = Utc::now();
        for job in &mut store.jobs {
            match &mut job.state {
                JobState::Printing => job.state = JobState::Queued,
                JobState::Failed { next_retry, .. } => *next_retry = now,
                _ => {}
            }
        }
        store
    }

    pub fn save(&self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        if let Err(e) = crate::config::write_json_atomic(&self.path, self) {
            log::error!("No se pudo guardar la cola: {e}");
        }
    }

    /// Una comanda recibida por Realtime.
    pub fn receive(
        &mut self,
        c: Comanda,
        accepts_place: impl Fn(&str) -> bool,
        now: DateTime<Utc>,
    ) -> Outcome {
        self.note_sent_at(&c.sent_at);
        self.last_received = Some(now);
        let key = c.dedupe_key();
        if c.kind() == EventKind::Other || c.order.id.is_empty() {
            return Outcome::Ignored;
        }
        if self.seen.contains_key(&key) {
            return Outcome::Duplicate;
        }
        // Si estaba esperando confirmación (p. ej. vino también en `pending`), se trata ahora.
        self.asks.retain(|a| a.comanda.dedupe_key() != key);
        if !accepts_place(c.place_type()) {
            self.seen.insert(key, now);
            return Outcome::Filtered;
        }
        self.route(c, Source::Live, now)
    }

    /// Lo que devolvió `/api/pos/print/pending`. Devuelve cuántas quedan por confirmar.
    pub fn absorb_pending(
        &mut self,
        list: Vec<Comanda>,
        accepts_place: impl Fn(&str) -> bool,
        now: DateTime<Utc>,
    ) -> usize {
        for c in list {
            self.note_sent_at(&c.sent_at);
            let key = c.dedupe_key();
            if self.seen.contains_key(&key)
                || self.asks.iter().any(|a| a.comanda.dedupe_key() == key)
            {
                continue;
            }
            if !accepts_place(c.place_type()) {
                self.seen.insert(key, now);
                continue;
            }
            if c.kind() == EventKind::Voided && self.cancel_queued_original(&c, now) {
                continue;
            }
            self.asks.push(Ask {
                comanda: c,
                reason: AskReason::Missed,
                selected: true,
            });
        }
        self.asks.len()
    }

    /// Imprime las marcadas y descarta el resto.
    pub fn resolve_asks(&mut self, now: DateTime<Utc>) -> Vec<u64> {
        let asks = std::mem::take(&mut self.asks);
        let mut ids = Vec::new();
        for ask in asks {
            let key = ask.comanda.dedupe_key();
            self.seen.insert(key, now);
            if ask.selected {
                ids.push(self.push_job(Some(ask.comanda), Source::Recovered, now));
            }
        }
        self.trim(now);
        ids
    }

    pub fn discard_asks(&mut self, now: DateTime<Utc>) {
        for ask in &mut self.asks {
            ask.selected = false;
        }
        self.resolve_asks(now);
    }

    /// Primer inicio de sesión: lo anterior a este momento no se imprime.
    pub fn init_last_sent_at(&mut self, now: DateTime<Utc>) {
        if self.last_sent_at.is_none() {
            self.last_sent_at = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Millis, false));
        }
    }

    fn note_sent_at(&mut self, sent_at: &str) {
        let Some(t) = model::parse_time(sent_at) else {
            return;
        };
        let newer = match self.last_sent_at.as_deref().and_then(model::parse_time) {
            Some(prev) => t > prev,
            None => true,
        };
        if newer {
            self.last_sent_at = Some(sent_at.trim().to_string());
        }
    }

    fn route(&mut self, c: Comanda, source: Source, now: DateTime<Utc>) -> Outcome {
        let key = c.dedupe_key();
        let original_out = self.printed_orders.contains_key(&c.order.id)
            || self.jobs.iter().any(|j| {
                j.state == JobState::Printing
                    && j.comanda
                        .as_ref()
                        .is_some_and(|o| o.order.id == c.order.id && o.kind() == EventKind::Created)
            });
        if c.kind() == EventKind::Voided && !original_out {
            if self.cancel_queued_original(&c, now) {
                self.seen.insert(key, now);
                return Outcome::CancelledBeforePrinting;
            }
            self.asks.push(Ask {
                comanda: c,
                reason: AskReason::VoidWithoutOriginal,
                selected: true,
            });
            return Outcome::Ask;
        }
        self.seen.insert(key, now);
        Outcome::Queued(self.push_job(Some(c), source, now))
    }

    /// Si la comanda original de esta anulación sigue esperando en la cola, se retira y la
    /// anulación queda en el historial sin imprimirse.
    fn cancel_queued_original(&mut self, voided: &Comanda, now: DateTime<Utc>) -> bool {
        let pending = self.jobs.iter_mut().find(|j| {
            matches!(j.state, JobState::Queued | JobState::Failed { .. })
                && j.source != Source::Reprint
                && j.comanda.as_ref().is_some_and(|c| {
                    c.order.id == voided.order.id && c.kind() == EventKind::Created
                })
        });
        let Some(job) = pending else { return false };
        job.state = JobState::Discarded {
            at: now,
            reason: "Anulada antes de imprimirse".into(),
        };
        job.updated = now;
        let id = self.push_job(Some(voided.clone()), Source::Live, now);
        if let Some(job) = self.job_mut(id) {
            job.state = JobState::Discarded {
                at: now,
                reason: "No se imprime: la comanda original no llegó a salir".into(),
            };
        }
        self.seen.insert(voided.dedupe_key(), now);
        self.trim(now);
        true
    }

    pub fn push_job(
        &mut self,
        comanda: Option<Comanda>,
        source: Source,
        now: DateTime<Utc>,
    ) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.jobs.push(Job {
            id,
            received: now,
            updated: now,
            source,
            comanda,
            state: JobState::Queued,
            attempts: 0,
        });
        self.trim(now);
        id
    }

    pub fn reprint(&mut self, id: u64, now: DateTime<Utc>) -> Option<u64> {
        let comanda = self.job(id)?.comanda.clone();
        let source = if comanda.is_some() {
            Source::Reprint
        } else {
            Source::TestPage
        };
        Some(self.push_job(comanda, source, now))
    }

    pub fn job(&self, id: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    fn job_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// La siguiente a imprimir. Orden estricto: si la primera está esperando un reintento, las
    /// demás esperan detrás (así las comandas salen en el orden en que se pidieron).
    pub fn next(&self, now: DateTime<Utc>) -> Next {
        let head = self
            .jobs
            .iter()
            .find(|j| !j.state.is_finished() && j.state != JobState::Printing);
        match head {
            None => Next::Idle,
            Some(job) => match &job.state {
                JobState::Failed { next_retry, .. } if *next_retry > now => {
                    Next::WaitUntil(*next_retry)
                }
                _ => Next::Ready(job.id),
            },
        }
    }

    pub fn start(&mut self, id: u64, now: DateTime<Utc>) {
        if let Some(job) = self.job_mut(id) {
            job.state = JobState::Printing;
            job.updated = now;
        }
    }

    pub fn finish(&mut self, id: u64, result: Result<Option<String>, String>, now: DateTime<Utc>) {
        let Some(job) = self.job_mut(id) else { return };
        job.updated = now;
        // Si se descartó mientras se imprimía, se respeta.
        if matches!(job.state, JobState::Discarded { .. }) {
            return;
        }
        match result {
            Ok(note) => {
                job.state = JobState::Done { at: now, note };
                let printed_created = job
                    .comanda
                    .as_ref()
                    .filter(|c| c.kind() == EventKind::Created)
                    .map(|c| c.order.id.clone());
                if let Some(order_id) = printed_created {
                    self.printed_orders.insert(order_id, now);
                }
            }
            Err(error) => {
                job.attempts += 1;
                let wait = BACKOFF_SECS[(job.attempts as usize - 1).min(BACKOFF_SECS.len() - 1)];
                job.state = JobState::Failed {
                    error,
                    attempts: job.attempts,
                    next_retry: now + Duration::seconds(wait),
                };
            }
        }
        self.trim(now);
    }

    pub fn retry_now(&mut self, id: u64, now: DateTime<Utc>) {
        if let Some(job) = self.job_mut(id)
            && let JobState::Failed { next_retry, .. } = &mut job.state
        {
            *next_retry = now;
        }
    }

    pub fn retry_all_now(&mut self, now: DateTime<Utc>) {
        for job in &mut self.jobs {
            if let JobState::Failed { next_retry, .. } = &mut job.state {
                *next_retry = now;
            }
        }
    }

    pub fn discard(&mut self, id: u64, now: DateTime<Utc>) {
        if let Some(job) = self.job_mut(id)
            && !job.state.is_finished()
        {
            job.state = JobState::Discarded {
                at: now,
                reason: "Descartada a mano".into(),
            };
            job.updated = now;
        }
        self.trim(now);
    }

    pub fn failed_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| matches!(j.state, JobState::Failed { .. }))
            .count()
    }

    pub fn pending_count(&self) -> usize {
        self.jobs.iter().filter(|j| !j.state.is_finished()).count()
    }

    /// La impresión que se enseña arriba a la derecha: la que está saliendo, la que bloquea la cola
    /// por un error, o la última que terminó.
    pub fn current(&self) -> Option<&Job> {
        self.jobs
            .iter()
            .find(|j| j.state == JobState::Printing)
            .or_else(|| {
                self.jobs
                    .iter()
                    .find(|j| matches!(j.state, JobState::Failed { .. }))
            })
            .or_else(|| {
                self.jobs
                    .iter()
                    .filter(|j| !matches!(j.state, JobState::Queued))
                    .max_by_key(|j| (j.updated, j.id))
            })
    }

    fn trim(&mut self, now: DateTime<Utc>) {
        let mut finished = 0;
        let mut keep = vec![true; self.jobs.len()];
        for (i, job) in self.jobs.iter().enumerate().rev() {
            if job.state.is_finished() {
                finished += 1;
                keep[i] = finished <= HISTORY_LEN;
            }
        }
        let mut it = keep.into_iter();
        self.jobs.retain(|_| it.next().unwrap_or(true));
        let limit = now - Duration::days(SEEN_DAYS);
        self.seen.retain(|_, t| *t > limit);
        self.printed_orders.retain(|_, t| *t > limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::sample;
    use crate::model::{EV_CREATED, EV_VOIDED};

    fn all(_: &str) -> bool {
        true
    }

    fn voided(c: &Comanda) -> Comanda {
        let mut v = c.clone();
        v.event = EV_VOIDED.into();
        v.sent_at = "2026-10-15T20:30:00+00:00".into();
        v
    }

    fn print_head(s: &mut Store, now: DateTime<Utc>) -> u64 {
        let Next::Ready(id) = s.next(now) else {
            panic!("nada que imprimir")
        };
        s.start(id, now);
        s.finish(id, Ok(None), now);
        id
    }

    #[test]
    fn dedupes_by_order_and_event() {
        let mut s = Store::default();
        let now = Utc::now();
        assert!(matches!(s.receive(sample(), all, now), Outcome::Queued(_)));
        assert_eq!(s.receive(sample(), all, now), Outcome::Duplicate);
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn keeps_latest_sent_at() {
        let mut s = Store::default();
        let now = Utc::now();
        let mut a = sample();
        a.sent_at = "2026-10-15T20:00:00+00:00".into();
        let mut b = sample();
        b.order.id = "o2".into();
        b.sent_at = "2026-10-15T19:00:00+00:00".into();
        s.receive(a, all, now);
        s.receive(b, all, now);
        assert_eq!(s.last_sent_at.as_deref(), Some("2026-10-15T20:00:00+00:00"));
    }

    #[test]
    fn voided_after_printed_original_prints_directly() {
        let mut s = Store::default();
        let now = Utc::now();
        let c = sample();
        s.receive(c.clone(), all, now);
        print_head(&mut s, now);
        assert!(matches!(
            s.receive(voided(&c), all, now),
            Outcome::Queued(_)
        ));
        assert!(s.asks.is_empty());
    }

    #[test]
    fn voided_without_original_asks_first() {
        let mut s = Store::default();
        let now = Utc::now();
        assert_eq!(s.receive(voided(&sample()), all, now), Outcome::Ask);
        assert_eq!(s.asks.len(), 1);
        assert_eq!(s.asks[0].reason, AskReason::VoidWithoutOriginal);
        let ids = s.resolve_asks(now);
        assert_eq!(ids.len(), 1);
        assert_eq!(s.receive(voided(&sample()), all, now), Outcome::Duplicate);
    }

    #[test]
    fn voided_while_original_still_queued_cancels_both() {
        let mut s = Store::default();
        let now = Utc::now();
        let c = sample();
        s.receive(c.clone(), all, now);
        assert_eq!(
            s.receive(voided(&c), all, now),
            Outcome::CancelledBeforePrinting
        );
        assert_eq!(s.pending_count(), 0);
        assert!(matches!(s.next(now), Next::Idle));
    }

    #[test]
    fn pending_skips_already_seen_and_asks_for_the_rest() {
        let mut s = Store::default();
        let now = Utc::now();
        let a = sample();
        let mut b = sample();
        b.order.id = "o2".into();
        b.sent_at = "2026-10-15T21:00:00+00:00".into();
        s.receive(a.clone(), all, now);
        assert_eq!(s.absorb_pending(vec![a, b.clone()], all, now), 1);
        assert_eq!(s.asks[0].comanda.order.id, "o2");
        assert_eq!(s.last_sent_at.as_deref(), Some("2026-10-15T21:00:00+00:00"));
        // Si luego llega por Realtime, se imprime y deja de preguntarse.
        assert!(matches!(s.receive(b, all, now), Outcome::Queued(_)));
        assert!(s.asks.is_empty());
    }

    #[test]
    fn discarded_asks_are_not_asked_again() {
        let mut s = Store::default();
        let now = Utc::now();
        s.absorb_pending(vec![sample()], all, now);
        s.discard_asks(now);
        assert_eq!(s.absorb_pending(vec![sample()], all, now), 0);
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn failed_head_blocks_queue_with_backoff() {
        let mut s = Store::default();
        let now = Utc::now();
        let a = sample();
        let mut b = sample();
        b.order.id = "o2".into();
        s.receive(a, all, now);
        s.receive(b, all, now);
        let Next::Ready(first) = s.next(now) else {
            panic!()
        };
        s.start(first, now);
        s.finish(first, Err("Sin papel".into()), now);
        assert!(matches!(s.next(now), Next::WaitUntil(_)));
        let later = now + Duration::seconds(4);
        assert!(matches!(s.next(later), Next::Ready(id) if id == first));
        s.retry_now(first, now);
        assert!(matches!(s.next(now), Next::Ready(id) if id == first));
    }

    #[test]
    fn history_keeps_last_ten_finished_but_never_pending() {
        let mut s = Store::default();
        let now = Utc::now();
        for i in 0..15 {
            let mut c = sample();
            c.order.id = format!("o{i}");
            s.receive(c, all, now);
            print_head(&mut s, now);
        }
        let mut c = sample();
        c.order.id = "pendiente".into();
        s.receive(c, all, now);
        assert_eq!(
            s.jobs.iter().filter(|j| j.state.is_finished()).count(),
            HISTORY_LEN
        );
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn reprint_is_a_copy() {
        let mut s = Store::default();
        let now = Utc::now();
        s.receive(sample(), all, now);
        let id = print_head(&mut s, now);
        let copy = s.reprint(id, now).unwrap();
        assert!(s.job(copy).unwrap().is_copy());
    }

    #[test]
    fn place_filter() {
        let mut s = Store::default();
        let now = Utc::now();
        assert_eq!(
            s.receive(sample(), |t| t == "barra", now),
            Outcome::Filtered
        );
        assert_eq!(s.pending_count(), 0);
        let _ = EV_CREATED;
    }

    #[test]
    fn persists_and_resets_printing() {
        let dir = std::env::temp_dir().join(format!("ayudita-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("cola.json");
        let now = Utc::now();
        let mut s = Store::load(&path);
        s.receive(sample(), all, now);
        let Next::Ready(id) = s.next(now) else {
            panic!()
        };
        s.start(id, now);
        s.save();
        let s2 = Store::load(&path);
        assert_eq!(s2.job(id).unwrap().state, JobState::Queued);
        assert_eq!(s2.last_sent_at, s.last_sent_at);
        let _ = std::fs::remove_dir_all(dir);
    }
}
