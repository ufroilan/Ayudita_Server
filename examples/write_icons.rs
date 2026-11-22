//! Escribe los iconos de los paquetes a partir del que dibuja la app (`src/icon.rs`):
//!
//! - `assets/icon/AppIcon.iconset/`: los PNG que `iconutil` convierte en `AppIcon.icns` (macOS).
//! - `assets/icon/icon.ico`: el del `.exe` de Windows (lo incrusta `build.rs`).
//! - `assets/icon/icon-256.png`: el de Linux.
//!
//! Se ejecuta a mano cuando cambia el dibujo, y se suben los archivos:
//!
//! ```sh
//! cargo run --example write_icons
//! iconutil -c icns assets/icon/AppIcon.iconset -o assets/icon/AppIcon.icns
//! ```

#[path = "../src/icon.rs"]
mod icon;

use std::fs;
use std::path::Path;

fn png(size: u32) -> Vec<u8> {
    let (w, h, px) = icon::rgba(size);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("cabecera PNG");
        writer.write_image_data(&px).expect("datos PNG");
    }
    out
}

/// Un `.ico` con PNG dentro, que Windows admite desde Vista.
fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| png(s)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (size, data) in sizes.iter().zip(&images) {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for data in &images {
        out.extend_from_slice(data);
    }
    out
}

fn main() {
    let dir = Path::new("assets/icon");
    let set = dir.join("AppIcon.iconset");
    fs::create_dir_all(&set).expect("carpeta de iconos");

    for base in [16u32, 32, 128, 256, 512] {
        fs::write(set.join(format!("icon_{base}x{base}.png")), png(base)).unwrap();
        fs::write(set.join(format!("icon_{base}x{base}@2x.png")), png(base * 2)).unwrap();
    }

    fs::write(dir.join("icon.ico"), ico(&[16, 24, 32, 48, 64, 256])).unwrap();
    fs::write(dir.join("icon-256.png"), png(256)).unwrap();
    println!("Iconos escritos en {}", dir.display());
}
