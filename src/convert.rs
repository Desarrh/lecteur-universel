//! Conversions en « graphe à pivots », à la manière de ConvertX / VERT :
//! chaque format d'entrée se décode vers un pivot (image, son PCM, suite d'images,
//! document, texte) et chaque format de sortie s'encode depuis un pivot.
//! Les cibles proposées se déduisent donc du graphe au lieu d'être codées une à une,
//! et ajouter un format = ajouter un décodeur ou un encodeur.

use crate::kind::{Kind, free_path};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Png,
    Jpg,
    Webp,
    Bmp,
    Tiff,
    Ico,
    Gif,
    Wav,
    Mp3,
    Txt,
    Md,
    Html,
    Pdf,
    Mp4,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hub {
    Raster,
    Pcm,
    Frames,
    Doc,
    Text,
    /// Flux vidéo décodé (réencodage H.264)
    Stream,
}

impl Target {
    pub fn ext(self) -> &'static str {
        match self {
            Target::Png => "png",
            Target::Jpg => "jpg",
            Target::Webp => "webp",
            Target::Bmp => "bmp",
            Target::Tiff => "tiff",
            Target::Ico => "ico",
            Target::Gif => "gif",
            Target::Wav => "wav",
            Target::Mp3 => "mp3",
            Target::Txt => "txt",
            Target::Md => "md",
            Target::Html => "html",
            Target::Pdf => "pdf",
            Target::Mp4 => "mp4",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Target::Md => "Markdown",
            Target::Tiff => "TIFF",
            _ => match self.ext() {
                "png" => "PNG",
                "jpg" => "JPG",
                "webp" => "WEBP",
                "bmp" => "BMP",
                "ico" => "ICO",
                "gif" => "GIF",
                "wav" => "WAV",
                "mp3" => "MP3",
                "txt" => "TXT",
                "pdf" => "PDF",
                "mp4" => "MP4",
                _ => "HTML",
            },
        }
    }
    pub fn hint(self, k: Kind) -> &'static str {
        match (self, k) {
            (Target::Gif, Kind::Video) => "animation",
            (Target::Png, Kind::Video) => "image actuelle",
            (Target::Mp3, Kind::Video) => "extraire le son",
            (Target::Txt, Kind::Pdf) => "texte des pages",
            (Target::Html, Kind::Pdf) => "texte, page web",
            (_, Kind::Pdf) => "une image par page",
            (Target::Png, _) => "sans perte",
            (Target::Jpg, _) => "léger, photos",
            (Target::Webp, _) => "web, très léger",
            (Target::Mp4, _) => "plus léger (H.264)",
            (Target::Pdf, Kind::Docx) => "mise en page intégrée",
            (Target::Pdf, Kind::Image) => "page A4, à envoyer",
            (Target::Pdf, _) => "document imprimable",
            (Target::Bmp, _) => "brut",
            (Target::Tiff, _) => "impression",
            (Target::Ico, _) => "icône 256 px",
            (Target::Gif, _) => "image fixe",
            (Target::Wav, _) => "sans perte",
            (Target::Mp3, _) => "compressé",
            (Target::Txt, _) => "texte brut",
            (Target::Md, _) => "titres, listes",
            (Target::Html, _) => "page web",
        }
    }
    fn hub(self, k: Kind) -> Hub {
        match self {
            Target::Wav | Target::Mp3 => Hub::Pcm,
            Target::Mp4 => Hub::Stream,
            Target::Pdf if k == Kind::Docx => Hub::Doc,
            Target::Pdf if k == Kind::Image => Hub::Raster,
            Target::Pdf => Hub::Text,
            Target::Gif if k == Kind::Video => Hub::Frames,
            Target::Txt | Target::Md | Target::Html if k == Kind::Docx => Hub::Doc,
            Target::Txt | Target::Md | Target::Html => Hub::Text,
            _ => Hub::Raster,
        }
    }
}

fn decoders(k: Kind) -> &'static [Hub] {
    match k {
        Kind::Image => &[Hub::Raster],
        Kind::Pdf => &[Hub::Raster, Hub::Text],
        Kind::Audio => &[Hub::Pcm],
        Kind::Video => &[Hub::Stream, Hub::Frames, Hub::Pcm, Hub::Raster],
        Kind::Docx => &[Hub::Doc],
        Kind::Text => &[Hub::Text],
        Kind::Unknown => &[],
    }
}

fn encoders(h: Hub) -> &'static [Target] {
    match h {
        Hub::Raster => &[Target::Pdf, Target::Png, Target::Jpg, Target::Webp, Target::Bmp, Target::Tiff, Target::Ico, Target::Gif],
        Hub::Pcm => &[Target::Mp3, Target::Wav],
        Hub::Frames => &[Target::Gif],
        Hub::Doc => &[Target::Pdf, Target::Txt, Target::Md, Target::Html],
        Hub::Text => &[Target::Pdf, Target::Html, Target::Txt],
        Hub::Stream => &[Target::Mp4],
    }
}

/// Cibles possibles pour un type d'entrée (sans le format d'origine).
pub fn targets(k: Kind, src_ext: &str) -> Vec<Target> {
    let mut v: Vec<Target> = Vec::new();
    for h in decoders(k) {
        for t in encoders(*h) {
            if !v.contains(t) && !((t.ext() == src_ext && *t != Target::Mp4) || (src_ext == "jpeg" && *t == Target::Jpg) || (src_ext == "tif" && *t == Target::Tiff)) {
                // Une image fixe en PNG/JPG depuis une vidéo : seul PNG est utile.
                if k == Kind::Video && *h == Hub::Raster && *t != Target::Png {
                    continue;
                }
                if *h == Hub::Raster && *t == Target::Pdf && k != Kind::Image {
                    continue;
                }
                if k == Kind::Pdf && matches!(t, Target::Ico | Target::Gif | Target::Tiff | Target::Bmp) {
                    continue;
                }
                v.push(*t);
            }
        }
    }
    v
}

#[derive(Clone)]
pub struct Opts {
    pub jpg_quality: u8,
    pub mp3_kbps: u32,
    pub gif_fps: f64,
    pub gif_width: u32,
    pub pdf_width: u32,
    pub webp_quality: u8,
    pub mp4_height: u32,
    pub mp4_kbps: u32,
    /// Image → PDF : rendu « document scanné », et assemblage de plusieurs images en un seul PDF
    pub pdf_scan: bool,
    pub pdf_merge: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { jpg_quality: 88, mp3_kbps: 192, gif_fps: 12.0, gif_width: 480, pdf_width: 1654, webp_quality: 82, mp4_height: 1080, mp4_kbps: 6000, pdf_scan: false, pdf_merge: true }
    }
}

/// Ce qu'il faut savoir de la source (dont les modifications en cours).
pub struct Source {
    pub path: PathBuf,
    pub kind: Kind,
    pub image: Option<image::RgbaImage>,
    pub text: Option<String>,
    pub range: Option<(f64, f64)>,
    pub at: f64,
    pub duration: f64,
}

pub fn run(src: &Source, t: Target, o: &Opts, progress: &dyn Fn(f32)) -> Result<Vec<PathBuf>, String> {
    let path = src.path.to_string_lossy().to_string();
    let dir = src.path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let stem = src.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "sortie".into());
    let out = |ext: &str| free_path(&dir, &stem, ext);
    match t.hub(src.kind) {
        Hub::Raster => {
            if src.kind == Kind::Pdf {
                let folder = free_path(&dir, &format!("{stem}-pages"), "").with_extension("");
                let doc_pages = crate::pdf::export_pngs(&path, &folder, o.pdf_width, progress)?;
                if t != Target::Png {
                    for i in 1..=doc_pages {
                        let p = folder.join(format!("page-{i:03}.png"));
                        let img = image::open(&p).map_err(|e| e.to_string())?.to_rgba8();
                        save_raster(&img, &p.with_extension(t.ext()), t, o)?;
                        let _ = std::fs::remove_file(&p);
                    }
                }
                return Ok(vec![folder]);
            }
            let img = if src.kind == Kind::Video {
                let mut f = crate::media::video_frames(&path, src.at, src.at + 0.5, 1.0, 0, &|_| {}).map_err(|e| e.message().to_string())?;
                if f.is_empty() {
                    return Err("aucune image à cet instant".into());
                }
                f.remove(0)
            } else {
                match &src.image {
                    Some(i) => i.clone(),
                    None => crate::wic::open_any(&src.path)?,
                }
            };
            let p = if src.kind == Kind::Video { free_path(&dir, &format!("{stem} {}", crate::kind::hms(src.at).replace(':', "m")), t.ext()) } else { out(t.ext()) };
            if t == Target::Pdf {
                crate::pdfgen::images_to_pdf(&[crate::pdfgen::page_of(&img, o.pdf_scan)], &p, 85)?;
                progress(1.0);
                return Ok(vec![p]);
            }
            save_raster(&img, &p, t, o)?;
            progress(1.0);
            Ok(vec![p])
        }
        Hub::Pcm => {
            // MP3 en flux quand c'est possible : une vidéo d'une heure ne passe pas par 700 Mo de PCM
            if t == Target::Mp3 {
                let p = out(t.ext());
                let ps = p.to_string_lossy().to_string();
                match crate::media::write_mp3_stream(&path, src.range, &ps, o.mp3_kbps, progress) {
                    Ok(true) => {
                        progress(1.0);
                        return Ok(vec![p]);
                    }
                    Ok(false) => {}
                    Err(e) => return Err(format!("encodeur MP3 de Windows : {}", e.message())),
                }
            }
            let pcm = crate::media::decode_pcm(&path, src.range, &|f| progress(f * 0.5)).map_err(|e| e.message().to_string())?;
            if pcm.samples.is_empty() {
                return Err("aucune piste audio".into());
            }
            let p = out(t.ext());
            let ps = p.to_string_lossy().to_string();
            if t == Target::Wav {
                crate::media::write_wav(&pcm, &ps)?;
            } else {
                crate::media::write_mp3(&pcm, &ps, o.mp3_kbps, &|f| progress(0.5 + f * 0.5))
                    .map_err(|e| format!("encodeur MP3 de Windows : {}", e.message()))?;
            }
            Ok(vec![p])
        }
        Hub::Frames => {
            let (t0, t1) = src.range.unwrap_or((src.at, (src.at + 5.0).min(src.duration.max(src.at + 0.1))));
            let frames = crate::media::video_frames(&path, t0, t1, o.gif_fps, o.gif_width, &|f| progress(f * 0.6)).map_err(|e| e.message().to_string())?;
            if frames.is_empty() {
                return Err("aucune image dans la plage".into());
            }
            let p = out("gif");
            write_gif(&p, frames, o.gif_fps, &|f| progress(0.6 + 0.4 * f))?;
            Ok(vec![p])
        }
        Hub::Stream => {
            let (t0, t1) = src.range.unwrap_or((0.0, src.duration.max(1e6)));
            let suffix = if src.range.is_some() { " (extrait)" } else { " (compressé)" };
            let p = free_path(&dir, &format!("{stem}{suffix}"), "mp4");
            crate::montage::transcode(&path, &p.to_string_lossy(), t0, t1, o.mp4_height, o.mp4_kbps, progress).map_err(|e| e.message().to_string())?;
            Ok(vec![p])
        }
        Hub::Doc => {
            if t == Target::Pdf {
                let p = out("pdf");
                progress(0.1);
                if crate::pdfgen::word_export(&src.path, &p).is_err() {
                    let b = crate::docx::parse(&path)?;
                    crate::pdfgen::docx_to_pdf(&b, &p)?;
                }
                progress(1.0);
                return Ok(vec![p]);
            }
            let b = crate::docx::parse(&path)?;
            let s = match t {
                Target::Txt => crate::docx::to_txt(&b),
                Target::Md => crate::docx::to_md(&b),
                _ => crate::docx::to_html(&b, &stem),
            };
            let p = out(t.ext());
            std::fs::write(&p, s).map_err(|e| e.to_string())?;
            Ok(vec![p])
        }
        Hub::Text => {
            let text = match &src.text {
                Some(s) => s.clone(),
                None if src.kind == Kind::Pdf => {
                    let pages = crate::pdf::page_texts(&path)?;
                    if pages.iter().all(|p| p.trim().is_empty()) {
                        return Err("aucun texte dans ce PDF (document scanné ?)".into());
                    }
                    pages.iter().enumerate().map(|(i, t)| format!("--- Page {} ---\n{t}\n", i + 1)).collect::<Vec<_>>().join("\n")
                }
                None => String::from_utf8_lossy(&std::fs::read(&src.path).map_err(|e| e.to_string())?).into_owned(),
            };
            if t == Target::Pdf {
                let p = out("pdf");
                crate::pdfgen::text_to_pdf(&text, &p)?;
                return Ok(vec![p]);
            }
            let s = if t == Target::Html {
                format!(
                    "<!doctype html><html lang=\"fr\"><meta charset=\"utf-8\"><title>{0}</title><body><pre style=\"font:14px Consolas,monospace;white-space:pre-wrap\">{1}</pre></body></html>\n",
                    crate::docx::esc(&stem),
                    crate::docx::esc(&text)
                )
            } else {
                text
            };
            let p = out(t.ext());
            std::fs::write(&p, s).map_err(|e| e.to_string())?;
            Ok(vec![p])
        }
    }
}

/// GIF optimisé : seuls les pixels qui changent d'une image à l'autre sont écrits
/// (les autres sont transparents et laissent voir l'image précédente), comme les
/// encodeurs « optimisés » (gifsicle, ezgif). Divise souvent la taille par 3 à 10.
pub fn write_gif(p: &Path, frames: Vec<image::RgbaImage>, fps: f64, progress: &dyn Fn(f32)) -> Result<(), String> {
    let Some(first) = frames.first() else { return Err("aucune image".into()) };
    let (w, h) = (first.width() as u16, first.height() as u16);
    let f = std::fs::File::create(p).map_err(|e| e.to_string())?;
    let mut enc = gif::Encoder::new(std::io::BufWriter::new(f), w, h, &[]).map_err(|e| e.to_string())?;
    enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
    let delay = (100.0 / fps).round() as u16;
    let n = frames.len();
    let mut prev: Option<image::RgbaImage> = None;
    for (i, fr) in frames.into_iter().enumerate() {
        let mut px = fr.clone().into_raw();
        if let Some(pv) = &prev {
            for (d, s) in px.chunks_exact_mut(4).zip(pv.as_raw().chunks_exact(4)) {
                let diff = (d[0] as i32 - s[0] as i32).abs() + (d[1] as i32 - s[1] as i32).abs() + (d[2] as i32 - s[2] as i32).abs();
                if diff < 18 {
                    d[3] = 0;
                }
            }
        }
        let mut frame = gif::Frame::from_rgba_speed(w, h, &mut px, 10);
        frame.delay = delay;
        frame.dispose = gif::DisposalMethod::Keep;
        enc.write_frame(&frame).map_err(|e| e.to_string())?;
        // L'image « vue » = la précédente + les pixels changés.
        prev = Some(match prev {
            Some(mut pv) => {
                for (d, s) in pv.as_mut().chunks_exact_mut(4).zip(fr.as_raw().chunks_exact(4)) {
                    let diff = (d[0] as i32 - s[0] as i32).abs() + (d[1] as i32 - s[1] as i32).abs() + (d[2] as i32 - s[2] as i32).abs();
                    if diff >= 18 {
                        d.copy_from_slice(s);
                    }
                }
                pv
            }
            None => fr,
        });
        progress((i + 1) as f32 / n as f32);
    }
    Ok(())
}

pub fn save_raster(img: &image::RgbaImage, p: &Path, t: Target, o: &Opts) -> Result<(), String> {
    use image::ImageFormat as F;
    let e = |e: image::ImageError| e.to_string();
    if t == Target::Webp && img.width().max(img.height()) > 16_383 {
        return Err("WebP : 16 383 px de côté au plus (réduisez l'image d'abord)".into());
    }
    match t {
        Target::Jpg => {
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            let f = std::fs::File::create(p).map_err(|e| e.to_string())?;
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(f), o.jpg_quality);
            enc.encode_image(&rgb).map_err(e)
        }
        Target::Ico => {
            let k = (256.0 / img.width().max(img.height()) as f32).min(1.0);
            let (w, h) = (((img.width() as f32 * k).round() as u32).max(1), ((img.height() as f32 * k).round() as u32).max(1));
            let s = image::imageops::resize(img, w, h, image::imageops::FilterType::Lanczos3);
            s.save_with_format(p, F::Ico).map_err(e)
        }
        Target::Bmp => image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().save_with_format(p, F::Bmp).map_err(e),
        Target::Png => img.save_with_format(p, F::Png).map_err(e),
        Target::Webp if o.webp_quality < 100 => {
            let enc = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height());
            std::fs::write(p, &*enc.encode(o.webp_quality as f32)).map_err(|e| e.to_string())
        }
        Target::Webp => img.save_with_format(p, F::WebP).map_err(e),
        Target::Tiff => img.save_with_format(p, F::Tiff).map_err(e),
        Target::Gif => img.save_with_format(p, F::Gif).map_err(e),
        _ => Err("format non image".into()),
    }
}
