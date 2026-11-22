//! Impresora de red: TCP al puerto RAW (9100) y estado en tiempo real con `DLE EOT`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};

use super::{PState, PrinterStatus};
use crate::escpos::{RtStatus, STATUS_QUERY, parse_status};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const STATUS_TIMEOUT: Duration = Duration::from_millis(800);

async fn connect(host: &str, port: u16) -> Result<TcpStream, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("Escribe la dirección IP de la impresora".into());
    }
    match timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port))).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(format!("No se puede conectar con {host}:{port}: {e}")),
        Err(_) => Err(format!("La impresora {host}:{port} no responde")),
    }
}

/// Pide el estado. `None` si la impresora no contesta a `DLE EOT` (algunas no lo hacen).
async fn query(stream: &mut TcpStream) -> Option<RtStatus> {
    stream.write_all(&STATUS_QUERY).await.ok()?;
    let deadline = Instant::now() + STATUS_TIMEOUT;
    let mut got = Vec::new();
    let mut buf = [0u8; 16];
    while got.len() < 3 {
        let left = deadline.checked_duration_since(Instant::now())?;
        match timeout(left, stream.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => got.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    parse_status(&got)
}

pub async fn print(host: &str, port: u16, bytes: &[u8]) -> Result<Option<String>, String> {
    let mut stream = connect(host, port).await?;
    if let Some(st) = query(&mut stream).await {
        let st = PrinterStatus::from(st);
        if st.state.is_problem() {
            return Err(format!("{}: {}", st.state.label(), st.detail));
        }
    }
    let send = async {
        stream.write_all(bytes).await?;
        stream.flush().await?;
        stream.shutdown().await
    };
    match timeout(WRITE_TIMEOUT, send).await {
        Ok(Ok(())) => Ok(None),
        Ok(Err(e)) => Err(format!("Se cortó la conexión con la impresora: {e}")),
        Err(_) => Err("La impresora no acepta datos (¿atascada o sin papel?)".into()),
    }
}

pub async fn status(host: &str, port: u16) -> PrinterStatus {
    match connect(host, port).await {
        Err(e) => PrinterStatus::new(PState::Offline, e),
        Ok(mut stream) => {
            let st = match query(&mut stream).await {
                Some(st) => PrinterStatus::from(st),
                None => PrinterStatus::new(PState::Unknown, "Conectada; no informa de su estado"),
            };
            let _ = stream.shutdown().await;
            st
        }
    }
}
