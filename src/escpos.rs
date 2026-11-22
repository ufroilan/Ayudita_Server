//! Traducción de la comanda maquetada a bytes ESC/POS.

use crate::ticket::{Doc, Out, layout};

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const FS: u8 = 0x1C;
const LF: u8 = 0x0A;

/// Petición de estado en tiempo real: impresora, causa de offline y sensor de papel.
pub const STATUS_QUERY: [u8; 9] = [0x10, 0x04, 0x01, 0x10, 0x04, 0x02, 0x10, 0x04, 0x04];

/// Bytes ESC/POS de la comanda. Siempre igual para cualquier impresora:
/// - `FS .` saca a las impresoras chinas (Xprinter y similares) del modo de caracteres chinos,
///   que convierte las tildes en ideogramas; las que no tienen ese modo lo ignoran.
/// - Tabla PC850, la de más caracteres del español (á é í ó ú ü ñ Ñ ¿ ¡ º ª × ·).
/// - `GS V` corta el papel; las impresoras sin cuchilla lo ignoran.
pub fn render(doc: &Doc, copies: u8) -> Vec<u8> {
    let lines = layout(doc);
    let mut out = Vec::with_capacity(2048);
    for _ in 0..copies.max(1) {
        out.extend([ESC, b'@', FS, b'.', ESC, b't', 2]);
        for line in &lines {
            match line {
                Out::Line {
                    text,
                    w,
                    h,
                    bold,
                    invert,
                } => {
                    let lead = crate::ticket::lead(doc.cols, text, *w);
                    if lead > 0 {
                        out.extend([GS, b'!', 0, GS, b'B', 0]);
                        out.extend(std::iter::repeat_n(b' ', lead));
                    }
                    let size = ((w.clamp(&1, &8) - 1) << 4) | (h.clamp(&1, &8) - 1);
                    out.extend([GS, b'!', size]);
                    out.extend([ESC, b'E', u8::from(*bold)]);
                    out.extend([GS, b'B', u8::from(*invert)]);
                    out.extend(encode(text));
                    out.extend([GS, b'B', 0]);
                    out.push(LF);
                }
                Out::Feed(n) => out.extend([ESC, b'd', *n]),
                // Avanza hasta la cuchilla y corta parcialmente.
                Out::Cut => out.extend([GS, b'V', 66, 0]),
            }
        }
        out.extend([GS, b'!', 0, ESC, b'E', 0]);
    }
    out
}

/// Estado de la impresora a partir de las respuestas a `STATUS_QUERY` (DLE EOT 1, 2 y 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RtStatus {
    pub offline: bool,
    pub cover_open: bool,
    pub paper_out: bool,
    pub paper_near_end: bool,
    pub error: bool,
}

pub fn parse_status(bytes: &[u8]) -> Option<RtStatus> {
    // Cada byte de respuesta tiene el bit 1 y el bit 4 a 1 y los bits 0 y 7 a 0.
    let valid: Vec<u8> = bytes.iter().copied().filter(|b| b & 0x93 == 0x12).collect();
    if valid.len() < 3 {
        return None;
    }
    let (printer, offline, paper) = (valid[0], valid[1], valid[2]);
    Some(RtStatus {
        offline: printer & 0x08 != 0,
        cover_open: offline & 0x04 != 0,
        paper_out: offline & 0x20 != 0 || paper & 0x60 != 0,
        paper_near_end: paper & 0x0C != 0,
        error: offline & 0x40 != 0,
    })
}

/// Página de códigos PC850, posiciones 0x80..=0xFF.
const CP850_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', 'ø', '£', 'Ø', '×', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '®', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', 'Á', 'Â', 'À', '©', '╣', '║', '╗', '╝', '¢', '¥', '┐', //
    '└', '┴', '┬', '├', '─', '┼', 'ã', 'Ã', '╚', '╔', '╩', '╦', '╠', '═', '╬', '¤', //
    'ð', 'Ð', 'Ê', 'Ë', 'È', 'ı', 'Í', 'Î', 'Ï', '┘', '┌', '█', '▄', '¦', 'Ì', '▀', //
    'Ó', 'ß', 'Ô', 'Ò', 'õ', 'Õ', 'µ', 'þ', 'Þ', 'Ú', 'Û', 'Ù', 'ý', 'Ý', '¯', '´', //
    '\u{AD}', '±', '‗', '¾', '¶', '§', '÷', '¸', '°', '¨', '·', '¹', '³', '²', '■', '\u{A0}',
];

pub fn encode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(if c.is_ascii_control() { b' ' } else { c as u8 });
            continue;
        }
        // Acentos combinados (forma NFD): se omiten, la letra base ya está escrita.
        if ('\u{300}'..='\u{36F}').contains(&c) {
            continue;
        }
        if let Some(i) = CP850_HIGH.iter().position(|&x| x == c) {
            out.push(0x80 + i as u8);
            continue;
        }
        out.extend(transliterate(c).bytes());
    }
    out
}

fn transliterate(c: char) -> &'static str {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => "a",
        'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => "A",
        'é' | 'è' | 'ê' | 'ë' => "e",
        'É' | 'È' | 'Ê' | 'Ë' => "E",
        'í' | 'ì' | 'î' | 'ï' => "i",
        'Í' | 'Ì' | 'Î' | 'Ï' => "I",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => "o",
        'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => "O",
        'ú' | 'ù' | 'û' | 'ü' => "u",
        'Ú' | 'Ù' | 'Û' | 'Ü' => "U",
        'ñ' => "n",
        'Ñ' => "N",
        'ç' => "c",
        'Ç' => "C",
        '×' => "x",
        '·' | '•' => "-",
        '»' => ">",
        '«' => "<",
        '¿' | '¡' => "",
        '“' | '”' | '„' => "\"",
        '‘' | '’' | '´' => "'",
        '–' | '—' => "-",
        '…' => "...",
        '€' => "EUR",
        'º' | '°' => "o",
        'ª' => "a",
        '\u{A0}' => " ",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp850_spanish_characters() {
        assert_eq!(
            encode("á é í ó ú ñ Ñ ¿¡ × · »"),
            vec![
                0xA0, b' ', 0x82, b' ', 0xA1, b' ', 0xA2, b' ', 0xA3, b' ', 0xA4, b' ', 0xA5, b' ',
                0xA8, 0xAD, b' ', 0x9E, b' ', 0xFA, b' ', 0xAF
            ]
        );
    }

    #[test]
    fn fallback_for_symbols_outside_pc850() {
        assert_eq!(
            encode("Piña “2” – ¡ya! 🔥"),
            b"Pi\xa4a \"2\" - \xadya! ?".to_vec()
        );
        assert_eq!(encode("€"), b"EUR".to_vec());
        // NFD: «e» + acento combinado.
        assert_eq!(encode("Cafe\u{301}"), b"Cafe".to_vec());
    }

    #[test]
    fn status_bytes() {
        assert_eq!(parse_status(&[0x12, 0x12, 0x12]), Some(RtStatus::default()));
        let s = parse_status(&[0x1A, 0x32, 0x72]).unwrap();
        assert!(s.offline && s.paper_out && !s.cover_open);
        let s = parse_status(&[0x1A, 0x16, 0x12]).unwrap();
        assert!(s.cover_open);
        assert_eq!(parse_status(&[0x12]), None);
    }

    #[test]
    fn render_starts_with_init_and_ends_with_cut() {
        let c = crate::model::tests::sample();
        let doc = crate::ticket::comanda(
            &c,
            crate::ticket::Mark::Original,
            &crate::ticket::Options {
                paper: crate::ticket::Paper::Mm80,
                show_prices: false,
            },
        );
        let bytes = render(&doc, 2);
        assert_eq!(&bytes[..7], &[ESC, b'@', FS, b'.', ESC, b't', 2]);
        assert_eq!(
            bytes.windows(4).filter(|w| *w == [GS, b'V', 66, 0]).count(),
            2
        );
    }
}
