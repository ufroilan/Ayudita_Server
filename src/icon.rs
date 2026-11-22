//! Icono de la app dibujado por código: un ticket blanco sobre el azul marino de Ayudita, con
//! los renglones en el teal de la marca.

pub fn rgba(size: u32) -> (u32, u32, Vec<u8>) {
    let s = size as f32;
    let mut px = vec![0u8; (size * size * 4) as usize];
    let navy = [0x01, 0x0D, 0x17, 0xFF];
    let white = [0xFF, 0xFF, 0xFF, 0xFF];
    let ink = [0x00, 0xA3, 0x8B, 0xFF];
    let radius = s * 0.22;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Cuadrado redondeado.
            let cx = fx.clamp(radius, s - radius);
            let cy = fy.clamp(radius, s - radius);
            if (fx - cx).powi(2) + (fy - cy).powi(2) > radius * radius {
                continue;
            }
            let mut c = navy;
            // Ticket con borde inferior en zigzag.
            let (l, r, t, b) = (s * 0.28, s * 0.72, s * 0.16, s * 0.84);
            let tooth = s * 0.075;
            let zig = (((fx - l) / tooth).fract() - 0.5).abs() * tooth;
            if fx >= l && fx <= r && fy >= t && fy <= b - zig {
                c = white;
                // Renglones.
                let rows = [0.30, 0.42, 0.54, 0.66];
                for (i, row) in rows.iter().enumerate() {
                    let ry = s * row;
                    let end = if i == 0 { r - s * 0.16 } else { r - s * 0.07 };
                    let thick = if i == 0 { s * 0.05 } else { s * 0.03 };
                    if (fy - ry).abs() <= thick / 2.0 && fx >= l + s * 0.07 && fx <= end {
                        c = ink;
                    }
                }
            }
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&c);
        }
    }
    (size, size, px)
}
