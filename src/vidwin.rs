//! Sous-fenêtre vidéo : Media Foundation y affiche la vidéo directement avec la
//! carte graphique (aucun passage par la mémoire CPU) — fluide même en 5K 60 i/s.
//! Elle est placée sous l'interface ; la souris la traverse (HTTRANSPARENT) et les
//! éléments d'interface qui la recouvrent (barre du haut, toasts, menus) sont
//! découpés de sa région pour rester visibles.

use std::sync::Once;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    if m == WM_ERASEBKGND {
        return LRESULT(1);
    }
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

static CLASS: Once = Once::new();

pub struct VideoWindow {
    pub hwnd: HWND,
    placed: Option<(i32, i32, i32, i32)>,
    cuts: Vec<(i32, i32, i32, i32)>,
    visible: bool,
}

impl VideoWindow {
    pub fn new(parent: isize) -> Option<VideoWindow> {
        unsafe {
            let inst = GetModuleHandleW(None).ok()?;
            CLASS.call_once(|| {
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(proc),
                    hInstance: inst.into(),
                    lpszClassName: w!("LecteurVideo"),
                    hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            });
            let parent = HWND(parent as *mut _);
            // Le parent ne doit pas repeindre par-dessus la vidéo.
            let st = GetWindowLongPtrW(parent, GWL_STYLE);
            SetWindowLongPtrW(parent, GWL_STYLE, st | WS_CLIPCHILDREN.0 as isize);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("LecteurVideo"),
                None,
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                2,
                2,
                Some(parent),
                None,
                Some(inst.into()),
                None,
            )
            .ok()?;
            Some(VideoWindow { hwnd, placed: None, cuts: Vec::new(), visible: false })
        }
    }

    /// Place la vidéo (pixels, coordonnées client du parent) ; `cuts` = zones d'interface à laisser visibles.
    /// Renvoie vrai si la taille a changé (le moteur doit alors ajuster sa sortie).
    pub fn place(&mut self, r: (i32, i32, i32, i32), cuts: &[(i32, i32, i32, i32)]) -> bool {
        let resized = self.placed.map(|p| (p.2, p.3)) != Some((r.2, r.3));
        unsafe {
            if self.placed != Some(r) || !self.visible {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOP), r.0, r.1, r.2.max(2), r.3.max(2), SWP_NOACTIVATE | SWP_SHOWWINDOW);
                self.placed = Some(r);
                self.visible = true;
            }
            let local: Vec<(i32, i32, i32, i32)> = cuts
                .iter()
                .map(|c| (c.0 - r.0, c.1 - r.1, c.2, c.3))
                .filter(|c| c.0 < r.2 && c.1 < r.3 && c.0 + c.2 > 0 && c.1 + c.3 > 0)
                .collect();
            if local != self.cuts {
                if local.is_empty() {
                    SetWindowRgn(self.hwnd, None, true);
                } else {
                    let rgn = CreateRectRgn(0, 0, r.2, r.3);
                    for c in &local {
                        let o = CreateRectRgn(c.0, c.1, c.0 + c.2, c.1 + c.3);
                        CombineRgn(Some(rgn), Some(rgn), Some(o), RGN_DIFF);
                        let _ = DeleteObject(o.into());
                    }
                    SetWindowRgn(self.hwnd, Some(rgn), true);
                }
                self.cuts = local;
            }
        }
        resized
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.placed.map(|p| (p.2, p.3)).unwrap_or((0, 0))
    }

    pub fn client_rect(&self) -> RECT {
        let (w, h) = self.size();
        RECT { left: 0, top: 0, right: w, bottom: h }
    }
}

impl Drop for VideoWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
