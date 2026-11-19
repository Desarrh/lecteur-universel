//! PDF via Windows.Data.Pdf (WinRT, intégré à Windows : 0 Mo embarqué).
//! Un thread dédié garde le document ouvert et rend les pages à la demande.

use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::core::{HSTRING, Interface};

pub enum Msg {
    Opened { pages: u32, sizes: Vec<(f32, f32)> },
    Page { index: u32, width: u32, img: image::RgbaImage },
    Error(String),
}

pub struct PdfWorker {
    pub req: Sender<(u32, u32)>,
    pub rx: Receiver<Msg>,
}

pub fn open(path: &str, wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> PdfWorker {
    let (req_tx, req_rx) = channel::<(u32, u32)>();
    let (tx, rx) = channel();
    let path = path.to_string();
    std::thread::spawn(move || {
        crate::media::init_thread();
        let doc = match load(&path) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(Msg::Error(format!("PDF illisible : {e}")));
                wake();
                return;
            }
        };
        let pages = doc.PageCount().unwrap_or(0);
        let sizes = (0..pages)
            .map(|i| doc.GetPage(i).and_then(|p| p.Size()).map(|s| (s.Width, s.Height)).unwrap_or((612.0, 792.0)))
            .collect();
        let _ = tx.send(Msg::Opened { pages, sizes });
        wake();
        while let Ok((index, width)) = req_rx.recv() {
            match render(&doc, index, width) {
                Ok(img) => {
                    let _ = tx.send(Msg::Page { index, width, img });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("page {} : {e}", index + 1)));
                }
            }
            wake();
        }
    });
    PdfWorker { req: req_tx, rx }
}

fn load(path: &str) -> windows::core::Result<PdfDocument> {
    // WinRT exige un chemin absolu avec des « \ ».
    let abs = std::path::absolute(path).map(|p| p.to_string_lossy().replace('/', "\\")).unwrap_or_else(|_| path.replace('/', "\\"));
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(abs))?.join()?;
    PdfDocument::LoadFromFileAsync(&file)?.join()
}

pub fn render(doc: &PdfDocument, index: u32, width: u32) -> Result<image::RgbaImage, String> {
    let r = || -> windows::core::Result<Vec<u8>> {
        let page = doc.GetPage(index)?;
        let stream = InMemoryRandomAccessStream::new()?;
        let opts = PdfPageRenderOptions::new()?;
        opts.SetDestinationWidth(width)?;
        page.RenderWithOptionsToStreamAsync(&stream, &opts)?.join()?;
        let size = stream.Size()? as u32;
        let input = stream.GetInputStreamAt(0)?;
        let reader = DataReader::CreateDataReader(&input)?;
        let op = reader.LoadAsync(size)?;
        op.cast::<windows_future::IAsyncOperation<u32>>()?.join()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf)?;
        Ok(buf)
    };
    let bytes = r().map_err(|e| e.message().to_string())?;
    image::load_from_memory(&bytes).map(|i| i.to_rgba8()).map_err(|e| e.to_string())
}

/// Exporte toutes les pages en PNG (dossier à côté du PDF).
pub fn export_pngs(path: &str, out_dir: &std::path::Path, width: u32, progress: &dyn Fn(f32)) -> Result<usize, String> {
    let doc = load(path).map_err(|e| e.message().to_string())?;
    let n = doc.PageCount().map_err(|e| e.message().to_string())?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    for i in 0..n {
        let img = render(&doc, i, width)?;
        img.save(out_dir.join(format!("page-{:03}.png", i + 1))).map_err(|e| e.to_string())?;
        progress((i + 1) as f32 / n as f32);
    }
    Ok(n as usize)
}
