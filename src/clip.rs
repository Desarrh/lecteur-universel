//! Presse-papiers : image au format DIB (collable dans Discord, Word, Paint…).

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_DIB;

pub fn copy_image(rgba: &[u8], w: u32, h: u32) -> bool {
    let header = 40usize;
    let size = header + (w * h * 4) as usize;
    unsafe {
        let Ok(hg): windows::core::Result<HGLOBAL> = GlobalAlloc(GMEM_MOVEABLE, size) else { return false };
        let ptr = GlobalLock(hg) as *mut u8;
        if ptr.is_null() {
            return false;
        }
        let buf = std::slice::from_raw_parts_mut(ptr, size);
        buf[..header].fill(0);
        buf[0..4].copy_from_slice(&(header as u32).to_le_bytes());
        buf[4..8].copy_from_slice(&(w as i32).to_le_bytes());
        buf[8..12].copy_from_slice(&(h as i32).to_le_bytes()); // positif = de bas en haut
        buf[12..14].copy_from_slice(&1u16.to_le_bytes());
        buf[14..16].copy_from_slice(&32u16.to_le_bytes());
        buf[20..24].copy_from_slice(&((w * h * 4) as u32).to_le_bytes());
        let row = (w * 4) as usize;
        for y in 0..h as usize {
            let src = &rgba[(h as usize - 1 - y) * row..(h as usize - y) * row];
            let dst = &mut buf[header + y * row..header + (y + 1) * row];
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
        }
        let _ = GlobalUnlock(hg);
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = SetClipboardData(CF_DIB.0 as u32, Some(HANDLE(hg.0))).is_ok();
        let _ = CloseClipboard();
        ok
    }
}
