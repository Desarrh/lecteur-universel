//! Sous-titres externes .srt / .vtt (chargés à côté de la vidéo ou glissés dessus).

use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

fn ts(s: &str) -> Option<f64> {
    // 00:01:02,345 · 01:02.345 · 00:01:02.345
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Retire les balises simples (<i>, <b>, <font …>, {\an8}).
fn clean(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in t.chars() {
        match c {
            '<' | '{' => depth += 1,
            '>' | '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

pub fn parse(text: &str) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(i) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let mut it = lines[i].split("-->");
        let (Some(a), Some(b)) = (it.next(), it.next()) else { continue };
        // la fin peut être suivie de réglages VTT (« align:start »)
        let b = b.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (ts(a), ts(b)) else { continue };
        let body = clean(&lines[i + 1..].join("\n"));
        if !body.is_empty() {
            cues.push(Cue { start, end, text: body });
        }
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

pub fn load(path: &Path) -> Option<Vec<Cue>> {
    let bytes = std::fs::read(path).ok()?;
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        // ancien fichier en Windows-1252 : on garde l'essentiel lisible
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    };
    let c = parse(&text);
    if c.is_empty() { None } else { Some(c) }
}

/// Sous-titres à côté de la vidéo : « film.srt », « film.fr.srt », « film.vtt »…
pub fn find_sidecar(video: &Path) -> Option<PathBuf> {
    let dir = video.parent()?;
    let stem = video.file_stem()?.to_string_lossy().to_lowercase();
    let mut best: Option<PathBuf> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext != "srt" && ext != "vtt" {
            continue;
        }
        let s = p.file_stem().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if s == stem {
            return Some(p);
        }
        if s.starts_with(&format!("{stem}.")) {
            // préfère le français
            if s.ends_with(".fr") || best.is_none() {
                best = Some(p);
            }
        }
    }
    best
}

/// Répliques qui contiennent `q` (sans tenir compte de la casse ni des accents) : leurs débuts (s).
pub fn find(cues: &[Cue], q: &str) -> Vec<f64> {
    let q = socle::fuzzy::fold(q.trim());
    if q.is_empty() {
        return Vec::new();
    }
    cues.iter().filter(|c| socle::fuzzy::fold(&c.text).contains(&q)).map(|c| c.start).collect()
}

/// Texte de toutes les répliques (mots proposés par l'autocomplétion).
pub fn all_text(cues: &[Cue]) -> String {
    cues.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n")
}

pub fn at(cues: &[Cue], t: f64) -> Option<&str> {
    let i = cues.partition_point(|c| c.start <= t);
    cues[..i].iter().rev().take(4).find(|c| t < c.end).map(|c| c.text.as_str())
}
