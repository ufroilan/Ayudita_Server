//! Impresoras del sistema en Windows: winspool con tipo de datos RAW.

use std::ptr::{null, null_mut};

use windows_sys::Win32::Graphics::Printing::{
    ClosePrinter, DOC_INFO_1W, EndDocPrinter, EndPagePrinter, EnumPrintersW, GetDefaultPrinterW,
    GetPrinterW, OpenPrinterW, PRINTER_ATTRIBUTE_WORK_OFFLINE, PRINTER_ENUM_CONNECTIONS,
    PRINTER_ENUM_LOCAL, PRINTER_HANDLE, PRINTER_INFO_2W, PRINTER_INFO_4W, PRINTER_STATUS_DOOR_OPEN,
    PRINTER_STATUS_ERROR, PRINTER_STATUS_NOT_AVAILABLE, PRINTER_STATUS_OFFLINE,
    PRINTER_STATUS_PAPER_JAM, PRINTER_STATUS_PAPER_OUT, PRINTER_STATUS_PAPER_PROBLEM,
    PRINTER_STATUS_PAUSED, PRINTER_STATUS_PRINTING, PRINTER_STATUS_USER_INTERVENTION,
    StartDocPrinterW, StartPagePrinter, WritePrinter,
};

use super::{PState, PrinterStatus, SystemPrinters};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

struct Handle(PRINTER_HANDLE);

impl Handle {
    fn open(name: &str) -> Result<Handle, String> {
        let name = wide(name);
        let mut h: PRINTER_HANDLE = unsafe { std::mem::zeroed() };
        let ok = unsafe { OpenPrinterW(name.as_ptr(), &mut h, null()) };
        if ok == 0 {
            return Err("La impresora ya no está instalada en Windows".into());
        }
        Ok(Handle(h))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { ClosePrinter(self.0) };
    }
}

pub fn list() -> SystemPrinters {
    let mut names = Vec::new();
    unsafe {
        let flags = PRINTER_ENUM_LOCAL | PRINTER_ENUM_CONNECTIONS;
        let mut needed = 0u32;
        let mut count = 0u32;
        EnumPrintersW(flags, null(), 4, null_mut(), 0, &mut needed, &mut count);
        if needed > 0 {
            let mut buf = vec![0u8; needed as usize];
            if EnumPrintersW(
                flags,
                null(),
                4,
                buf.as_mut_ptr(),
                needed,
                &mut needed,
                &mut count,
            ) != 0
            {
                let infos = std::slice::from_raw_parts(
                    buf.as_ptr() as *const PRINTER_INFO_4W,
                    count as usize,
                );
                for info in infos {
                    names.push(from_wide(info.pPrinterName));
                }
            }
        }
    }
    names.sort();
    let default = unsafe {
        let mut len = 0u32;
        GetDefaultPrinterW(null_mut(), &mut len);
        if len == 0 {
            None
        } else {
            let mut buf = vec![0u16; len as usize];
            if GetDefaultPrinterW(buf.as_mut_ptr(), &mut len) != 0 {
                Some(from_wide(buf.as_ptr()))
            } else {
                None
            }
        }
    };
    SystemPrinters { names, default }
}

fn print_blocking(name: &str, bytes: &[u8], title: &str) -> Result<Option<String>, String> {
    let h = Handle::open(name)?;
    let mut doc_name = wide(title);
    let mut datatype = wide("RAW");
    let info = DOC_INFO_1W {
        pDocName: doc_name.as_mut_ptr(),
        pOutputFile: null_mut(),
        pDatatype: datatype.as_mut_ptr(),
    };
    unsafe {
        if StartDocPrinterW(h.0, 1, &info) == 0 {
            return Err("Windows no aceptó el trabajo de impresión".into());
        }
        if StartPagePrinter(h.0) == 0 {
            EndDocPrinter(h.0);
            return Err("Windows no aceptó el trabajo de impresión".into());
        }
        let mut written = 0u32;
        let ok = WritePrinter(h.0, bytes.as_ptr().cast(), bytes.len() as u32, &mut written);
        EndPagePrinter(h.0);
        EndDocPrinter(h.0);
        if ok == 0 || written as usize != bytes.len() {
            return Err("No se pudieron enviar los datos a la impresora".into());
        }
    }
    Ok(Some("Enviada a la cola de Windows".into()))
}

fn status_blocking(name: &str) -> PrinterStatus {
    let h = match Handle::open(name) {
        Ok(h) => h,
        Err(e) => return PrinterStatus::new(PState::NotConfigured, e),
    };
    unsafe {
        let mut needed = 0u32;
        GetPrinterW(h.0, 2, null_mut(), 0, &mut needed);
        if needed == 0 {
            return PrinterStatus::new(PState::Unknown, "Instalada en Windows");
        }
        let mut buf = vec![0u8; needed as usize];
        if GetPrinterW(h.0, 2, buf.as_mut_ptr(), needed, &mut needed) == 0 {
            return PrinterStatus::new(PState::Unknown, "Instalada en Windows");
        }
        let info = &*(buf.as_ptr() as *const PRINTER_INFO_2W);
        let s = info.Status;
        if s & (PRINTER_STATUS_PAPER_OUT | PRINTER_STATUS_PAPER_PROBLEM) != 0 {
            return PrinterStatus::new(PState::PaperOut, "Cambia el rollo de papel");
        }
        if s & PRINTER_STATUS_DOOR_OPEN != 0 {
            return PrinterStatus::new(PState::CoverOpen, "Cierra la tapa de la impresora");
        }
        if s & PRINTER_STATUS_PAUSED != 0 {
            return PrinterStatus::new(PState::Offline, "Está en pausa en Windows");
        }
        if s & (PRINTER_STATUS_OFFLINE | PRINTER_STATUS_NOT_AVAILABLE) != 0
            || info.Attributes & PRINTER_ATTRIBUTE_WORK_OFFLINE != 0
        {
            return PrinterStatus::new(PState::Offline, "Windows la marca como desconectada");
        }
        if s & (PRINTER_STATUS_ERROR | PRINTER_STATUS_PAPER_JAM | PRINTER_STATUS_USER_INTERVENTION)
            != 0
        {
            return PrinterStatus::new(PState::Error, "La impresora indica un error");
        }
        if s & PRINTER_STATUS_PRINTING != 0 {
            return PrinterStatus::new(PState::Busy, "");
        }
        if info.cJobs > 0 {
            return PrinterStatus::new(
                PState::Error,
                format!(
                    "{} trabajo(s) esperando en la cola de Windows: ¿está encendida?",
                    info.cJobs
                ),
            );
        }
    }
    PrinterStatus::new(PState::Unknown, "Instalada en Windows")
}

pub async fn print(name: &str, bytes: &[u8], title: &str) -> Result<Option<String>, String> {
    let st = status(name).await;
    if matches!(
        st.state,
        PState::Offline | PState::NotConfigured | PState::PaperOut | PState::CoverOpen
    ) {
        return Err(st.detail);
    }
    let (name, bytes, title) = (name.to_string(), bytes.to_vec(), title.to_string());
    tokio::task::spawn_blocking(move || print_blocking(&name, &bytes, &title))
        .await
        .map_err(|e| e.to_string())?
}

pub async fn status(name: &str) -> PrinterStatus {
    let name = name.to_string();
    tokio::task::spawn_blocking(move || status_blocking(&name))
        .await
        .unwrap_or_else(|e| PrinterStatus::new(PState::Error, e.to_string()))
}
