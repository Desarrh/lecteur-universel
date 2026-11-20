//! Points forts automatiques (L2) : dans un enregistrement de jeu, les moments où le son monte
//! d'un coup (explosions, combats, cris). Calcul en arrière-plan sur le niveau sonore, lu en flux
//! (aucun gros tampon : environ 1 s de calcul par heure de vidéo hors décodage), annulable, puis
//! mis en cache par fichier (chemin + taille + date) dans %LOCALAPPDATA%\lecteur-universel.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// Un point fort : instant (s) et hausse du son par rapport aux 10 s précédentes (dB).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Moment {
    pub t: f64,
    pub gain: f32,
}

/// Pas d'analyse : une valeur de niveau toutes les 50 ms.
pub const HOP: f64 = 0.05;

/// Détecte les montées brusques dans une suite de niveaux (dBFS, un par `hop` secondes).
/// Base = moyenne des 10 s précédentes ; on garde les pics d'au moins 10 dB au-dessus, assez
/// forts (> -35 dBFS), espacés d'au moins 8 s, les 40 meilleurs, triés par instant.
pub fn detect(env: &[f32], hop: f64) -> Vec<Moment> {
    let n = env.len();
    if n < 4 {
        return Vec::new();
    }
    let w = ((10.0 / hop) as usize).max(1);
    let mut pre = vec![0f64; n + 1];
    for (i, v) in env.iter().enumerate() {
        pre[i + 1] = pre[i] + v.max(-90.0) as f64;
    }
    let mut cand: Vec<(usize, f32)> = Vec::new();
    for i in 1..n {
        let a = i.saturating_sub(w);
        let base = ((pre[i] - pre[a]) / (i - a) as f64) as f32;
        let v = env[i].max(-90.0);
        let gain = v - base;
        // maximum local (sur ± 0,5 s) pour ne garder que le sommet de chaque pic
        let r = (0.5 / hop) as usize;
        let lo = i.saturating_sub(r);
        let hi = (i + r + 1).min(n);
        let is_peak = env[lo..hi].iter().all(|x| *x <= env[i]);
        if gain >= 10.0 && v > -35.0 && is_peak {
            cand.push((i, gain));
        }
    }
    cand.sort_by(|a, b| b.1.total_cmp(&a.1));
    let gap = (8.0 / hop) as usize;
    let mut kept: Vec<(usize, f32)> = Vec::new();
    for c in cand {
        if kept.len() >= 40 {
            break;
        }
        if kept.iter().all(|k| k.0.abs_diff(c.0) >= gap) {
            kept.push(c);
        }
    }
    kept.sort_by_key(|k| k.0);
    kept.into_iter().map(|(i, gain)| Moment { t: i as f64 * hop, gain }).collect()
}

fn cache_file(path: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let m = std::fs::metadata(path).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    m.len().hash(&mut h);
    m.modified().ok()?.hash(&mut h);
    Some(socle::paths::cache_dir(crate::APP).join("points-forts").join(format!("{:016x}.json", h.finish())))
}

/// Lance l'analyse dans un fil (ou lit le cache). Le résultat arrive par le canal ; `cancel`
/// l'interrompt (onglet fermé).
pub fn spawn(path: PathBuf, wake: Arc<dyn Fn() + Send + Sync>, cancel: socle::Cancel) -> Receiver<Vec<Moment>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let cache = cache_file(&path);
        if let Some(v) = cache.as_ref().and_then(|c| std::fs::read(c).ok()).and_then(|b| serde_json::from_slice::<Vec<Moment>>(&b).ok()) {
            let _ = tx.send(v);
            wake();
            return;
        }
        crate::media::init_thread();
        if let Ok(env) = crate::media::loudness(&path.to_string_lossy(), HOP, &cancel) {
            let v = detect(&env, HOP);
            if let (Some(c), Ok(b)) = (cache, serde_json::to_vec(&v)) {
                let _ = socle::paths::write_atomic(&c, &b);
            }
            let _ = tx.send(v);
            wake();
        }
    });
    rx
}

/// Purge le cache des points forts (au démarrage, sur un fil) : entrées de plus de 90 jours,
/// puis les plus anciennes jusqu'à 20 Mo au total.
pub fn prune_cache() {
    let dir = socle::paths::cache_dir(crate::APP).join("points-forts");
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> =
        rd.flatten().filter_map(|e| e.metadata().ok().map(|m| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))).collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut total = 0u64;
    for (t, len, p) in files {
        total += len;
        let old = t.elapsed().map(|a| a.as_secs() > 90 * 24 * 3600).unwrap_or(false);
        if old || total > 20 << 20 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Annule un travail quand son propriétaire disparaît (onglet fermé).
pub struct CancelOnDrop(pub socle::Cancel);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
