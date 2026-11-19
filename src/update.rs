//! Mise à jour automatique depuis les releases GitHub (même mécanisme que l'explorateur).
//! - au plus une fois par 24 h, en arrière-plan : `curl.exe` (fourni avec Windows 10/11)
//!   interroge l'API GitHub ; aucune dépendance réseau dans l'exe ;
//! - si `tag_name` est plus récent que la version compilée : bandeau « Installer » ;
//! - installation : zip téléchargé dans %TEMP%, exe extrait, l'exe en cours est renommé
//!   en `.old` (Windows l'autorise), le nouveau prend sa place, relance, fermeture.
//!   Le `.old` est supprimé au démarrage suivant.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const REPO: &str = "Desarrh/lecteur-universel";
const NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub notes: String,
}

fn parse_ver(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let mut it = s.split(['.', '-']).map(|x| x.parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_ver(tag), parse_ver(current)), (Some(a), Some(b)) if a > b)
}

/// Interroge GitHub. `Ok(None)` = déjà à jour.
pub fn check() -> Result<Option<Release>, String> {
    let out = Command::new("curl.exe")
        .args(["-sL", "--max-time", "15", "-H", "User-Agent: lecteur-universel-updater", "-H", "Accept: application/vnd.github+json"])
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| format!("curl introuvable : {e}"))?;
    if !out.status.success() {
        return Err("pas de connexion".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| "réponse GitHub illisible".to_string())?;
    let Some(tag) = v.get("tag_name").and_then(|t| t.as_str()) else {
        return Err(v.get("message").and_then(|m| m.as_str()).unwrap_or("aucune release").to_string());
    };
    if !is_newer(tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let want = format!("Lecteur-{tag}-windows-x64.zip");
    let assets = v.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let url = assets
        .iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(want.as_str()))
        .or_else(|| assets.iter().find(|a| a.get("name").and_then(|n| n.as_str()).is_some_and(|n| n.ends_with(".zip"))))
        .and_then(|a| a.get("browser_download_url").and_then(|u| u.as_str()))
        .ok_or("aucun zip dans la release")?
        .to_string();
    let notes = v.get("body").and_then(|b| b.as_str()).unwrap_or("").to_string();
    Ok(Some(Release { version: tag.trim_start_matches('v').to_string(), url, notes }))
}

/// Télécharge, remplace l'exe en cours et renvoie le chemin du nouvel exe (à relancer).
pub fn install(r: &Release, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = std::env::temp_dir().join(format!("lecteur-maj-{}", r.version));
    let _ = std::fs::create_dir_all(&tmp);
    let zip_path = tmp.join("release.zip");
    progress(0.05);
    let st = Command::new("curl.exe")
        .args(["-sL", "--fail", "--max-time", "600", "-H", "User-Agent: lecteur-universel-updater", "-o"])
        .arg(&zip_path)
        .arg(&r.url)
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    if !st.success() {
        return Err("téléchargement échoué".into());
    }
    progress(0.7);
    let f = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("zip invalide : {e}"))?;
    let new_exe = tmp.join("lecteur-universel.exe");
    let mut found = false;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| e.to_string())?;
        let name = e.name().map(|n| n.replace('\\', "/")).unwrap_or_default();
        if name.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("lecteur-universel.exe")) {
            let mut o = std::fs::File::create(&new_exe).map_err(|e| e.to_string())?;
            std::io::copy(&mut e, &mut o).map_err(|e| e.to_string())?;
            found = true;
            break;
        }
    }
    if !found {
        return Err("lecteur-universel.exe absent du zip".into());
    }
    progress(0.9);
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("impossible de renommer l'exe : {e}"))?;
    if let Err(e) = std::fs::copy(&new_exe, &exe) {
        let _ = std::fs::rename(&old, &exe); // on remet l'ancien
        return Err(format!("impossible d'installer : {e}"));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    progress(1.0);
    Ok(exe)
}

fn old_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// À appeler au démarrage : supprime l'exe remplacé lors d'une mise à jour précédente.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

/// Relance le nouvel exe (il attend que celui-ci se ferme avant de démarrer).
pub fn relaunch(exe: &Path) {
    let _ = Command::new(exe).arg("--apres-maj").spawn();
}
