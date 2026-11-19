//! Écriture de PDF.
//! 1. Si Microsoft Word est installé : export fidèle via COM (PowerShell, Word invisible).
//! 2. Sinon : petit générateur maison (polices PDF standard Helvetica/Courier,
//!    encodage WinAnsi pour les accents) — mise en page simple mais propre.

use crate::docx::Block;
use std::path::Path;

/// DOCX → PDF via Word. Renvoie Err si Word n'est pas là ou a échoué.
pub fn word_export(src: &Path, out: &Path) -> Result<(), String> {
    let esc = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; $w=New-Object -ComObject Word.Application; $w.Visible=$false; $w.DisplayAlerts=0; \
         try {{ $d=$w.Documents.Open('{}', $false, $true); $d.ExportAsFixedFormat('{}', 17); $d.Close(0) }} finally {{ $w.Quit() }}",
        esc(src),
        esc(out)
    );
    let st = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags_no_window()
        .output()
        .map_err(|e| e.to_string())?;
    if st.status.success() && out.exists() { Ok(()) } else { Err("Word indisponible".into()) }
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}
impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Font {
    Regular,
    Bold,
    Mono,
}

// Largeurs Helvetica (AFM standard, 1/1000 em) pour les caractères 32..126.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584,
    584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667,
    611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500,
    722, 500, 500, 500, 334, 260, 334, 584,
];

fn base_char(c: char) -> char {
    match c {
        'à' | 'â' | 'ä' | 'á' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' => 'i',
        'ô' | 'ö' | 'ó' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ç' => 'c',
        'À' | 'Â' => 'A',
        'É' | 'È' | 'Ê' => 'E',
        'Ç' => 'C',
        c => c,
    }
}

fn width(c: char, f: Font, size: f32) -> f32 {
    if f == Font::Mono {
        return 0.6 * size;
    }
    let b = base_char(c) as u32;
    let w = if (32..127).contains(&b) {
        HELV[(b - 32) as usize] as f32
    } else {
        match c {
            'œ' | 'Œ' => 944.0,
            '—' => 1000.0,
            _ => 556.0,
        }
    };
    w * size / 1000.0 * if f == Font::Bold { 1.06 } else { 1.0 }
}

/// Caractère → octet WinAnsi (cp1252). Inconnu → '?'.
fn winansi(c: char) -> u8 {
    let u = c as u32;
    if (32..127).contains(&u) || (0xA0..=0xFF).contains(&u) {
        return u as u8;
    }
    match c {
        '€' => 0x80,
        '‚' => 0x82,
        '„' => 0x84,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        'œ' => 0x9C,
        'Œ' => 0x8C,
        '\u{202F}' | '\u{2009}' => 0xA0,
        '\t' => b' ',
        _ => b'?',
    }
}

struct Writer {
    pages: Vec<Vec<u8>>,
    cur: Vec<u8>,
    y: f32,
}

const PW: f32 = 595.0;
const PH: f32 = 842.0;
const M: f32 = 62.0;

impl Writer {
    fn new() -> Self {
        Writer { pages: Vec::new(), cur: Vec::new(), y: PH - M }
    }
    fn need(&mut self, h: f32) {
        if self.y - h < M {
            self.pages.push(std::mem::take(&mut self.cur));
            self.y = PH - M;
        }
    }
    fn text(&mut self, x: f32, s: &str, f: Font, size: f32) {
        let fname = match f {
            Font::Regular => "F1",
            Font::Bold => "F2",
            Font::Mono => "F3",
        };
        let mut bytes = Vec::new();
        for c in s.chars() {
            match winansi(c) {
                b'(' => bytes.extend_from_slice(b"\\("),
                b')' => bytes.extend_from_slice(b"\\)"),
                b'\\' => bytes.extend_from_slice(b"\\\\"),
                b => bytes.push(b),
            }
        }
        self.cur.extend_from_slice(format!("BT /{fname} {size} Tf {x:.1} {:.1} Td (", self.y).as_bytes());
        self.cur.extend_from_slice(&bytes);
        self.cur.extend_from_slice(b") Tj ET\n");
    }
    fn line(&mut self, x0: f32, x1: f32, y: f32) {
        self.cur.extend_from_slice(format!("0.75 G 0.5 w {x0:.1} {y:.1} m {x1:.1} {y:.1} l S 0 G\n").as_bytes());
    }
    /// Paragraphe à plusieurs styles, coupé aux espaces.
    fn para(&mut self, runs: &[(String, Font)], size: f32, indent: f32, lead: f32) {
        let maxw = PW - 2.0 * M - indent;
        // découpe en mots (avec leur police)
        let mut words: Vec<(String, Font)> = Vec::new();
        for (t, f) in runs {
            for (i, part) in t.split(' ').enumerate() {
                if i > 0 {
                    words.push((" ".into(), *f));
                }
                if !part.is_empty() {
                    words.push((part.to_string(), *f));
                }
            }
        }
        let mut lines: Vec<Vec<(String, Font)>> = vec![vec![]];
        let mut w = 0.0;
        for (word, f) in words {
            let ww: f32 = word.chars().map(|c| width(c, f, size)).sum();
            if word == " " && w == 0.0 {
                continue;
            }
            if w + ww > maxw && w > 0.0 {
                lines.push(vec![]);
                w = 0.0;
                if word == " " {
                    continue;
                }
            }
            w += ww;
            lines.last_mut().unwrap().push((word, f));
        }
        for l in lines {
            self.need(size * lead);
            self.y -= size * lead;
            let mut x = M + indent;
            // regroupe les morceaux de même police
            let mut seg = String::new();
            let mut sf = Font::Regular;
            for (t, f) in l {
                if f != sf && !seg.is_empty() {
                    self.text(x, &seg, sf, size);
                    x += seg.chars().map(|c| width(c, sf, size)).sum::<f32>();
                    seg.clear();
                }
                sf = f;
                seg.push_str(&t);
            }
            if !seg.is_empty() {
                self.text(x, &seg, sf, size);
            }
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if !self.cur.is_empty() || self.pages.is_empty() {
            self.pages.push(std::mem::take(&mut self.cur));
        }
        let n = self.pages.len();
        let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offs = Vec::new();
        let mut obj = |out: &mut Vec<u8>, offs: &mut Vec<usize>, body: Vec<u8>| {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", offs.len()).as_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(b"\nendobj\n");
        };
        // 1 catalogue, 2 pages, 3-5 polices, puis (page, contenu) × n
        let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 6 + i * 2)).collect();
        obj(&mut out, &mut offs, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        obj(&mut out, &mut offs, format!("<< /Type /Pages /Count {n} /Kids [{}] >>", kids.join(" ")).into_bytes());
        for f in ["Helvetica", "Helvetica-Bold", "Courier"] {
            obj(&mut out, &mut offs, format!("<< /Type /Font /Subtype /Type1 /BaseFont /{f} /Encoding /WinAnsiEncoding >>").into_bytes());
        }
        for (i, content) in self.pages.iter().enumerate() {
            obj(
                &mut out,
                &mut offs,
                format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PW} {PH}] /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 5 0 R >> >> /Contents {} 0 R >>", 7 + i * 2).into_bytes(),
            );
            let mut body = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            body.extend_from_slice(content);
            body.extend_from_slice(b"\nendstream");
            obj(&mut out, &mut offs, body);
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offs.len() + 1).as_bytes());
        for o in &offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offs.len() + 1).as_bytes());
        out
    }
}

pub fn docx_to_pdf(blocks: &[Block], out: &Path) -> Result<(), String> {
    let mut w = Writer::new();
    for b in blocks {
        match b {
            Block::Para { level, list, runs } => {
                let size = match level {
                    1 => 22.0,
                    2 => 17.0,
                    3 => 14.0,
                    4 => 12.5,
                    _ => 11.0,
                };
                if runs.iter().all(|r| r.text.trim().is_empty()) {
                    w.y -= 6.0;
                    continue;
                }
                if *level > 0 {
                    w.y -= 8.0;
                }
                let mut rs: Vec<(String, Font)> = Vec::new();
                if *list {
                    rs.push(("•  ".into(), Font::Regular));
                }
                for r in runs {
                    let f = if r.bold || *level > 0 { Font::Bold } else { Font::Regular };
                    rs.push((r.text.replace('\n', " "), f));
                }
                w.para(&rs, size, if *list { 14.0 } else { 0.0 }, 1.42);
                w.y -= size * 0.45;
            }
            Block::Table(rows) => {
                let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
                let cw = (PW - 2.0 * M) / cols as f32;
                for (ri, row) in rows.iter().enumerate() {
                    w.need(18.0);
                    w.y -= 15.0;
                    for (ci, cell) in row.iter().enumerate() {
                        let f = if ri == 0 { Font::Bold } else { Font::Regular };
                        let mut s = String::new();
                        let mut acc = 0.0;
                        for c in cell.chars() {
                            acc += width(c, f, 10.0);
                            if acc > cw - 10.0 {
                                s.push('…');
                                break;
                            }
                            s.push(c);
                        }
                        w.text(M + ci as f32 * cw + 4.0, &s, f, 10.0);
                    }
                    let y = w.y - 5.0;
                    w.line(M, PW - M, y);
                }
                w.y -= 12.0;
            }
        }
    }
    std::fs::write(out, w.finish()).map_err(|e| e.to_string())
}

pub fn text_to_pdf(text: &str, out: &Path) -> Result<(), String> {
    let mut w = Writer::new();
    let size = 9.5;
    let maxc = ((PW - 2.0 * M) / (0.6 * size)) as usize;
    for line in text.lines() {
        let line = line.replace('\t', "    ");
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            w.need(size * 1.35);
            w.y -= size * 1.35;
            continue;
        }
        for chunk in chars.chunks(maxc) {
            w.need(size * 1.35);
            w.y -= size * 1.35;
            let s: String = chunk.iter().collect();
            w.text(M, &s, Font::Mono, size);
        }
    }
    std::fs::write(out, w.finish()).map_err(|e| e.to_string())
}
