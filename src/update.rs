//! Mise à jour automatique depuis les releases GitHub, par le socle (`socle::update`) :
//! au plus une fois par 24 h, en arrière-plan, `curl.exe` interroge l'API GitHub ; si la version
//! est plus récente, bandeau « Installer » ; l'exe en cours est renommé `.old`, le nouveau prend
//! sa place, puis relance avec `--apres-maj`. Le `.old` est supprimé au démarrage suivant.

use std::path::{Path, PathBuf};

pub use socle::update::{Release, cleanup_old};

pub const REPO: &str = "Desarrh/lecteur-universel";

fn updater() -> socle::update::Updater {
    socle::update::Updater { repo: REPO, current: env!("CARGO_PKG_VERSION"), asset_suffix: "windows-x64.zip", exe_name: Some("lecteur-universel.exe") }
}

pub fn check() -> Result<Option<Release>, String> {
    updater().check()
}

pub fn install(r: &Release, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    updater().install(r, progress)
}

/// Relance le nouvel exe (il attend que celui-ci se ferme avant de démarrer).
pub fn relaunch(exe: &Path) {
    let _ = socle::update::relaunch(exe);
}
