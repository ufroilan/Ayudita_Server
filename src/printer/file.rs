//! Impresora «de mentira»: guarda los bytes ESC/POS y una vista en texto.

use std::path::Path;

use super::PrintJob;

pub async fn print(dir: &Path, job: &PrintJob<'_>) -> Result<String, String> {
    let dir = dir.to_path_buf();
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let name: String = job
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    let base = dir.join(format!("{stamp}-{name}"));
    let bytes = job.bytes.to_vec();
    let preview = job.preview.to_string();
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(base.with_extension("bin"), bytes)?;
        std::fs::write(base.with_extension("txt"), preview)?;
        Ok::<_, std::io::Error>(base.with_extension("txt").display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("No se pudo guardar: {e}"))
}
