//! Montage vidéo via Media Foundation : vignettes de la frise, coupe sans
//! réencodage (copie des échantillons compressés), réencodage H.264/AAC.

use crate::media::{duration_of, pv_i64, reader};
use std::collections::{HashMap, HashSet};
use windows::Win32::Media::MediaFoundation::*;
use windows::core::*;

/// Une vignette par position : un « seek » chacune, pas de décodage de toute
/// la vidéo (rapide même sur un fichier de 2 Go).
pub fn video_thumbs(path: &str, n: usize, max_side: u32, on_thumb: &dyn Fn(usize, image::RgbaImage) -> bool) -> Result<()> {
    unsafe {
        // Le décodeur réduit lui-même l'image (traitement vidéo avancé) : bien plus
        // rapide que convertir une image 2560×720 puis la redimensionner.
        let mut ra = None;
        MFCreateAttributes(&mut ra, 1)?;
        let ra = ra.unwrap();
        ra.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let r = MFCreateSourceReaderFromURL(&HSTRING::from(path), &ra)?;
        let v = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        r.SetStreamSelection(v, true)?;
        let native = r.GetNativeMediaType(v, 0)?;
        let fs = native.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (fw, fh) = ((fs >> 32) as u32, (fs & 0xffff_ffff) as u32);
        let sc = (max_side as f64 / fw.max(fh) as f64).min(1.0);
        let (tw, th) = ((((fw as f64 * sc) as u32).max(2) + 1) & !1, (((fh as f64 * sc) as u32).max(2) + 1) & !1);
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        mt.SetUINT64(&MF_MT_FRAME_SIZE, ((tw as u64) << 32) | th as u64)?;
        let scaled = r.SetCurrentMediaType(v, None, &mt).is_ok();
        if !scaled {
            let mt = MFCreateMediaType()?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            r.SetCurrentMediaType(v, None, &mt)?;
        }
        let cur = r.GetCurrentMediaType(v)?;
        let cs = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (cw, ch) = ((cs >> 32) as u32, (cs & 0xffff_ffff) as u32);
        let dur = duration_of(&r);
        // Ordre « grossier → fin » : la frise se remplit partout tout de suite.
        let mut order = Vec::with_capacity(n);
        let mut step = n.next_power_of_two();
        while step > 0 {
            for i in (0..n).step_by(step) {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
            step /= 2;
        }
        for i in order {
            let t = dur * (i as f64 + 0.5) / n as f64;
            r.SetCurrentPosition(&GUID::zeroed(), &pv_i64((t * 1e7) as i64))?;
            for _ in 0..8 {
                let mut flags = 0u32;
                let mut sample = None;
                r.ReadSample(v, 0, None, Some(&mut flags), None, Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(s) = sample else { continue };
                let img = sample_to_rgba(&s, cw, ch)?;
                let img = if cw == tw && ch == th { img } else { image::imageops::resize(&img, tw, th, image::imageops::FilterType::Triangle) };
                // Faux : plus personne n'attend (onglet fermé) → on arrête tout de suite
                if !on_thumb(i, img) {
                    return Ok(());
                }
                break;
            }
        }
        Ok(())
    }
}

pub unsafe fn sample_to_rgba(s: &IMFSample, fw: u32, fh: u32) -> Result<image::RgbaImage> {
    unsafe {
        let buf = s.ConvertToContiguousBuffer()?;
        let mut img = image::RgbaImage::new(fw, fh);
        let row_len = fw as usize * 4;
        if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
            let mut p0 = std::ptr::null_mut();
            let mut pitch = 0i32;
            b2.Lock2D(&mut p0, &mut pitch)?;
            for y in 0..fh as usize {
                let row = std::slice::from_raw_parts(p0.offset(y as isize * pitch as isize), row_len);
                let out = &mut img.as_mut()[y * row_len..(y + 1) * row_len];
                for (d, px) in out.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                    d.copy_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
            b2.Unlock2D()?;
        }
        Ok(img)
    }
}

fn streams(r: &IMFSourceReader) -> Vec<(u32, bool)> {
    let mut v = Vec::new();
    for i in 0..16u32 {
        match unsafe { r.GetNativeMediaType(i, 0) } {
            Ok(mt) => {
                let major = unsafe { mt.GetMajorType() }.unwrap_or_default();
                if major == MFMediaType_Video {
                    v.push((i, true));
                } else if major == MFMediaType_Audio {
                    v.push((i, false));
                }
            }
            Err(_) => break,
        }
    }
    v
}

/// Coupe sans réencodage : copie des échantillons compressés de la plage, en
/// partant de l'image clé qui précède `t0`. Instantané, qualité intacte.
pub fn remux(path: &str, out: &str, t0: f64, t1: f64, progress: &dyn Fn(f32)) -> Result<()> {
    let r = unsafe { remux_inner(path, out, t0, t1, progress) };
    if r.is_err() {
        let _ = std::fs::remove_file(out);
    }
    r
}

unsafe fn remux_inner(path: &str, out: &str, t0: f64, t1: f64, progress: &dyn Fn(f32)) -> Result<()> {
    unsafe {
        let r = reader(path, false)?;
        let st = streams(&r);
        let w = MFCreateSinkWriterFromURL(&HSTRING::from(out), None, None)?;
        let mut map = HashMap::new();
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        for (i, _) in &st {
            let mt = r.GetNativeMediaType(*i, 0)?;
            r.SetStreamSelection(*i, true)?;
            r.SetCurrentMediaType(*i, None, &mt)?;
            let wi = w.AddStream(&mt)?;
            w.SetInputMediaType(wi, &mt, None)?;
            map.insert(*i, wi);
        }
        w.BeginWriting()?;
        r.SetCurrentPosition(&GUID::zeroed(), &pv_i64((t0 * 1e7) as i64))?;
        let video_idx = st.iter().find(|s| s.1).map(|s| s.0);
        let mut base: Option<i64> = None;
        let mut done = HashSet::new();
        let end = (t1 * 1e7) as i64;
        while done.len() < st.len() {
            let mut idx = 0u32;
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            r.ReadSample(MF_SOURCE_READER_ANY_STREAM.0 as u32, 0, Some(&mut idx), Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                done.insert(idx);
                continue;
            }
            let Some(s) = sample else { continue };
            if done.contains(&idx) {
                continue;
            }
            if ts > end {
                done.insert(idx);
                continue;
            }
            // Temps zéro = la première image vidéo (clé) ; l'audio d'avant est ignoré.
            if base.is_none() {
                if Some(idx) == video_idx || video_idx.is_none() {
                    base = Some(ts);
                } else {
                    continue;
                }
            }
            let b = base.unwrap_or(0);
            if ts < b {
                continue;
            }
            s.SetSampleTime(ts - b)?;
            if let Some(wi) = map.get(&idx) {
                w.WriteSample(*wi, &s)?;
            }
            progress(((ts as f64 / 1e7 - t0) / (t1 - t0).max(0.01)).clamp(0.0, 1.0) as f32);
        }
        w.Finalize()?;
        Ok(())
    }
}

/// Réencodage H.264 + AAC : coupe précise à l'image près, ou recompression.
/// `max_h` = hauteur maxi (0 = native), `kbps` = débit vidéo.
pub fn transcode(path: &str, out: &str, t0: f64, t1: f64, max_h: u32, kbps: u32, progress: &dyn Fn(f32)) -> Result<()> {
    let mut r = unsafe { transcode_inner(path, out, t0, t1, max_h, kbps, true, progress) };
    if r.is_err() {
        let _ = std::fs::remove_file(out);
        // Sans encodeur matériel : encodeur logiciel intégré à Windows.
        r = unsafe { transcode_inner(path, out, t0, t1, max_h, kbps, false, progress) };
    }
    if r.is_err() {
        let _ = std::fs::remove_file(out);
    }
    r
}

#[allow(clippy::too_many_arguments)]
unsafe fn transcode_inner(path: &str, out: &str, t0: f64, t1: f64, max_h: u32, kbps: u32, hw: bool, progress: &dyn Fn(f32)) -> Result<()> {
    unsafe {
        let mut ra = None;
        MFCreateAttributes(&mut ra, 1)?;
        let ra = ra.unwrap();
        ra.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        let r = MFCreateSourceReaderFromURL(&HSTRING::from(path), &ra)?;
        let st = streams(&r);
        let mut wa = None;
        MFCreateAttributes(&mut wa, 2)?;
        let wa = wa.unwrap();
        wa.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, hw as u32)?;
        wa.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
        let w = MFCreateSinkWriterFromURL(&HSTRING::from(out), None, &wa)?;
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        let mut map = HashMap::new();
        let mut video_idx = None;
        let mut audio_done = false;
        for (i, is_video) in &st {
            let native = r.GetNativeMediaType(*i, 0)?;
            if *is_video {
                if video_idx.is_some() {
                    continue;
                }
                let fs = native.GetUINT64(&MF_MT_FRAME_SIZE)?;
                let (mut fw, mut fh) = ((fs >> 32) as u32, (fs & 0xffff_ffff) as u32);
                if max_h > 0 && fh > max_h {
                    fw = ((fw as u64 * max_h as u64 / fh as u64) as u32) & !1;
                    fh = max_h & !1;
                }
                let size = ((fw as u64) << 32) | fh as u64;
                let rate = native.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or((30u64 << 32) | 1);
                let dec = MFCreateMediaType()?;
                dec.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                dec.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                dec.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
                r.SetStreamSelection(*i, true)?;
                r.SetCurrentMediaType(*i, None, &dec)?;
                let mk = |sub: &GUID| -> Result<IMFMediaType> {
                    let t = MFCreateMediaType()?;
                    t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    t.SetGUID(&MF_MT_SUBTYPE, sub)?;
                    t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                    t.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
                    t.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
                    t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
                    Ok(t)
                };
                let enc = mk(&MFVideoFormat_H264)?;
                enc.SetUINT32(&MF_MT_AVG_BITRATE, kbps * 1000)?;
                let wi = w.AddStream(&enc)?;
                w.SetInputMediaType(wi, &mk(&MFVideoFormat_NV12)?, None)?;
                map.insert(*i, wi);
                video_idx = Some(*i);
            } else {
                if audio_done {
                    continue;
                }
                let rate = native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(48000);
                let ch = native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2).min(2);
                if rate != 44100 && rate != 48000 {
                    continue; // l'encodeur AAC de Windows n'accepte que 44,1/48 kHz
                }
                let pcm = MFCreateMediaType()?;
                pcm.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                pcm.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
                pcm.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                pcm.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
                pcm.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch)?;
                r.SetStreamSelection(*i, true)?;
                r.SetCurrentMediaType(*i, None, &pcm)?;
                let cur = r.GetCurrentMediaType(*i)?;
                let aac = MFCreateMediaType()?;
                aac.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
                aac.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
                aac.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                aac.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
                aac.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, ch)?;
                aac.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 20000)?;
                let wi = w.AddStream(&aac)?;
                w.SetInputMediaType(wi, &cur, None)?;
                map.insert(*i, wi);
                audio_done = true;
            }
        }
        if video_idx.is_none() {
            return Err(Error::from(windows::Win32::Foundation::E_FAIL));
        }
        w.BeginWriting()?;
        if t0 > 0.0 {
            r.SetCurrentPosition(&GUID::zeroed(), &pv_i64((t0 * 1e7) as i64))?;
        }
        let start = (t0 * 1e7) as i64;
        let end = (t1 * 1e7) as i64;
        let mut done = HashSet::new();
        while done.len() < map.len() {
            let mut idx = 0u32;
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            r.ReadSample(MF_SOURCE_READER_ANY_STREAM.0 as u32, 0, Some(&mut idx), Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                done.insert(idx);
                continue;
            }
            let Some(s) = sample else { continue };
            let Some(wi) = map.get(&idx) else { continue };
            if done.contains(&idx) {
                continue;
            }
            if ts > end {
                done.insert(idx);
                continue;
            }
            // Coupe précise : on jette ce qui précède t0 (décodé depuis l'image clé).
            if ts < start {
                continue;
            }
            s.SetSampleTime(ts - start)?;
            w.WriteSample(*wi, &s)?;
            if Some(idx) == video_idx {
                progress(((ts - start) as f64 / (end - start).max(1) as f64).clamp(0.0, 1.0) as f32);
            }
        }
        w.Finalize()?;
        Ok(())
    }
}
