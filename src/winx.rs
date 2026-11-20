//! Intégration Windows : boîte « Ouvrir », inscription dans « Ouvrir avec ».
//! (L'instance unique est dans le socle : `socle::instance`.)

use std::path::PathBuf;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::core::*;

/// Boîte de dialogue « Ouvrir » native (sélection multiple).
pub fn open_dialog() -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        let Ok(d): Result<IFileOpenDialog> = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) else { return out };
        let _ = d.SetOptions(FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST);
        let n1 = HSTRING::from("Fichiers pris en charge");
        let p1 = HSTRING::from(crate::kind::dialog_pattern());
        let n2 = HSTRING::from("Tous les fichiers");
        let p2 = HSTRING::from("*.*");
        let specs = [
            COMDLG_FILTERSPEC { pszName: PCWSTR(n1.as_ptr()), pszSpec: PCWSTR(p1.as_ptr()) },
            COMDLG_FILTERSPEC { pszName: PCWSTR(n2.as_ptr()), pszSpec: PCWSTR(p2.as_ptr()) },
        ];
        let _ = d.SetFileTypes(&specs);
        if d.Show(None).is_err() {
            return out;
        }
        if let Ok(items) = d.GetResults() {
            let n = items.GetCount().unwrap_or(0);
            for i in 0..n {
                if let Ok(it) = items.GetItemAt(i) {
                    if let Ok(p) = it.GetDisplayName(SIGDN_FILESYSPATH) {
                        out.push(PathBuf::from(p.to_string().unwrap_or_default()));
                        CoTaskMemFree(Some(p.0 as _));
                    }
                }
            }
        }
    }
    out
}

fn set_str(root: HKEY, sub: &str, name: Option<&str>, val: &str) -> bool {
    unsafe {
        let mut k = HKEY::default();
        if RegCreateKeyExW(root, &HSTRING::from(sub), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut k, None).is_err() {
            return false;
        }
        let w: Vec<u16> = val.encode_utf16().chain(Some(0)).collect();
        let bytes = std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2);
        let nm = name.map(HSTRING::from);
        let r = match &nm {
            Some(n) => RegSetValueExW(k, n, None, REG_SZ, Some(bytes)),
            None => RegSetValueExW(k, PCWSTR::null(), None, REG_SZ, Some(bytes)),
        };
        let _ = RegCloseKey(k);
        r.is_ok()
    }
}

/// Inscrit le lecteur dans « Ouvrir avec » (HKCU, sans droits admin).
pub fn register_open_with() -> std::result::Result<usize, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy();
    let cmd = format!("\"{exe}\" \"%1\"");
    let progid = "LecteurUniversel.Fichier";
    let base = format!(r"Software\Classes\{progid}");
    let mut ok = set_str(HKEY_CURRENT_USER, &base, None, "Fichier (Lecteur universel)");
    ok &= set_str(HKEY_CURRENT_USER, &format!(r"{base}\shell\open\command"), None, &cmd);
    ok &= set_str(HKEY_CURRENT_USER, r"Software\Classes\Applications\lecteur-universel.exe", Some("FriendlyAppName"), "Lecteur universel");
    ok &= set_str(HKEY_CURRENT_USER, r"Software\Classes\Applications\lecteur-universel.exe\shell\open\command", None, &cmd);
    let mut n = 0;
    for ext in crate::kind::all_exts() {
        let e = format!(".{ext}");
        set_str(HKEY_CURRENT_USER, &format!(r"Software\Classes\{e}\OpenWithProgids"), Some(progid), "");
        set_str(HKEY_CURRENT_USER, r"Software\Classes\Applications\lecteur-universel.exe\SupportedTypes", Some(&e), "");
        n += 1;
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    if ok { Ok(n) } else { Err("écriture dans le registre refusée".into()) }
}
