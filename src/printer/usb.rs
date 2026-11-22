//! USB directo, sin controlador, con `nusb`.
//!
//! Funciona en macOS y Linux sin instalar nada (en Linux hace falta permiso sobre el dispositivo).
//! En Windows solo si el dispositivo usa el controlador WinUSB; lo normal allí es instalar la
//! impresora en el sistema y elegirla como «impresora del sistema».

use std::io::{Read, Write};
use std::time::Duration;

use nusb::MaybeFuture;
use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, In, Out};

use super::{PState, PrinterStatus};
use crate::escpos::{STATUS_QUERY, parse_status};

const PRINTER_CLASS: u8 = 7;

#[derive(Clone, Debug, PartialEq)]
pub struct UsbPrinter {
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial: Option<String>,
    pub label: String,
}

pub fn list() -> Vec<UsbPrinter> {
    let Ok(devices) = nusb::list_devices().wait() else {
        return Vec::new();
    };
    devices
        .filter(|d| {
            d.class() == PRINTER_CLASS || d.interfaces().any(|i| i.class() == PRINTER_CLASS)
        })
        .map(|d| {
            let name = [d.manufacturer_string(), d.product_string()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            let label = if name.trim().is_empty() {
                format!("{:04x}:{:04x}", d.vendor_id(), d.product_id())
            } else {
                name
            };
            UsbPrinter {
                vendor_id: d.vendor_id(),
                product_id: d.product_id(),
                serial: d.serial_number().map(str::to_string),
                label,
            }
        })
        .collect()
}

struct Opened {
    out: nusb::Endpoint<Bulk, Out>,
    input: Option<nusb::Endpoint<Bulk, In>>,
}

fn open(vendor_id: u16, product_id: u16, serial: Option<&str>) -> Result<Opened, String> {
    let devices = nusb::list_devices().wait().map_err(|e| e.to_string())?;
    let info = devices
        .filter(|d| d.vendor_id() == vendor_id && d.product_id() == product_id)
        .find(|d| serial.is_none() || d.serial_number() == serial)
        .ok_or("La impresora USB no está conectada")?;
    let number = info
        .interfaces()
        .find(|i| i.class() == PRINTER_CLASS)
        .map(|i| i.interface_number())
        .unwrap_or(0);
    let device = info
        .open()
        .wait()
        .map_err(|e| format!("No se puede abrir la impresora USB: {e}"))?;
    let iface = device
        .detach_and_claim_interface(number)
        .wait()
        .map_err(|e| format!("La impresora USB está ocupada por otro programa: {e}"))?;
    let desc = iface
        .descriptor()
        .ok_or("La impresora USB no describe sus puertos")?;
    let mut out_addr = None;
    let mut in_addr = None;
    for ep in desc.endpoints() {
        if ep.transfer_type() != TransferType::Bulk {
            continue;
        }
        match ep.direction() {
            Direction::Out if out_addr.is_none() => out_addr = Some(ep.address()),
            Direction::In if in_addr.is_none() => in_addr = Some(ep.address()),
            _ => {}
        }
    }
    let out_addr = out_addr.ok_or("La impresora USB no tiene puerto de datos")?;
    let out = iface
        .endpoint::<Bulk, Out>(out_addr)
        .map_err(|e| e.to_string())?;
    let input = in_addr.and_then(|a| iface.endpoint::<Bulk, In>(a).ok());
    Ok(Opened { out, input })
}

fn query(opened: Opened) -> (nusb::Endpoint<Bulk, Out>, Option<PrinterStatus>) {
    let Opened { out, input } = opened;
    let Some(input) = input else {
        return (out, None);
    };
    let mut writer = out.writer(64);
    writer.set_write_timeout(Duration::from_secs(2));
    if writer
        .write_all(&STATUS_QUERY)
        .and_then(|_| writer.flush_end())
        .is_err()
    {
        return (writer.into_inner(), None);
    }
    let mut reader = input.reader(64);
    reader.set_read_timeout(Duration::from_millis(800));
    let mut got = Vec::new();
    let mut buf = [0u8; 64];
    while got.len() < 3 {
        match reader.read(&mut buf) {
            Ok(n) if n > 0 => got.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    (
        writer.into_inner(),
        parse_status(&got).map(PrinterStatus::from),
    )
}

pub async fn print(
    vendor_id: u16,
    product_id: u16,
    serial: Option<String>,
    bytes: Vec<u8>,
) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || {
        let opened = open(vendor_id, product_id, serial.as_deref())?;
        let (out, status) = query(opened);
        if let Some(st) = status
            && st.state.is_problem()
        {
            return Err(format!("{}: {}", st.state.label(), st.detail));
        }
        let mut writer = out.writer(4096);
        writer.set_write_timeout(Duration::from_secs(15));
        writer
            .write_all(&bytes)
            .map_err(|e| format!("Error al enviar a la impresora USB: {e}"))?;
        writer
            .flush_end()
            .map_err(|e| format!("Error al enviar a la impresora USB: {e}"))?;
        Ok(None)
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn status(vendor_id: u16, product_id: u16, serial: Option<String>) -> PrinterStatus {
    tokio::task::spawn_blocking(
        move || match open(vendor_id, product_id, serial.as_deref()) {
            Err(e) => PrinterStatus::new(PState::Offline, e),
            Ok(opened) => query(opened).1.unwrap_or_else(|| {
                PrinterStatus::new(PState::Unknown, "Conectada; no informa de su estado")
            }),
        },
    )
    .await
    .unwrap_or_else(|e| PrinterStatus::new(PState::Error, e.to_string()))
}
