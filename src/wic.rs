//! Images : décodage complet par le socle ; ici, l'aperçu réduit rapide par Windows (WIC)
//! et l'encodeur HEIC (fichiers de test).

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::core::HSTRING;

/// Décode n'importe quelle image lisible par le socle : `image` d'abord, puis PSD, RAW, SVG, EXR,
/// JXL en Rust, et WIC (HEIC, AVIF, JXR…) en repli, avec un message clair s'il manque une extension.
pub fn open_any(path: &std::path::Path) -> Result<image::RgbaImage, String> {
    socle::images::decode(path, &socle::Cancel::new()).or_else(|e| by_extension(path).ok_or(e))
}

/// Secours : format donné par l'extension (un TGA non compressé commence comme un curseur .cur,
/// que la détection par en-tête du socle prend pour une icône).
fn by_extension(path: &std::path::Path) -> Option<image::RgbaImage> {
    let fmt = image::ImageFormat::from_path(path).ok()?;
    let r = image::ImageReader::open(path).ok()?;
    let mut r = r;
    r.set_format(fmt);
    r.decode().ok().map(|i| i.to_rgba8())
}

/// Encode en HEIC avec l'encodeur de Windows (extension HEIF requise) — sert aux fichiers de test.
pub fn encode_heic(img: &image::RgbaImage, out: &std::path::Path) -> Result<(), String> {
    let r = encode_heic_inner(img, out);
    if r.is_err() {
        let _ = std::fs::remove_file(out);
    }
    r.map_err(|e| format!("{e} (l'encodeur HEIF de Windows demande aussi l'extension « Extensions vidéo HEVC »)"))
}

fn encode_heic_inner(img: &image::RgbaImage, out: &std::path::Path) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let e = |x: windows::core::Error| x.message().to_string();
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        let stream = f.CreateStream().map_err(e)?;
        stream.InitializeFromFilename(&HSTRING::from(out.to_string_lossy().as_ref()), GENERIC_WRITE.0).map_err(e)?;
        let enc = f.CreateEncoder(&GUID_ContainerFormatHeif, std::ptr::null()).map_err(|_| "pas d'encodeur HEIF (extension HEIF absente)".to_string())?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache).map_err(e)?;
        let mut frame = None;
        enc.CreateNewFrame(&mut frame, std::ptr::null_mut()).map_err(e)?;
        let frame = frame.ok_or("cadre HEIF")?;
        frame.Initialize(None).map_err(e)?;
        let (w, h) = img.dimensions();
        frame.SetSize(w, h).map_err(e)?;
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt).map_err(e)?;
        let mut bgra: Vec<u8> = img.as_raw().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        if fmt != GUID_WICPixelFormat32bppBGRA {
            // l'encodeur a demandé autre chose (souvent BGR 24 bits)
            bgra = img.as_raw().chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
            frame.WritePixels(h, w * 3, &mut bgra).map_err(e)?;
        } else {
            frame.WritePixels(h, w * 4, &mut bgra).map_err(e)?;
        }
        frame.Commit().map_err(e)?;
        enc.Commit().map_err(e)?;
        Ok(())
    }
}

/// Aperçu rapide : Windows décode directement à taille réduite (pour un JPEG, la réduction
/// se fait dans le décodeur : 4 à 8 fois plus rapide qu'un décodage complet).
/// Renvoie l'image réduite et la taille d'origine.
pub fn decode_scaled(path: &str, max_side: u32) -> Result<(image::RgbaImage, (u32, u32)), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let e = |x: windows::core::Error| x.message().to_string();
        let f: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(e)?;
        let dec = f.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).map_err(e)?;
        let frame = dec.GetFrame(0).map_err(e)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).map_err(e)?;
        if w.max(h) <= max_side {
            return Err("déjà petite".into()); // le décodage complet suffit
        }
        let k =(max_side as f64 / w.max(h) as f64).min(1.0);
        let (nw, nh) = (((w as f64 * k) as u32).max(1), ((h as f64 * k) as u32).max(1));
        let sc = f.CreateBitmapScaler().map_err(e)?;
        sc.Initialize(&frame, nw, nh, WICBitmapInterpolationModeFant).map_err(e)?;
        let conv = f.CreateFormatConverter().map_err(e)?;
        conv.Initialize(&sc, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).map_err(e)?;
        let mut buf = vec![0u8; (nw * nh * 4) as usize];
        conv.CopyPixels(std::ptr::null(), nw * 4, &mut buf).map_err(e)?;
        Ok((image::RgbaImage::from_raw(nw, nh, buf).ok_or("image vide")?, (w, h)))
    }
}
