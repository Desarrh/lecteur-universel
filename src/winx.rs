//! Intégration Windows : boîte « Ouvrir », instance unique (tube nommé),
//! inscription dans « Ouvrir avec ».

use std::io::{Read, Write};
use std::path::PathBuf;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{PIPE_ACCESS_INBOUND, ReadFile};
use windows::Win32::System::Com::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::core::*;

const PIPE: &str = r"\\.\pipe\lecteur-universel-v1";
/// Instance d'aperçu, séparée de la fenêtre de travail normale
const PIPE_APERCU: &str = r"\\.\pipe\lecteur-universel-apercu-v1";

fn pipe(preview: bool) -> &'static str {
    if preview { PIPE_APERCU } else { PIPE }
}

/// Si une instance tourne déjà, lui envoie les fichiers et renvoie vrai.
/// Protocole (UTF-8, une ligne par chemin) : une ligne « \x02 » demande l'aperçu rapide,
/// un message « \x01 » seul ramène simplement la fenêtre au premier plan.
pub fn forward_to_running(paths: &[PathBuf], preview: bool) -> bool {
    match std::fs::OpenOptions::new().write(true).open(pipe(preview)) {
        Ok(mut f) => {
            // Ce processus, lancé par la fenêtre au premier plan (l'explorateur), a le droit de prendre
            // le focus : il le cède à l'instance qui va s'afficher, sinon Windows la laisse derrière
            // et Échap partirait à l'explorateur.
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(u32::MAX);
            }
            let mut s: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            if preview && !s.is_empty() {
                s.insert(0, "\u{2}".into());
            }
            let msg = if s.is_empty() { "\u{1}".to_string() } else { s.join("\n") };
            let _ = f.write_all(msg.as_bytes());
            true
        }
        Err(_) => false,
    }
}

/// Serveur : chaque message reçu est passé à `on_msg` (chemins, vide = « au premier plan » ; aperçu ?).
pub fn serve(preview_instance: bool, on_msg: impl Fn(Vec<PathBuf>, bool) + Send + 'static) {
    std::thread::spawn(move || {
        let name = HSTRING::from(pipe(preview_instance));
        loop {
            let h: HANDLE = unsafe {
                CreateNamedPipeW(&name, PIPE_ACCESS_INBOUND, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, 0, 65536, 0, None)
            };
            if h.is_invalid() {
                return;
            }
            let connected = unsafe { ConnectNamedPipe(h, None).is_ok() } || std::io::Error::last_os_error().raw_os_error() == Some(535);
            if connected {
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let mut n = 0u32;
                    let ok = unsafe { ReadFile(h, Some(&mut buf), Some(&mut n), None).is_ok() };
                    if !ok || n == 0 {
                        break;
                    }
                    data.extend_from_slice(&buf[..n as usize]);
                }
                let s = String::from_utf8_lossy(&data);
                let preview = s.split('\n').any(|l| l.trim_end_matches('\r') == "\u{2}");
                let paths = s
                    .split('\n')
                    .map(|l| l.trim_end_matches('\r'))
                    .filter(|l| !l.is_empty() && *l != "\u{1}" && *l != "\u{2}")
                    .map(PathBuf::from)
                    .collect();
                on_msg(paths, preview || preview_instance);
            }
            unsafe {
                let _ = DisconnectNamedPipe(h);
                let _ = CloseHandle(h);
            }
        }
    });
    let _ = std::io::empty().read(&mut []);
}

/// Boîte de dialogue « Ouvrir » native (sélection multiple).
pub fn open_dialog() -> Vec<PathBuf> {
    let mut out = Vec::new();
    unsafe {
        let Ok(d): Result<IFileOpenDialog> = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) else { return out };
        let _ = d.SetOptions(FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST);
        let n1 = HSTRING::from("Fichiers pris en charge");
        let p1 = HSTRING::from(crate::kind::DIALOG_PATTERN);
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
    for ext in crate::kind::ALL_EXTS {
        let e = format!(".{ext}");
        set_str(HKEY_CURRENT_USER, &format!(r"Software\Classes\{e}\OpenWithProgids"), Some(progid), "");
        set_str(HKEY_CURRENT_USER, r"Software\Classes\Applications\lecteur-universel.exe\SupportedTypes", Some(&e), "");
        n += 1;
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    if ok { Ok(n) } else { Err("écriture dans le registre refusée".into()) }
}
