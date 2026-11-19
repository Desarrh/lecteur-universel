//! Génère un dossier de fichiers de test (tous les formats ouverts par le lecteur).
//! `lecteur-universel --generer-tests <dossier> [largeur hauteur ips secondes]`
//! La vidéo est synthétique : barre qui balaie l'écran à vitesse constante, carré
//! qui rebondit, compteur d'images, flash + bip à chaque seconde → saccades et
//! décalage son/image se voient immédiatement.

use crate::media::Pcm;
use std::path::Path;
use windows::Win32::Media::MediaFoundation::*;
use windows::core::*;

// Chiffres 7 segments (a b c d e f g)
const SEG: [u8; 10] = [0b1111110, 0b0110000, 0b1101101, 0b1111001, 0b0110011, 0b1011011, 0b1011111, 0b1110000, 0b1111111, 0b1111011];

fn rect(px: &mut [u32], w: usize, h: usize, x: i64, y: i64, rw: i64, rh: i64, c: u32) {
    let (x0, y0) = (x.max(0) as usize, y.max(0) as usize);
    let (x1, y1) = (((x + rw).max(0) as usize).min(w), ((y + rh).max(0) as usize).min(h));
    for yy in y0..y1 {
        px[yy * w + x0..yy * w + x1.max(x0)].fill(c);
    }
}

fn digits(px: &mut [u32], w: usize, h: usize, x: i64, y: i64, s: i64, text: &str, c: u32) {
    let mut cx = x;
    for ch in text.chars() {
        if let Some(d) = ch.to_digit(10) {
            let m = SEG[d as usize];
            let (t, l) = (s / 5, s);
            let seg = |px: &mut [u32], on: bool, sx: i64, sy: i64, sw: i64, sh: i64| {
                if on {
                    rect(px, w, h, sx, sy, sw, sh, c)
                }
            };
            seg(px, m & 0b1000000 != 0, cx + t, y, l, t);
            seg(px, m & 0b0100000 != 0, cx + l + t, y + t, t, l);
            seg(px, m & 0b0010000 != 0, cx + l + t, y + l + 2 * t, t, l);
            seg(px, m & 0b0001000 != 0, cx + t, y + 2 * l + 2 * t, l, t);
            seg(px, m & 0b0000100 != 0, cx, y + l + 2 * t, t, l);
            seg(px, m & 0b0000010 != 0, cx, y + t, t, l);
            seg(px, m & 0b0000001 != 0, cx + t, y + l + t, l, t);
        }
        cx += s + s / 5 * 2 + s / 2;
        if ch == ':' {
            rect(px, w, h, cx - s / 2, y + s / 2, s / 5, s / 5, c);
            rect(px, w, h, cx - s / 2, y + s + s / 2, s / 5, s / 5, c);
        }
    }
}

fn frame(px: &mut [u32], w: usize, h: usize, i: u64, fps: u32) {
    let t = i as f64 / fps as f64;
    // Fond : dégradé bleu nuit → bleu, qui défile lentement
    let shift = (t * 120.0) as usize;
    for x in 0..w {
        let v = ((x + shift) % 1024) as f64 / 1024.0;
        let k = (v * std::f64::consts::TAU).sin() * 0.5 + 0.5;
        let r = (15.0 + 30.0 * k) as u32;
        let g = (27.0 + 60.0 * k) as u32;
        let b = (45.0 + 110.0 * k) as u32;
        px[x] = (r << 16) | (g << 8) | b;
    }
    for y in 1..h {
        px.copy_within(0..w, y * w);
    }
    // Grille fixe (repères pour juger le défilement)
    for gx in (0..w).step_by(160) {
        rect(px, w, h, gx as i64, 0, 2, h as i64, 0x003A_4A66);
    }
    // Barre verticale : un tour d'écran toutes les 4 s, vitesse constante
    let bx = ((t / 4.0).fract() * w as f64) as i64;
    rect(px, w, h, bx, 0, 24, h as i64, 0x00F5_EDDC);
    // Carré qui rebondit
    let sz = (h / 6) as i64;
    let px_ = (((t * 0.37).fract() * 2.0 - 1.0).abs() * (w as f64 - sz as f64)) as i64;
    let py_ = (((t * 0.53).fract() * 2.0 - 1.0).abs() * (h as f64 - sz as f64)) as i64;
    rect(px, w, h, px_, py_, sz, sz, 0x00F2_B84B);
    // Flash à chaque seconde (en même temps que le bip)
    if i % fps as u64 == 0 {
        rect(px, w, h, (w / 2) as i64 - 150, (h / 2) as i64 - 150, 300, 300, 0x00FF_FFFF);
    }
    // Compteur d'images et temps
    let s = (h / 14) as i64;
    rect(px, w, h, 30, 30, s * 16, s * 3 + 40, 0x000B_1524);
    digits(px, w, h, 50, 50, s, &format!("{:05}", i), 0x006F_A8FF);
    let secs = t as u64;
    rect(px, w, h, (w as i64) - s * 13 - 30, 30, s * 13, s * 3 + 40, 0x000B_1524);
    digits(px, w, h, (w as i64) - s * 13 - 10, 50, s, &format!("{:02}:{:02}", secs / 60, secs % 60), 0x00F5_EDDC);
}

/// Bip de 80 ms à chaque seconde (1 kHz), sinon silence léger.
fn beeps(secs: f64, rate: u32) -> Pcm {
    let n = (secs * rate as f64) as usize;
    let mut samples = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let v = if t.fract() < 0.08 { (t * 1000.0 * std::f64::consts::TAU).sin() * 0.5 } else { (t * 220.0 * std::f64::consts::TAU).sin() * 0.04 };
        let s = (v * 32767.0) as i16;
        samples.push(s);
        samples.push(s);
    }
    Pcm { samples, channels: 2, rate }
}

/// Mélodie courte (gamme) pour les fichiers audio.
fn melody(secs: f64, rate: u32) -> Pcm {
    let notes = [261.6, 293.7, 329.6, 349.2, 392.0, 440.0, 493.9, 523.3];
    let n = (secs * rate as f64) as usize;
    let mut samples = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let k = ((t * 2.0) as usize) % notes.len();
        let env = (1.0 - (t * 2.0).fract()).powf(2.0);
        let v = (t * notes[k] * std::f64::consts::TAU).sin() * 0.4 * env;
        samples.push((v * 32767.0) as i16);
        samples.push((v * 0.8 * 32767.0) as i16);
    }
    Pcm { samples, channels: 2, rate }
}

fn video_type(sub: &GUID, w: u32, h: u32, fps: u32) -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, sub)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        Ok(t)
    }
}

/// Vidéo synthétique + bips. `codec` : H.264 (MP4), HEVC (MP4) ou WMV (ASF).
#[allow(clippy::too_many_arguments)]
pub fn make_video(out: &Path, codec: &GUID, w: u32, h: u32, fps: u32, secs: u32, kbps: u32, hw: bool, progress: &dyn Fn(f32)) -> Result<()> {
    let _ = std::fs::remove_file(out);
    unsafe {
        let mut wa = None;
        MFCreateAttributes(&mut wa, 2)?;
        let wa = wa.unwrap();
        wa.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hw as u32)?;
        wa.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        let wr = MFCreateSinkWriterFromURL(&HSTRING::from(out.to_string_lossy().as_ref()), None, &wa)?;
        let enc = video_type(codec, w, h, fps)?;
        enc.SetUINT32(&MF_MT_AVG_BITRATE, kbps * 1000)?;
        let vi = wr.AddStream(&enc)?;
        let input = video_type(&MFVideoFormat_RGB32, w, h, fps)?;
        input.SetUINT32(&MF_MT_DEFAULT_STRIDE, w * 4)?;
        wr.SetInputMediaType(vi, &input, None)?;
        // Audio
        let wma = *codec == MFVideoFormat_WMV3;
        let rate = if wma { 44100 } else { 48000 };
        let a_out = match if wma { wma_type(rate, 2, 16000) } else { None } {
            Some(t) => t,
            None => {
                let a = MFCreateMediaType()?;
                a.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                a.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                a.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                a.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
                a.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
                a.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24000)?;
                a
            }
        };
        let ai = wr.AddStream(&a_out).ok();
        let a_in = MFCreateMediaType()?;
        a_in.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        a_in.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        a_in.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        a_in.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        a_in.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        a_in.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
        a_in.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 4)?;
        let ai = match ai {
            Some(ai) if wr.SetInputMediaType(ai, &a_in, None).is_ok() => Some(ai),
            _ => None,
        };
        wr.BeginWriting()?;
        let pcm = beeps(secs as f64, rate);
        let total = (fps * secs) as u64;
        let fdur = 10_000_000i64 / fps as i64;
        let mut px = vec![0u32; (w * h) as usize];
        let mut a_pos = 0usize;
        for i in 0..total {
            frame(&mut px, w as usize, h as usize, i, fps);
            let bytes = (w * h * 4) as usize;
            let buf = MFCreateMemoryBuffer(bytes as u32)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(px.as_ptr() as *const u8, p, bytes);
            buf.Unlock()?;
            buf.SetCurrentLength(bytes as u32)?;
            let s = MFCreateSample()?;
            s.AddBuffer(&buf)?;
            s.SetSampleTime(i as i64 * fdur)?;
            s.SetSampleDuration(fdur)?;
            wr.WriteSample(vi, &s)?;
            // Audio par tranches d'une image
            if let Some(ai) = ai {
                let end = (((i + 1) as f64 / fps as f64) * rate as f64) as usize * 2;
                let end = end.min(pcm.samples.len());
                if end > a_pos {
                    let part = &pcm.samples[a_pos..end];
                    let ab = part.len() * 2;
                    let b = MFCreateMemoryBuffer(ab as u32)?;
                    let mut q = std::ptr::null_mut();
                    b.Lock(&mut q, None, None)?;
                    std::ptr::copy_nonoverlapping(part.as_ptr() as *const u8, q, ab);
                    b.Unlock()?;
                    b.SetCurrentLength(ab as u32)?;
                    let sa = MFCreateSample()?;
                    sa.AddBuffer(&b)?;
                    sa.SetSampleTime((a_pos / 2) as i64 * 10_000_000 / rate as i64)?;
                    sa.SetSampleDuration((part.len() / 2) as i64 * 10_000_000 / rate as i64)?;
                    wr.WriteSample(ai, &sa)?;
                    a_pos = end;
                }
            }
            if i % 30 == 0 {
                progress(i as f32 / total as f32);
            }
        }
        wr.Finalize()?;
        Ok(())
    }
}

/// Type de sortie WMA réellement proposé par l'encodeur (44,1 kHz stéréo, débit le plus proche).
unsafe fn wma_type(rate: u32, ch: u32, target_bps: u32) -> Option<IMFMediaType> {
    unsafe {
        let col = MFTranscodeGetAudioOutputAvailableTypes(&MFAudioFormat_WMAudioV8, MFT_ENUM_FLAG_ALL.0 as u32, None).ok()?;
        let n = col.GetElementCount().ok()?;
        let mut best: Option<(u32, IMFMediaType)> = None;
        for i in 0..n {
            let Ok(t) = col.GetElement(i).and_then(|u| u.cast::<IMFMediaType>()) else { continue };
            let r = t.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(0);
            let c = t.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(0);
            let b = t.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0);
            if r == rate && c == ch {
                let d = b.abs_diff(target_bps);
                if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, t));
                }
            }
        }
        best.map(|b| b.1)
    }
}

/// Audio compressé via Media Foundation (AAC/.m4a, WMA, FLAC).
fn make_audio(out: &Path, sub: &GUID, pcm: &Pcm, bytes_per_sec: u32) -> Result<()> {
    let _ = std::fs::remove_file(out);
    unsafe {
        let wr = MFCreateSinkWriterFromURL(&HSTRING::from(out.to_string_lossy().as_ref()), None, None)?;
        let o = match if *sub == MFAudioFormat_WMAudioV8 { wma_type(pcm.rate, pcm.channels as u32, bytes_per_sec) } else { None } {
            Some(t) => t,
            None => {
                let o = MFCreateMediaType()?;
                o.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                o.SetGUID(&MF_MT_SUBTYPE, sub)?;
                o.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                o.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, pcm.rate)?;
                o.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, pcm.channels as u32)?;
                if bytes_per_sec > 0 {
                    o.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bytes_per_sec)?;
                }
                o
            }
        };
        let ai = wr.AddStream(&o)?;
        let i = MFCreateMediaType()?;
        i.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        i.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        i.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        i.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, pcm.rate)?;
        i.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, pcm.channels as u32)?;
        i.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, pcm.channels as u32 * 2)?;
        i.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, pcm.rate * pcm.channels as u32 * 2)?;
        wr.SetInputMediaType(ai, &i, None)?;
        wr.BeginWriting()?;
        let ch = pcm.channels as usize;
        let mut t = 0i64;
        for part in pcm.samples.chunks(pcm.rate as usize * ch / 10) {
            let ab = part.len() * 2;
            let b = MFCreateMemoryBuffer(ab as u32)?;
            let mut q = std::ptr::null_mut();
            b.Lock(&mut q, None, None)?;
            std::ptr::copy_nonoverlapping(part.as_ptr() as *const u8, q, ab);
            b.Unlock()?;
            b.SetCurrentLength(ab as u32)?;
            let s = MFCreateSample()?;
            s.AddBuffer(&b)?;
            let d = (part.len() / ch) as i64 * 10_000_000 / pcm.rate as i64;
            s.SetSampleTime(t)?;
            s.SetSampleDuration(d)?;
            t += d;
            wr.WriteSample(ai, &s)?;
        }
        wr.Finalize()?;
        Ok(())
    }
}

fn docx(out: &Path) -> std::result::Result<(), String> {
    use std::io::Write;
    let f = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default();
    let put = |z: &mut zip::ZipWriter<std::fs::File>, name: &str, s: &str| -> std::result::Result<(), String> {
        z.start_file(name, o).map_err(|e| e.to_string())?;
        z.write_all(s.as_bytes()).map_err(|e| e.to_string())
    };
    put(&mut z, "[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#)?;
    put(&mut z, "_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#)?;
    let p = |style: &str, runs: &str| format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>{runs}</w:p>");
    let r = |t: &str, b: bool, i: bool| {
        let mut pr = String::new();
        if b {
            pr.push_str("<w:b/>");
        }
        if i {
            pr.push_str("<w:i/>");
        }
        format!("<w:r><w:rPr>{pr}</w:rPr><w:t xml:space=\"preserve\">{t}</w:t></w:r>")
    };
    let mut body = String::new();
    body += &p("Title", &r("Document de test du Lecteur universel", false, false));
    body += &p("Heading1", &r("1. Mise en forme", false, false));
    body += &p("Normal", &(r("Texte normal avec des accents (é, è, à, ç, œ) puis ", false, false) + &r("du gras", true, false) + &r(", de l'", false, false) + &r("italique", false, true) + &r(" et les deux : ", false, false) + &r("gras italique", true, true) + &r(".", false, false)));
    body += &p("Heading2", &r("1.1 Liste", false, false));
    for it in ["Premier élément", "Deuxième élément", "Troisième élément"] {
        body += &p("ListParagraph", &r(it, false, false));
    }
    body += &p("Heading1", &r("2. Tableau", false, false));
    body += "<w:tbl>";
    for row in [["Format", "Lecture", "Conversion"], ["PDF", "oui", "images"], ["Word", "oui", "PDF, TXT, MD, HTML"], ["MP4", "oui", "MP4, GIF, MP3"]] {
        body += "<w:tr>";
        for c in row {
            body += &format!("<w:tc><w:p>{}</w:p></w:tc>", r(c, false, false));
        }
        body += "</w:tr>";
    }
    body += "</w:tbl>";
    for k in 0..12 {
        body += &p("Normal", &r(&format!("Paragraphe de remplissage n° {} : le lecteur doit défiler en douceur et la conversion en PDF doit paginer correctement ce document sur plusieurs pages.", k + 1), false, false));
    }
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#);
    put(&mut z, "word/document.xml", &doc)?;
    z.finish().map_err(|e| e.to_string())?;
    Ok(())
}

const TEXTS: &[(&str, &str)] = &[
    ("texte.txt", "Fichier texte de test.\nAccents : é è à ç ù œ € «guillemets»\nLigne longue : Lorem ipsum n'est pas utilisé ici, on écrit simplement une phrase assez longue pour tester le retour à la ligne automatique du lecteur universel.\n"),
    ("notes.md", "# Titre Markdown\n\nUn paragraphe avec du **gras**, de l'*italique*, du `code` et un [lien](https://example.com).\n\n## Liste\n\n- point un\n- point deux\n  - sous-point\n1. premier\n2. second\n\n> Une citation.\n\n| Format | Lecture | Conversion |\n|---|---|---|\n| PDF | oui | images, texte |\n| MP4 | oui | MP4, GIF, MP3 |\n\n---\n\n```rust\nfn main() {\n    println!(\"bonjour\");\n}\n```\n"),
    ("journal.log", "2026-11-19 14:00:01 INFO démarrage\n2026-11-19 14:00:02 WARN cache vide\n2026-11-19 14:00:03 ERROR fichier introuvable\n"),
    ("donnees.csv", "nom;type;taille\nphoto.png;image;1,2 Mo\nchanson.mp3;audio;4,5 Mo\n"),
    ("config.json", "{\n  \"theme\": \"sombre\",\n  \"volume\": 0.8,\n  \"recents\": [\"a.mp4\", \"b.pdf\"]\n}\n"),
    ("Cargo.toml", "[package]\nname = \"exemple\"\nversion = \"0.1.0\"\n\n[dependencies]\negui = \"0.37\"\n"),
    ("config.yaml", "theme: sombre\nvolume: 0.8\nformats:\n  - mp4\n  - pdf\n"),
    ("reglages.ini", "[Lecteur]\ntheme=sombre\nvolume=80\n"),
    ("page.xml", "<?xml version=\"1.0\"?>\n<lecteur version=\"1\">\n  <format nom=\"pdf\"/>\n</lecteur>\n"),
    ("page.html", "<!doctype html>\n<html lang=\"fr\"><body><h1>Bonjour</h1><p>Page de test.</p></body></html>\n"),
    ("style.css", "body {\n  background: #EFE6D3;\n  color: #0F1B2D;\n}\n"),
    ("script.js", "// JavaScript\nconst formats = ['mp4', 'pdf'];\nformats.forEach(f => console.log(`format : ${f}`));\n"),
    ("types.ts", "interface Fichier {\n  nom: string;\n  taille: number;\n}\nconst f: Fichier = { nom: 'a.pdf', taille: 42 };\n"),
    ("main.rs", "use std::path::Path;\n\n/// Point d'entrée\nfn main() {\n    let p = Path::new(\"video.mp4\");\n    println!(\"{}\", p.display());\n}\n"),
    ("script.py", "# Python\ndef bonjour(nom: str) -> str:\n    return f\"Bonjour {nom}\"\n\nprint(bonjour('monde'))\n"),
    ("prog.c", "#include <stdio.h>\n\nint main(void) {\n    printf(\"bonjour\\n\");\n    return 0;\n}\n"),
    ("prog.cpp", "#include <iostream>\nint main() { std::cout << \"bonjour\" << std::endl; }\n"),
    ("entete.h", "#pragma once\nint addition(int a, int b);\n"),
    ("Programme.cs", "using System;\nclass Programme { static void Main() { Console.WriteLine(\"bonjour\"); } }\n"),
    ("Main.java", "public class Main {\n  public static void main(String[] a) { System.out.println(\"bonjour\"); }\n}\n"),
    ("main.go", "package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(\"bonjour\") }\n"),
    ("script.sh", "#!/bin/sh\nfor f in *.mp4; do echo \"$f\"; done\n"),
    ("script.ps1", "Get-ChildItem *.mp4 | ForEach-Object { $_.Name }\n"),
    ("lancer.bat", "@echo off\necho bonjour\npause\n"),
    ("requete.sql", "SELECT nom, taille FROM fichiers WHERE type = 'video' ORDER BY taille DESC;\n"),
    ("script.lua", "local formats = { 'mp4', 'pdf' }\nfor _, f in ipairs(formats) do print(f) end\n"),
    ("joueur.gd", "extends CharacterBody2D\n\nfunc _physics_process(delta):\n\tmove_and_slide()\n"),
];

/// Génère tout le dossier de test. `big` = (largeur, hauteur, ips, secondes) de la grosse vidéo.
pub fn generate(dir: &Path, big: (u32, u32, u32, u32), log: &dyn Fn(String)) {
    let sub = |s: &str| {
        let d = dir.join(s);
        let _ = std::fs::create_dir_all(&d);
        d
    };
    let (img, aud, vid, doc, txt) = (sub("images"), sub("audio"), sub("video"), sub("documents"), sub("texte-code"));
    let ok = |name: &str, r: std::result::Result<(), String>| match r {
        Ok(_) => log(format!("OK     {name}")),
        Err(e) => log(format!("ÉCHEC  {name} : {e}")),
    };
    // Images
    let (w, h) = (1600u32, 1000u32);
    let mut im = image::RgbaImage::new(w, h);
    for (x, y, p) in im.enumerate_pixels_mut() {
        let fx = x as f32 / w as f32;
        let fy = y as f32 / h as f32;
        let ring = (((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt() * 40.0).sin() * 0.5 + 0.5;
        *p = image::Rgba([(46.0 + 200.0 * fx) as u8, (94.0 + 120.0 * ring) as u8, (158.0 + 90.0 * fy) as u8, 255]);
    }
    // zone transparente (damier visible)
    for y in 0..200 {
        for x in 0..300 {
            im.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        }
    }
    let o = crate::convert::Opts::default();
    use crate::convert::Target as T;
    for (name, t) in [("photo.png", T::Png), ("photo.jpg", T::Jpg), ("photo.webp", T::Webp), ("photo.bmp", T::Bmp), ("photo.tiff", T::Tiff), ("icone.ico", T::Ico)] {
        ok(name, crate::convert::save_raster(&im, &img.join(name), t, &o));
    }
    let _ = std::fs::copy(img.join("photo.jpg"), img.join("photo2.jpeg"));
    let _ = std::fs::copy(img.join("photo.tiff"), img.join("photo3.tif"));
    log("OK     photo2.jpeg, photo3.tif".into());
    let mut lossy = o.clone();
    lossy.webp_quality = 80;
    ok("photo-avec-perte.webp", crate::convert::save_raster(&im, &img.join("photo-avec-perte.webp"), T::Webp, &lossy));
    let frames: Vec<image::RgbaImage> = (0..36)
        .map(|i| {
            let mut f = image::RgbaImage::from_pixel(320, 200, image::Rgba([15, 27, 45, 255]));
            let cx = (i as f32 / 36.0 * std::f32::consts::TAU).cos() * 70.0 + 160.0;
            let cy = (i as f32 / 36.0 * std::f32::consts::TAU).sin() * 60.0 + 100.0;
            for (x, y, p) in f.enumerate_pixels_mut() {
                if ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() < 22.0 {
                    *p = image::Rgba([242, 184, 75, 255]);
                }
            }
            f
        })
        .collect();
    ok("anime.gif", crate::convert::write_gif(&img.join("anime.gif"), frames, 24.0, &|_| {}));
    ok("photo.heic", crate::wic::encode_heic(&im, &img.join("photo.heic")));
    // Audio
    let pcm = melody(16.0, 44100);
    ok("melodie.wav", crate::media::write_wav(&pcm, &aud.join("melodie.wav").to_string_lossy()));
    ok("melodie.mp3", crate::media::write_mp3(&pcm, &aud.join("melodie.mp3").to_string_lossy(), 192, &|_| {}).map_err(|e| e.message().to_string()));
    ok("melodie.m4a", make_audio(&aud.join("melodie.m4a"), &MFAudioFormat_AAC, &pcm, 24000).map_err(|e| e.message().to_string()));
    ok("melodie.wma", make_audio(&aud.join("melodie.wma"), &MFAudioFormat_WMAudioV8, &pcm, 16000).map_err(|e| e.message().to_string()));
    ok("melodie.flac", make_audio(&aud.join("melodie.flac"), &MFAudioFormat_FLAC, &pcm, 0).map_err(|e| e.message().to_string()));
    // Documents
    ok("document.docx", docx(&doc.join("document.docx")));
    if let Ok(b) = crate::docx::parse(&doc.join("document.docx").to_string_lossy()) {
        ok("document.pdf", crate::pdfgen::docx_to_pdf(&b, &doc.join("document.pdf")));
    }
    // Texte / code
    for (name, body) in TEXTS {
        ok(name, std::fs::write(txt.join(name), body).map_err(|e| e.to_string()));
    }
    // Petites vidéos (formats)
    let small = |name: &str, codec: &GUID, w: u32, h: u32, fps: u32, s: u32| {
        let p = vid.join(name);
        let mut r = make_video(&p, codec, w, h, fps, s, 6000, true, &|_| {});
        if r.is_err() {
            r = make_video(&p, codec, w, h, fps, s, 6000, false, &|_| {});
        }
        ok(name, r.map_err(|e| e.message().to_string()));
    };
    small("court-720p-30ips.mp4", &MFVideoFormat_H264, 1280, 720, 30, 8);
    ok(
        "court-720p-30ips.fr.srt",
        std::fs::write(
            vid.join("court-720p-30ips.fr.srt"),
            "1\r\n00:00:00,500 --> 00:00:03,000\r\nSous-titres de test (chargés automatiquement).\r\n\r\n2\r\n00:00:03,500 --> 00:00:07,500\r\nTouche V : afficher / masquer — G / H : décalage.\r\n",
        )
        .map_err(|e| e.to_string()),
    );
    small("court-1080p-60ips.mp4", &MFVideoFormat_H264, 1920, 1080, 60, 8);
    small("court-vertical.mp4", &MFVideoFormat_H264, 1080, 1920, 30, 6);
    small("court.wmv", &MFVideoFormat_WMV3, 1280, 720, 30, 6);
    let _ = std::fs::copy(vid.join("court-720p-30ips.mp4"), vid.join("court.m4v"));
    log("OK     court.m4v".into());
    // Grosse vidéo à la résolution de l'écran
    let (bw, bh, fps, secs) = big;
    let name = format!("ecran-{bw}x{bh}-{fps}ips.mp4");
    let p = vid.join(&name);
    let t0 = std::time::Instant::now();
    let pr = |f: f32| {
        if (f * 100.0) as u32 % 10 == 0 {
            eprint!("\r  {name} : {:.0} %   ", f * 100.0);
        }
    };
    let mut r = Err(Error::from(windows::Win32::Foundation::E_FAIL));
    for (codec, hw, label) in [(&MFVideoFormat_H264, true, "H.264 matériel"), (&MFVideoFormat_H264, false, "H.264 logiciel"), (&MFVideoFormat_HEVC, true, "HEVC matériel")] {
        r = make_video(&p, codec, bw, bh, fps, secs, 120_000, hw, &pr);
        if r.is_ok() {
            log(format!("\nOK     {name} ({label}, {:.0} s de calcul)", t0.elapsed().as_secs_f32()));
            break;
        }
        log(format!("\n       {label} : {}", r.as_ref().err().map(|e| e.message().to_string()).unwrap_or_default()));
    }
    if r.is_err() {
        log(format!("ÉCHEC  {name}"));
    }
}
