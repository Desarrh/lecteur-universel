use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Pdf,
    Docx,
    Text,
    Audio,
    Video,
    Unknown,
}

pub const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "jfif", "webp", "gif", "bmp", "ico", "tif", "tiff", "heic", "heif", "avif", "jxr", "wdp", "hdp", "dds", "dng", "cr2", "cr3", "nef",
    "nrw", "arw", "raf", "orf", "rw2", "pef", "srw",
];
pub const AUDIO: &[&str] = &["mp3", "wav", "m4a", "aac", "flac", "wma", "ogg"];
pub const VIDEO: &[&str] = &["mp4", "mov", "m4v", "mkv", "avi", "wmv", "webm"];
pub const TEXT: &[&str] = &[
    "txt", "md", "log", "csv", "json", "toml", "yaml", "yml", "ini", "cfg", "xml", "html", "htm", "css", "js", "ts", "tsx", "jsx", "rs", "py",
    "c", "h", "cpp", "hpp", "cs", "java", "go", "sh", "ps1", "bat", "sql", "lua", "gd", "kt", "swift", "php", "rb", "srt", "vtt",
];
pub const ALL_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "tif", "tiff", "heic", "heif", "avif", "mp3", "wav", "m4a", "aac", "flac", "wma", "mp4", "mov", "m4v", "mkv",
    "avi", "wmv", "pdf", "docx", "txt", "md", "log", "csv", "json", "toml", "yaml", "yml", "ini", "xml", "rs", "py", "js", "ts", "css",
];
pub const DIALOG_PATTERN: &str = "*.heic;*.heif;*.avif;*.dng;*.cr2;*.nef;*.arw;*.srt;*.vtt;*.png;*.jpg;*.jpeg;*.webp;*.gif;*.bmp;*.ico;*.tif;*.tiff;*.mp3;*.wav;*.m4a;*.aac;*.flac;*.wma;*.mp4;*.mov;*.m4v;*.mkv;*.avi;*.wmv;*.pdf;*.docx;*.txt;*.md;*.log;*.csv;*.json;*.toml;*.yaml;*.yml;*.ini;*.xml;*.html;*.css;*.js;*.ts;*.rs;*.py;*.c;*.cpp;*.h;*.cs;*.java;*.go;*.sh;*.ps1;*.bat;*.sql;*.lua;*.gd";

/// Formats d'image lisibles : la liste du lecteur plus celle du socle (PSD, EXR, SVG, JXL, TGA, QOI, HDR…).
fn is_image(e: &str) -> bool {
    static ALL: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    IMAGE.contains(&e) || ALL.get_or_init(socle::images::extensions).contains(&e)
}

/// Toutes les extensions prises en charge (« Ouvrir avec »).
pub fn all_exts() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ALL_EXTS.to_vec();
    for e in socle::images::extensions() {
        if !v.contains(&e) {
            v.push(e);
        }
    }
    v
}

/// Filtre de la boîte « Ouvrir ».
pub fn dialog_pattern() -> String {
    let mut s = DIALOG_PATTERN.to_string();
    for e in socle::images::extensions() {
        let pat = format!("*.{e}");
        if !s.split(';').any(|x| x == pat) {
            s.push(';');
            s.push_str(&pat);
        }
    }
    s
}

pub fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn of(p: &Path) -> Kind {
    let e = ext(p);
    let e = e.as_str();
    if is_image(e) {
        Kind::Image
    } else if e == "pdf" {
        Kind::Pdf
    } else if e == "docx" {
        Kind::Docx
    } else if AUDIO.contains(&e) {
        Kind::Audio
    } else if VIDEO.contains(&e) {
        Kind::Video
    } else if TEXT.contains(&e) || e.is_empty() {
        Kind::Text
    } else {
        Kind::Unknown
    }
}

pub fn supported(p: &Path) -> bool {
    of(p) != Kind::Unknown
}

/// Couleur de la pastille d'onglet, comme sur les maquettes.
pub fn color(k: Kind) -> egui::Color32 {
    let h = match k {
        Kind::Image => 0x2F7D5B,
        Kind::Pdf => 0xB5462F,
        Kind::Docx => 0x2E5E9E,
        Kind::Text => 0xD9822B,
        Kind::Audio => 0x7A4FA0,
        Kind::Video => 0xB5462F,
        Kind::Unknown => 0x888888,
    };
    egui::Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

pub fn label(k: Kind) -> &'static str {
    match k {
        Kind::Image => "Image",
        Kind::Pdf => "PDF",
        Kind::Docx => "Word",
        Kind::Text => "Texte",
        Kind::Audio => "Audio",
        Kind::Video => "Vidéo",
        Kind::Unknown => "Fichier",
    }
}

pub fn human_size(b: u64) -> String {
    let f = b as f64;
    if f < 1024.0 {
        format!("{b} o")
    } else if f < 1024.0 * 1024.0 {
        format!("{:.0} Ko", f / 1024.0)
    } else if f < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} Mo", f / 1048576.0).replace('.', ",")
    } else {
        format!("{:.2} Go", f / 1073741824.0).replace('.', ",")
    }
}

pub fn hms(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0) as u64;
    let s = t - m as f64 * 60.0;
    if m >= 60 {
        format!("{}:{:02}:{:04.1}", m / 60, m % 60, s)
    } else {
        format!("{:02}:{:04.1}", m, s).replace('.', ",")
    }
}

/// Fichiers voisins pris en charge dans le même dossier (tri par nom).
pub fn neighbours(p: &Path) -> Vec<std::path::PathBuf> {
    let Some(dir) = p.parent() else { return vec![] };
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|q| q.is_file() && supported(q)).collect())
        .unwrap_or_default();
    v.sort_by_key(|q| q.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    v
}

/// « nom.ext » → « nom (2).ext » si déjà pris.
pub fn free_path(dir: &Path, stem: &str, ext: &str) -> std::path::PathBuf {
    let mut p = dir.join(format!("{stem}.{ext}"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{stem} ({i}).{ext}"));
        i += 1;
    }
    p
}
