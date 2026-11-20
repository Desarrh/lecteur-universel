//! Audio / vidéo via Media Foundation (intégré à Windows : 0 Mo embarqué).
//! Lecture : IMFMediaEngine en mode « frame server ».
//! Vidéo : décodage par la carte graphique (DXVA, via Direct3D 11) quand c'est
//! possible — indispensable pour du 5K 60 i/s — puis l'image, mise à l'échelle par
//! le GPU, est relue en mémoire et copiée dans la fenêtre. Sans carte graphique
//! utilisable : décodage logiciel + bitmap WIC (repli).

use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::*;
use windows::Win32::System::Variant::VT_I8;
use windows::core::*;

thread_local! {
    /// Mesure (LECTEUR_FPS) : temps cumulés transfert GPU, relecture, nombre d'images.
    pub static PROF: std::cell::RefCell<(f64, f64, u32)> = const { std::cell::RefCell::new((0.0, 0.0, 0)) };
}

pub fn init_thread() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    }
}

pub fn init_main() {
    unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    }
}

#[implement(IMFMediaEngineNotify)]
struct Notify {
    q: Arc<Mutex<Vec<u32>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _p1: usize, _p2: u32) -> Result<()> {
        if let Ok(mut q) = self.q.lock() {
            q.push(event);
        }
        (self.wake)();
        Ok(())
    }
}

/// Décodage matériel : périphérique D3D11 partagé avec Media Foundation.
struct Gpu {
    dev: ID3D11Device,
    ctx: ID3D11DeviceContext,
    _mgr: IMFDXGIDeviceManager,
    /// cible de rendu + 2 textures de relecture (double tampon : pas d'attente du GPU)
    rt: Option<(ID3D11Texture2D, [ID3D11Texture2D; 2], u32, u32)>,
    k: usize,
    primed: bool,
}

impl Gpu {
    fn new() -> Result<Gpu> {
        unsafe {
            let mut dev = None;
            let mut ctx = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            )?;
            let dev = dev.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
            let ctx = ctx.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
            if let Ok(mt) = dev.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            let mut token = 0u32;
            let mut mgr = None;
            MFCreateDXGIDeviceManager(&mut token, &mut mgr)?;
            let mgr = mgr.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
            mgr.ResetDevice(&dev, token)?;
            Ok(Gpu { dev, ctx, _mgr: mgr, rt: None, k: 0, primed: false })
        }
    }

    fn tex(&self, w: u32, h: u32, staging: bool) -> Option<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
            BindFlags: if staging { 0 } else { (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32 },
            CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
            MiscFlags: 0,
        };
        let mut t = None;
        unsafe { self.dev.CreateTexture2D(&desc, None, Some(&mut t)).ok()? };
        t
    }

    fn read(&self, st: &ID3D11Texture2D, w: u32, h: u32) -> Option<Vec<u32>> {
        unsafe {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(st, 0, D3D11_MAP_READ, 0, Some(&mut m)).ok()?;
            // BGRA = format du tampon d'écran (l'octet alpha est ignoré par GDI) :
            // simple copie ligne par ligne, aucune conversion pixel par pixel.
            let pitch = m.RowPitch as usize;
            let (wu, hu) = (w as usize, h as usize);
            let mut out: Vec<u32> = Vec::with_capacity(wu * hu);
            let src = m.pData as *const u8;
            for y in 0..hu {
                std::ptr::copy_nonoverlapping(src.add(y * pitch), (out.as_mut_ptr() as *mut u8).add(y * wu * 4), wu * 4);
            }
            out.set_len(wu * hu);
            self.ctx.Unmap(st, 0);
            Some(out)
        }
    }
}

pub struct Player {
    engine: IMFMediaEngine,
    gpu: Option<Gpu>,
    /// Vrai : le moteur affiche lui-même la vidéo dans une sous-fenêtre (GPU direct).
    pub hw_render: bool,
    ex: Option<IMFMediaEngineEx>,
    events: Arc<Mutex<Vec<u32>>>,
    wic: Option<IWICImagingFactory>,
    bmp: Option<(IWICBitmap, u32, u32)>,
    last_pts: i64,
    pub ready: bool,
    pub has_video: bool,
    pub size: (u32, u32),
    pub duration: f64,
    pub error: Option<String>,
    pub ended: bool,
    pub rate: f64,
    pub volume: f64,
    pending_seek: Option<f64>,
}

impl Player {
    pub fn open(path: &str, audio_only: bool, wake: Arc<dyn Fn() + Send + Sync>, hwnd: Option<isize>) -> Result<Player> {
        unsafe {
            let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let mut attr = None;
            MFCreateAttributes(&mut attr, 3)?;
            let attr = attr.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?;
            let events = Arc::new(Mutex::new(Vec::new()));
            let notify: IMFMediaEngineNotify = Notify { q: events.clone(), wake }.into();
            attr.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attr.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
            // 1) Affichage direct par le moteur dans une sous-fenêtre (le plus fluide).
            // 2) Sinon décodage matériel + relecture. 3) Sinon logiciel.
            // LECTEUR_GPU=0 force le logiciel, LECTEUR_GPU=relecture force le mode 2.
            let mode = std::env::var("LECTEUR_GPU").unwrap_or_default();
            let hw_render = !audio_only && hwnd.is_some() && mode != "0" && mode != "relecture";
            if hw_render {
                attr.SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd.unwrap_or(0) as u64)?;
            }
            let gpu = if audio_only || hw_render || mode == "0" { None } else { Gpu::new().ok() };
            if let Some(g) = &gpu {
                attr.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &g._mgr)?;
            }
            let flags = if audio_only { MF_MEDIA_ENGINE_AUDIOONLY.0 as u32 } else { 0 };
            let engine = factory.CreateInstance(flags, &attr)?;
            let ex = engine.cast::<IMFMediaEngineEx>().ok();
            engine.SetSource(&BSTR::from(path))?;
            let wic: Option<IWICImagingFactory> = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok();
            Ok(Player {
                engine,
                gpu,
                hw_render,
                ex,
                events,
                wic,
                bmp: None,
                last_pts: -1,
                ready: false,
                has_video: false,
                size: (0, 0),
                duration: 0.0,
                error: None,
                ended: false,
                rate: 1.0,
                volume: 1.0,
                pending_seek: None,
            })
        }
    }

    /// Traite les événements du moteur. Renvoie vrai si quelque chose a changé.
    pub fn poll(&mut self) -> bool {
        let evs: Vec<u32> = self.events.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for e in &evs {
            let e = *e as i32;
            if e == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 || e == MF_MEDIA_ENGINE_EVENT_DURATIONCHANGE.0 || e == MF_MEDIA_ENGINE_EVENT_FORMATCHANGE.0 {
                unsafe {
                    let d = self.engine.GetDuration();
                    if d.is_finite() {
                        self.duration = d;
                    }
                    self.has_video = self.engine.HasVideo().as_bool();
                    if self.has_video {
                        let (mut w, mut h) = (0u32, 0u32);
                        let _ = self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
                        self.size = (w, h);
                    }
                }
                self.ready = true;
                if let Some(t) = self.pending_seek.take() {
                    self.seek(t);
                }
            } else if e == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
                self.error = Some(unsafe {
                    match self.engine.GetError() {
                        Ok(err) => format!("Lecture impossible (code {}). Codec manquant ?", err.GetErrorCode()),
                        Err(_) => "Lecture impossible".into(),
                    }
                });
            } else if e == MF_MEDIA_ENGINE_EVENT_ENDED.0 {
                self.ended = true;
            } else if e == MF_MEDIA_ENGINE_EVENT_PLAYING.0 {
                self.ended = false;
            }
        }
        !evs.is_empty()
    }

    #[cfg(feature = "dev")]
    /// Durée d'un fichier (s) via le Source Reader, sans lecture.
    pub fn probe_duration(path: &str) -> f64 {
        reader(path, false).map(|r| duration_of(&r)).unwrap_or(0.0)
    }

    pub fn playing(&self) -> bool {
        unsafe { !self.engine.IsPaused().as_bool() && !self.ended }
    }
    pub fn play(&mut self) {
        if self.ended {
            self.seek(0.0);
            self.ended = false;
        }
        unsafe {
            let _ = self.engine.Play();
        }
    }
    pub fn pause(&mut self) {
        unsafe {
            let _ = self.engine.Pause();
        }
    }
    pub fn toggle(&mut self) {
        if self.playing() { self.pause() } else { self.play() }
    }
    pub fn time(&self) -> f64 {
        unsafe { self.engine.GetCurrentTime() }
    }
    pub fn seek(&mut self, t: f64) {
        if !self.ready {
            self.pending_seek = Some(t);
            return;
        }
        let t = t.clamp(0.0, self.duration.max(0.0));
        unsafe {
            let _ = self.engine.SetCurrentTime(t);
        }
        self.ended = false;
        self.last_pts = -1;
    }
    pub fn set_rate(&mut self, r: f64) {
        self.rate = r;
        unsafe {
            let _ = self.engine.SetDefaultPlaybackRate(r);
            let _ = self.engine.SetPlaybackRate(r);
        }
    }
    pub fn set_volume(&mut self, v: f64) {
        self.volume = v.clamp(0.0, 1.0);
        unsafe {
            let _ = self.engine.SetVolume(self.volume);
        }
    }
    pub fn step(&mut self, forward: bool) {
        self.pause();
        if let Some(ex) = &self.ex {
            unsafe {
                let _ = ex.FrameStep(forward);
            }
        }
        self.last_pts = -1;
    }

    fn ensure_bmp(&mut self, w: u32, h: u32) -> Option<IWICBitmap> {
        if let Some((b, bw, bh)) = &self.bmp {
            if *bw == w && *bh == h {
                return Some(b.clone());
            }
        }
        let wic = self.wic.as_ref()?;
        let b = unsafe { wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppBGRA, WICBitmapCacheOnLoad).ok()? };
        self.bmp = Some((b.clone(), w, h));
        Some(b)
    }

    fn grab(&mut self, w: u32, h: u32) -> Option<egui::ColorImage> {
        let bmp = self.ensure_bmp(w, h)?;
        unsafe {
            let dst = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            let r = self.engine.TransferVideoFrame(&bmp, None, &dst, None);
            let img = bmp_to_image(&bmp, w, h);
            r.ok()?;
            img
        }
    }

    /// Image à la taille exacte d'affichage, au format du tampon d'écran
    /// (u32 0x00RRGGBB) : copiée telle quelle dans la fenêtre, sans passer par egui.
    pub fn frame_px(&mut self, w: u32, h: u32, force: bool) -> Option<(u32, u32, Vec<u32>)> {
        if self.hw_render || !self.has_video || self.size.0 == 0 || w < 2 || h < 2 {
            return None;
        }
        let pts = unsafe { self.engine.OnVideoStreamTick().ok()? };
        if pts == self.last_pts && !force {
            return None;
        }
        self.last_pts = pts;
        if self.gpu.is_some() {
            let sync = force || !self.playing();
            let g = self.gpu.as_mut()?;
            if g.rt.as_ref().map(|r| (r.2, r.3)) != Some((w, h)) {
                let rt = g.tex(w, h, false)?;
                let s0 = g.tex(w, h, true)?;
                let s1 = g.tex(w, h, true)?;
                g.rt = Some((rt, [s0, s1], w, h));
                g.primed = false;
            }
            let (rt, st, _, _) = g.rt.as_ref()?;
            let t0 = std::time::Instant::now();
            unsafe {
                let dst = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                self.engine.TransferVideoFrame(rt, None, &dst, None).ok()?;
                g.ctx.CopyResource(&st[g.k], rt);
            }
            let t1 = std::time::Instant::now();
            // En lecture : on relit l'image précédente (déjà prête) pour ne jamais attendre le GPU.
            let idx = if sync || !g.primed { g.k } else { 1 - g.k };
            let px = g.read(&st[idx], w, h)?;
            PROF.with(|pr| {
                let mut pr = pr.borrow_mut();
                pr.0 += (t1 - t0).as_secs_f64();
                pr.1 += t1.elapsed().as_secs_f64();
                pr.2 += 1;
            });
            g.k = 1 - g.k;
            g.primed = true;
            return Some((w, h, px));
        }
        let bmp = self.ensure_bmp(w, h)?;
        unsafe {
            let dst = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            self.engine.TransferVideoFrame(&bmp, None, &dst, None).ok()?;
            let rect = WICRect { X: 0, Y: 0, Width: w as i32, Height: h as i32 };
            let lock = bmp.Lock(&rect, WICBitmapLockRead.0 as u32).ok()?;
            let stride = lock.GetStride().ok()? as usize;
            let mut size = 0u32;
            let mut ptr = std::ptr::null_mut();
            lock.GetDataPointer(&mut size, &mut ptr).ok()?;
            let data = std::slice::from_raw_parts(ptr, size as usize);
            let mut out = Vec::with_capacity((w * h) as usize);
            for y in 0..h as usize {
                let row = &data[y * stride..y * stride + w as usize * 4];
                out.extend(row.chunks_exact(4).map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0])));
            }
            Some((w, h, out))
        }
    }

    /// Statistiques du moteur : (images affichées, images abandonnées).
    pub fn stats(&self) -> (u64, u64) {
        let get = |id| unsafe {
            self.ex.as_ref().and_then(|e| e.GetStatistics(id).ok()).map(|v| {
                let inner = &v.Anonymous.Anonymous;
                match inner.vt.0 {
                    19 | 3 => inner.Anonymous.ulVal as u64,
                    _ => inner.Anonymous.uhVal,
                }
            })
        };
        (get(MF_MEDIA_ENGINE_STATISTIC_FRAMES_RENDERED).unwrap_or(0), get(MF_MEDIA_ENGINE_STATISTIC_FRAMES_DROPPED).unwrap_or(0))
    }

    /// Mode affichage direct : la sortie vidéo occupe toute la sous-fenêtre (w × h).
    pub fn set_output(&self, w: i32, h: i32) {
        if let Some(ex) = &self.ex {
            let dst = RECT { left: 0, top: 0, right: w, bottom: h };
            let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
            unsafe {
                let _ = ex.UpdateVideoStream(None, Some(&dst), Some(&black));
            }
        }
    }

    /// Image en pleine définition (capture).
    pub fn capture(&mut self) -> Option<egui::ColorImage> {
        let (w, h) = self.size;
        if w == 0 || self.hw_render {
            return None;
        }
        if let Some(g) = &self.gpu {
            let rt = g.tex(w, h, false)?;
            let st = g.tex(w, h, true)?;
            unsafe {
                let dst = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
                self.engine.TransferVideoFrame(&rt, None, &dst, None).ok()?;
                g.ctx.CopyResource(&st, &rt);
            }
            let px = g.read(&st, w, h)?;
            let rgba: Vec<u8> = px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
            self.last_pts = -1;
            return Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba));
        }
        let img = self.grab(w, h);
        self.bmp = None;
        self.last_pts = -1;
        img
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}

unsafe fn bmp_to_image(bmp: &IWICBitmap, w: u32, h: u32) -> Option<egui::ColorImage> {
    unsafe {
        let rect = WICRect { X: 0, Y: 0, Width: w as i32, Height: h as i32 };
        let lock = bmp.Lock(&rect, WICBitmapLockRead.0 as u32).ok()?;
        let stride = lock.GetStride().ok()? as usize;
        let mut size = 0u32;
        let mut ptr = std::ptr::null_mut();
        lock.GetDataPointer(&mut size, &mut ptr).ok()?;
        let data = std::slice::from_raw_parts(ptr, size as usize);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let row = &data[y * stride..y * stride + w as usize * 4];
            for px in row.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba))
    }
}

// ---------------------------------------------------------------------------
// Décodage (Source Reader) : forme d'onde, conversions, GIF

pub struct Pcm {
    pub samples: Vec<i16>,
    pub channels: u16,
    pub rate: u32,
}

pub(crate) fn reader(path: &str, video_rgb: bool) -> Result<IMFSourceReader> {
    unsafe {
        let mut attr = None;
        MFCreateAttributes(&mut attr, 1)?;
        let attr = attr.unwrap();
        if video_rgb {
            attr.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        }
        MFCreateSourceReaderFromURL(&HSTRING::from(path), &attr)
    }
}

pub(crate) fn pv_i64(v: i64) -> PROPVARIANT {
    let mut p = PROPVARIANT::default();
    unsafe {
        let inner = &mut *p.Anonymous.Anonymous;
        inner.vt = VT_I8;
        inner.Anonymous.hVal = v;
    }
    p
}

pub fn duration_of(r: &IMFSourceReader) -> f64 {
    unsafe {
        match r.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION) {
            Ok(p) => p.Anonymous.Anonymous.Anonymous.uhVal as f64 / 1e7,
            Err(_) => 0.0,
        }
    }
}

/// Décode la première piste audio en PCM 16 bits. `progress` reçoit 0..1.
pub fn decode_pcm(path: &str, range: Option<(f64, f64)>, progress: &dyn Fn(f32)) -> Result<Pcm> {
    unsafe {
        let (r, a, channels, rate) = pcm_reader(path)?;
        let channels = channels as u16;
        let dur = duration_of(&r);
        let (t0, t1) = range.unwrap_or((0.0, f64::MAX));
        if t0 > 0.0 {
            r.SetCurrentPosition(&GUID::zeroed(), &pv_i64((t0 * 1e7) as i64))?;
        }
        let mut out: Vec<i16> = Vec::new();
        let mut first_ts: Option<f64> = None;
        loop {
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            r.ReadSample(a, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(s) = sample else { continue };
            let t = ts as f64 / 1e7;
            if t >= t1 {
                break;
            }
            let buf = s.ConvertToContiguousBuffer()?;
            let mut p = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut p, None, Some(&mut len))?;
            let sl = std::slice::from_raw_parts(p as *const i16, len as usize / 2);
            // Coupe précise au début de la plage (le seek s'arrête sur une trame).
            let mut start = 0usize;
            if range.is_some() && first_ts.is_none() && t < t0 {
                start = (((t0 - t) * rate as f64) as usize * channels as usize).min(sl.len());
            }
            first_ts.get_or_insert(t);
            out.extend_from_slice(&sl[start..]);
            buf.Unlock()?;
            if dur > 0.0 {
                progress((t / dur.min(t1)) as f32);
            }
        }
        if range.is_some() {
            let max = ((t1 - t0) * rate as f64) as usize * channels as usize;
            out.truncate(max);
        }
        Ok(Pcm { samples: out, channels, rate })
    }
}

/// Niveau sonore (dBFS) par tranche de `hop` secondes, lu en flux sans garder le son
/// (une heure de vidéo = 72 000 valeurs). `cancel` interrompt la lecture.
pub fn loudness(path: &str, hop: f64, cancel: &socle::Cancel) -> Result<Vec<f32>> {
    let (r, a, channels, rate) = pcm_reader(path)?;
    let win = ((rate as f64 * hop) as usize).max(1) * channels as usize;
    let (mut acc, mut n) = (0f64, 0usize);
    let mut out = Vec::new();
    let db = |ms: f64| (10.0 * ms.max(1e-9).log10()) as f32;
    pcm_stream(&r, a, cancel, &mut |sl, _| {
        for &x in sl {
            let v = x as f64 / 32768.0;
            acc += v * v;
            n += 1;
            if n == win {
                out.push(db(acc / n as f64));
                acc = 0.0;
                n = 0;
            }
        }
        true
    })?;
    if n > 0 {
        out.push(db(acc / n as f64));
    }
    Ok(out)
}

/// Lecteur de la première piste audio en PCM 16 bits : (lecteur, piste, canaux, fréquence).
pub(crate) fn pcm_reader(path: &str) -> Result<(IMFSourceReader, u32, u32, u32)> {
    unsafe {
        let r = reader(path, false)?;
        let a = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        r.SetStreamSelection(a, true)?;
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        r.SetCurrentMediaType(a, None, &mt)?;
        let cur = r.GetCurrentMediaType(a)?;
        let channels = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?.max(1);
        let rate = cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?.max(1);
        Ok((r, a, channels, rate))
    }
}

/// Lit le son en flux, échantillon par échantillon (bloc PCM, instant du bloc en s), sans le
/// garder en mémoire. S'arrête si `f` renvoie faux ou si `cancel` est annulé.
pub(crate) fn pcm_stream(r: &IMFSourceReader, a: u32, cancel: &socle::Cancel, f: &mut dyn FnMut(&[i16], f64) -> bool) -> Result<()> {
    unsafe {
        loop {
            if cancel.is_cancelled() {
                return Err(Error::from(windows::Win32::Foundation::E_ABORT));
            }
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            r.ReadSample(a, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(());
            }
            let Some(s) = sample else { continue };
            let buf = s.ConvertToContiguousBuffer()?;
            let mut p = std::ptr::null_mut();
            let mut len = 0u32;
            buf.Lock(&mut p, None, Some(&mut len))?;
            let sl = std::slice::from_raw_parts(p as *const i16, len as usize / 2);
            let go = f(sl, ts as f64 / 1e7);
            buf.Unlock()?;
            if !go {
                return Ok(());
            }
        }
    }
}

/// Forme d'onde (une crête 0..1 par tranche) lue en flux : la mémoire ne dépend pas de la durée.
pub fn peaks_stream(path: &str, buckets: usize, cancel: &socle::Cancel) -> Result<Vec<f32>> {
    let (r, a, channels, rate) = pcm_reader(path)?;
    let dur = duration_of(&r).max(0.001);
    let mut out = vec![0f32; buckets.max(1)];
    let ch = channels as usize;
    pcm_stream(&r, a, cancel, &mut |sl, t0| {
        for (k, f) in sl.chunks(ch).enumerate() {
            let t = t0 + k as f64 / rate as f64;
            let n = out.len();
            let b = (((t / dur) * n as f64) as usize).min(n - 1);
            if let Some(slot) = out.get_mut(b) {
                let v = f.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0;
                if v > *slot {
                    *slot = v;
                }
            }
        }
        true
    })?;
    Ok(out)
}

/// Son → MP3 en flux (sans tout décoder en mémoire) quand le format s'y prête (32, 44,1 ou
/// 48 kHz, 1 ou 2 canaux) ; `Ok(false)` sinon (l'appelant passe par le décodage complet).
pub fn write_mp3_stream(path: &str, range: Option<(f64, f64)>, out: &str, kbps: u32, progress: &dyn Fn(f32)) -> Result<bool> {
    let (r, a, channels, rate) = pcm_reader(path)?;
    if ![32000, 44100, 48000].contains(&rate) || channels > 2 {
        return Ok(false);
    }
    let dur = duration_of(&r);
    let (t0, t1) = range.unwrap_or((0.0, f64::MAX));
    unsafe {
        if t0 > 0.0 {
            r.SetCurrentPosition(&GUID::zeroed(), &pv_i64((t0 * 1e7) as i64))?;
        }
    }
    let (w, idx) = mp3_writer(out, rate, channels, kbps)?;
    let ch = channels as usize;
    let mut t: i64 = 0;
    let mut err = None;
    let res = pcm_stream(&r, a, &socle::Cancel::new(), &mut |sl, ts| {
        let frames = sl.len() / ch;
        let end_ts = ts + frames as f64 / rate as f64;
        if ts >= t1 {
            return false;
        }
        // coupe précise au début et à la fin de la plage
        let skip = if ts < t0 { (((t0 - ts) * rate as f64) as usize).min(frames) } else { 0 };
        let keep = if end_ts > t1 { (((t1 - ts) * rate as f64) as usize).min(frames) } else { frames };
        if keep > skip {
            if let Err(e) = mp3_chunk(&w, idx, &sl[skip * ch..keep * ch], ch, rate, &mut t) {
                err = Some(e);
                return false;
            }
        }
        if dur > 0.0 {
            progress(((ts - t0.min(ts)) / (dur.min(t1) - t0).max(0.001)) as f32);
        }
        end_ts < t1
    });
    if let Some(e) = err {
        let _ = std::fs::remove_file(out);
        return Err(e);
    }
    res.and_then(|_| unsafe { w.Finalize() }).map(|_| true).inspect_err(|_| {
        let _ = std::fs::remove_file(out);
    })
}

fn mp3_writer(out: &str, rate: u32, channels: u32, kbps: u32) -> Result<(IMFSinkWriter, u32)> {
    unsafe {
        let w = MFCreateSinkWriterFromURL(&HSTRING::from(out), None, None)?;
        let ot = MFCreateMediaType()?;
        ot.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        ot.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_MP3)?;
        ot.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        ot.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        ot.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, kbps * 1000 / 8)?;
        let idx = w.AddStream(&ot)?;
        let it = MFCreateMediaType()?;
        it.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        it.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
        it.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
        it.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
        it.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, channels)?;
        it.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * 2)?;
        it.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * channels * 2)?;
        w.SetInputMediaType(idx, &it, None)?;
        w.BeginWriting()?;
        Ok((w, idx))
    }
}

fn mp3_chunk(w: &IMFSinkWriter, idx: u32, part: &[i16], ch: usize, rate: u32, t: &mut i64) -> Result<()> {
    unsafe {
        let bytes = part.len() * 2;
        let buf = MFCreateMemoryBuffer(bytes as u32)?;
        let mut ptr = std::ptr::null_mut();
        buf.Lock(&mut ptr, None, None)?;
        std::ptr::copy_nonoverlapping(part.as_ptr() as *const u8, ptr, bytes);
        buf.Unlock()?;
        buf.SetCurrentLength(bytes as u32)?;
        let s = MFCreateSample()?;
        s.AddBuffer(&buf)?;
        let d = (part.len() / ch) as i64 * 10_000_000 / rate as i64;
        s.SetSampleTime(*t)?;
        s.SetSampleDuration(d)?;
        *t += d;
        w.WriteSample(idx, &s)
    }
}

pub fn write_wav(p: &Pcm, out: &str) -> std::result::Result<(), String> {
    let spec = hound::WavSpec { channels: p.channels, sample_rate: p.rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(out, spec).map_err(|e| e.to_string())?;
    for s in &p.samples {
        w.write_sample(*s).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

/// Rééchantillonnage linéaire (suffisant pour passer de 22/11 kHz à 44,1 kHz).
pub fn resample(p: &Pcm, rate: u32) -> Pcm {
    if p.rate == rate || p.samples.is_empty() {
        return Pcm { samples: p.samples.clone(), channels: p.channels, rate: p.rate };
    }
    let ch = p.channels.max(1) as usize;
    let frames = p.samples.len() / ch;
    let out_frames = (frames as u64 * rate as u64 / p.rate as u64) as usize;
    let mut out = Vec::with_capacity(out_frames * ch);
    let ratio = p.rate as f64 / rate as f64;
    for i in 0..out_frames {
        let x = i as f64 * ratio;
        let i0 = (x as usize).min(frames - 1);
        let i1 = (i0 + 1).min(frames - 1);
        let f = x - i0 as f64;
        for c in 0..ch {
            let a = p.samples[i0 * ch + c] as f64;
            let b = p.samples[i1 * ch + c] as f64;
            out.push((a + (b - a) * f) as i16);
        }
    }
    Pcm { samples: out, channels: p.channels, rate }
}

/// Encodage MP3 par l'encodeur intégré à Windows (MF « MP3 Encoder »).
/// Il n'accepte que 32 / 44,1 / 48 kHz et 1 ou 2 canaux : on adapte avant.
pub fn write_mp3(p: &Pcm, out: &str, kbps: u32, progress: &dyn Fn(f32)) -> Result<()> {
    let fixed;
    let p = if ![32000, 44100, 48000].contains(&p.rate) || p.channels > 2 {
        let mut q = resample(p, if p.rate > 44100 { 48000 } else { 44100 });
        if q.channels > 2 {
            let ch = q.channels as usize;
            q.samples = q.samples.chunks(ch).flat_map(|f| [f[0], f[1]]).collect();
            q.channels = 2;
        }
        fixed = q;
        &fixed
    } else {
        p
    };
    let r = write_mp3_inner(p, out, kbps, progress);
    if r.is_err() {
        let _ = std::fs::remove_file(out);
    }
    r
}

fn write_mp3_inner(p: &Pcm, out: &str, kbps: u32, progress: &dyn Fn(f32)) -> Result<()> {
    let (w, idx) = mp3_writer(out, p.rate, p.channels as u32, kbps)?;
    let ch = p.channels as usize;
    let chunk = p.rate as usize * ch; // 1 s
    let mut t: i64 = 0;
    let total = p.samples.len().max(1);
    for (i, part) in p.samples.chunks(chunk).enumerate() {
        mp3_chunk(&w, idx, part, ch, p.rate, &mut t)?;
        progress((i * chunk) as f32 / total as f32);
    }
    unsafe { w.Finalize() }
}

/// Images RGBA d'une vidéo entre t0 et t1, à `fps`, largeur `width`.
pub fn video_frames(path: &str, t0: f64, t1: f64, fps: f64, width: u32, progress: &dyn Fn(f32)) -> Result<Vec<image::RgbaImage>> {
    unsafe {
        let r = reader(path, true)?;
        let v = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        r.SetStreamSelection(v, true)?;
        let mt = MFCreateMediaType()?;
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        r.SetCurrentMediaType(v, None, &mt)?;
        let cur = r.GetCurrentMediaType(v)?;
        let fs = cur.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (fw, fh) = ((fs >> 32) as u32, (fs & 0xffff_ffff) as u32);
        if t0 > 0.0 {
            r.SetCurrentPosition(&GUID::zeroed(), &pv_i64((t0 * 1e7) as i64))?;
        }
        let mut next = t0;
        let mut frames = Vec::new();
        // `width` = plus grand côté voulu (0 = taille native)
        let sc = if width == 0 { 1.0 } else { (width as f64 / fw.max(fh) as f64).min(1.0) };
        let width = ((fw as f64 * sc) as u32).max(2);
        let h = ((fh as f64 * sc) as u32).max(2);
        loop {
            let mut flags = 0u32;
            let mut ts = 0i64;
            let mut sample = None;
            r.ReadSample(v, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))?;
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(s) = sample else { continue };
            let t = ts as f64 / 1e7;
            if t >= t1 {
                break;
            }
            if t + 1e-3 < next {
                continue;
            }
            next += 1.0 / fps;
            let buf = s.ConvertToContiguousBuffer()?;
            let mut img = image::RgbaImage::new(fw, fh);
            // Le vrai pas de ligne (souvent aligné sur 16/64 octets) vient du tampon 2D.
            if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                let mut p0 = std::ptr::null_mut();
                let mut pitch = 0i32;
                b2.Lock2D(&mut p0, &mut pitch)?;
                for y in 0..fh as isize {
                    let row = std::slice::from_raw_parts(p0.offset(y * pitch as isize), fw as usize * 4);
                    for (x, px) in row.chunks_exact(4).enumerate() {
                        img.put_pixel(x as u32, y as u32, image::Rgba([px[2], px[1], px[0], 255]));
                    }
                }
                b2.Unlock2D()?;
            } else {
                let mut p = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut p, None, Some(&mut len))?;
                let data = std::slice::from_raw_parts(p, len as usize);
                let stride = (len as usize / fh.max(1) as usize).max(fw as usize * 4);
                for y in 0..fh as usize {
                    let Some(row) = data.get(y * stride..y * stride + fw as usize * 4) else { break };
                    for (x, px) in row.chunks_exact(4).enumerate() {
                        img.put_pixel(x as u32, y as u32, image::Rgba([px[2], px[1], px[0], 255]));
                    }
                }
                buf.Unlock()?;
            }
            frames.push(image::imageops::resize(&img, width, h, image::imageops::FilterType::Triangle));
            progress(((t - t0) / (t1 - t0)).clamp(0.0, 1.0) as f32);
        }
        Ok(frames)
    }
}

// ---------------------------------------------------------------------------
// Coupe MP3 sans réencodage : on recopie les trames entières de la plage.

fn mp3_frame_len(h: &[u8]) -> Option<(usize, f64)> {
    if h.len() < 4 || h[0] != 0xFF || h[1] & 0xE0 != 0xE0 {
        return None;
    }
    let ver = (h[1] >> 3) & 3; // 3 = MPEG1, 2 = MPEG2, 0 = MPEG2.5
    let layer = (h[1] >> 1) & 3; // 1 = Layer III
    if layer != 1 || ver == 1 {
        return None;
    }
    let br_i = (h[2] >> 4) as usize;
    let sr_i = ((h[2] >> 2) & 3) as usize;
    if br_i == 0 || br_i == 15 || sr_i == 3 {
        return None;
    }
    let pad = ((h[2] >> 1) & 1) as usize;
    const BR1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const BR2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    const SR: [u32; 3] = [44100, 48000, 32000];
    let sr = SR[sr_i] / match ver { 3 => 1, 2 => 2, _ => 4 };
    let (br, spf) = if ver == 3 { (BR1[br_i] * 1000, 1152) } else { (BR2[br_i] * 1000, 576) };
    let len = (spf / 8 * br / sr) as usize + pad;
    Some((len, spf as f64 / sr as f64))
}

pub fn cut_mp3(src: &str, out: &str, t0: f64, t1: f64) -> std::result::Result<(), String> {
    let data = std::fs::read(src).map_err(|e| e.to_string())?;
    let mut i = 0usize;
    let mut res = Vec::new();
    // Tag ID3v2 conservé
    if data.len() > 10 && &data[0..3] == b"ID3" {
        let sz = ((data[6] as usize & 0x7f) << 21) | ((data[7] as usize & 0x7f) << 14) | ((data[8] as usize & 0x7f) << 7) | (data[9] as usize & 0x7f);
        i = (10 + sz).min(data.len());
        res.extend_from_slice(&data[..i]);
    }
    let mut t = 0.0;
    while i + 4 <= data.len() {
        match mp3_frame_len(&data[i..]) {
            Some((len, d)) if len > 4 && i + len <= data.len() => {
                if t + d > t0 && t < t1 {
                    res.extend_from_slice(&data[i..i + len]);
                }
                t += d;
                i += len;
                if t >= t1 {
                    break;
                }
            }
            _ => i += 1,
        }
    }
    std::fs::write(out, res).map_err(|e| e.to_string())
}
