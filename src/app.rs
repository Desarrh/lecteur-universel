use crate::convert::{self, Opts, Source, Target};
use crate::docx::{self, Block};
use crate::kind::{self, Kind, hms, human_size};
use crate::media::Player;
use crate::pdf::{self, PdfWorker};
use crate::theme::{self, Palette};
use crate::winx;

#[path = "app_ui.rs"]
mod ui;
#[path = "commands.rs"]
mod commands;

use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, FontFamily, FontId, Frame, Key, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke,
    TextFormat, TextureHandle, TextureOptions, Ui, Vec2, pos2, text::LayoutJob, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Réglages persistants

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub recents: Vec<String>,
    pub positions: HashMap<String, f64>,
    pub window: Option<(f32, f32)>,
    pub volume: Option<f64>,
    pub hide_rail: bool,
    /// Vrai = ne pas vérifier les mises à jour au démarrage
    pub no_auto_update: bool,
    /// Dernière vérification (secondes Unix)
    pub last_update_check: u64,
    /// Dernier usage (secondes Unix) de chaque position de reprise
    pub positions_used: HashMap<String, u64>,
    /// Commandes récentes de la palette Ctrl+K (en tête de liste)
    pub palette_recent: Vec<String>,
    /// Recherches récentes (autocomplétion des champs Ctrl+F)
    pub search_history: socle::autocomplete::History,
}

impl Settings {
    pub fn load() -> Settings {
        socle::settings::load(crate::APP)
    }
    pub fn save(&self) {
        if !crate::discret() {
            let _ = socle::settings::save(crate::APP, self);
        }
    }
}

// ---------------------------------------------------------------------------
// Onglets et vues

struct ImgV {
    img: image::RgbaImage,
    orig: (u32, u32),
    tex: Option<TextureHandle>,
    dirty: bool,
    zoom: Option<f32>,
    crop: bool,
    crop_rect: Option<(Pos2, Pos2)>,
    undo: Vec<(image::RgbaImage, String)>,
    history: Vec<String>,
    frames: usize,
    /// GIF animé : images + durée (s) ; textures créées au premier affichage.
    anim: Vec<(ColorImage, f32)>,
    anim_tex: Vec<TextureHandle>,
    anim_t0: Instant,
    /// Chargement en arrière-plan : aperçu réduit d'abord, pleine définition ensuite
    pending: Option<Receiver<ImgMsg>>,
    /// Vrai quand `img` est en pleine définition (les modifications attendent ce moment)
    full: bool,
    /// Annule le décodage si l'onglet disparaît
    _load: crate::moments::CancelOnDrop,
    /// Image d'affichage déjà prête pour la texture (≤ 2560 px, préparée dans le fil),
    /// absente après une modification
    disp: Option<ColorImage>,
}

impl ImgV {
    /// Taille affichée : réelle une fois chargée (modifications comprises), d'origine pendant le chargement.
    fn dims(&self) -> (u32, u32) {
        if self.full { self.img.dimensions() } else { self.orig }
    }
}

enum ImgMsg {
    Preview(ColorImage, (u32, u32)),
    Full(image::RgbaImage, ColorImage, Vec<(ColorImage, f32)>),
    Err(String),
}

/// Image prête pour une texture, réduite à 2560 px au plus (préparée hors du fil d'affichage).
fn display_copy(img: &image::RgbaImage) -> ColorImage {
    let m = img.width().max(img.height());
    if m > 2560 {
        let k = 2560.0 / m as f32;
        let r = image::imageops::resize(img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), image::imageops::FilterType::Triangle);
        ColorImage::from_rgba_unmultiplied([r.width() as usize, r.height() as usize], r.as_raw())
    } else {
        ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
    }
}

/// Décode une image dans un fil : aperçu rapide (gros JPEG), puis pleine définition
/// et copie réduite pour l'écran.
fn spawn_image_load(path: PathBuf, wake: Arc<dyn Fn() + Send + Sync>, cancel: socle::Cancel) -> Receiver<ImgMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let ext = kind::ext(&path);
        let big = std::fs::metadata(&path).map(|m| m.len() > 300_000).unwrap_or(false);
        if big && matches!(ext.as_str(), "jpg" | "jpeg" | "jfif" | "heic" | "heif" | "avif" | "tif" | "tiff") {
            if let Ok((img, orig)) = crate::wic::decode_scaled(&path.to_string_lossy(), 2560) {
                if orig.0.max(orig.1) > 2560 {
                    let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    let _ = tx.send(ImgMsg::Preview(ci, orig));
                    wake();
                }
            }
        }
        match crate::wic::open_any_with(&path, &cancel) {
            Ok(img) => {
                let disp = display_copy(&img);
                let anim = if ext == "gif" { load_gif(&path) } else { Vec::new() };
                let _ = tx.send(ImgMsg::Full(img, disp, anim));
            }
            Err(e) => {
                let _ = tx.send(ImgMsg::Err(e));
            }
        }
        wake();
    });
    rx
}

struct TxtV {
    text: String,
    saved: String,
    lang: String,
    wrap: bool,
    lossy: bool,
    truncated: bool,
    /// Fins de ligne Windows (CR LF) à remettre à l'enregistrement
    crlf: bool,
    ed: crate::editor::EditState,
}

struct PdfV {
    w: PdfWorker,
    pages: u32,
    sizes: Vec<(f32, f32)>,
    tex: HashMap<u32, (u32, TextureHandle)>,
    pending: HashSet<(u32, u32)>,
    page: u32,
    zoom: f32,
    err: Option<String>,
    scroll_to: Option<u32>,
    /// Texte des pages (extrait à la demande) et recherche
    texts: Option<Vec<String>>,
    texts_rx: Option<Receiver<Result<Vec<String>, String>>>,
    find_open: bool,
    find: String,
    find_hit: usize,
    focus_find: bool,
    /// Autocomplétion du champ de recherche
    sac: crate::editor::SearchAc,
    /// Page dont copier le texte dès qu'il est extrait (fil)
    copy_page: Option<u32>,
}

struct MedV {
    p: Option<Player>,
    err: Option<String>,
    peaks: Option<Vec<f32>>,
    peaks_rx: Option<Receiver<Vec<f32>>>,
    restore: Option<f64>,
    autoplay: bool,
    /// Vignettes de la frise de montage (remplies en arrière-plan)
    thumbs: Vec<Option<TextureHandle>>,
    thumbs_rx: Option<Receiver<(usize, image::RgbaImage)>>,
    thumbs_started: bool,
    loop_sel: bool,
    /// Dernière image vidéo (taille d'affichage), rectangle d'affichage (points), taille voulue (pixels)
    frame: Option<(u32, u32, Vec<u32>)>,
    vrect: Option<Rect>,
    want: (u32, u32),
    /// Sous-fenêtre où la carte graphique affiche la vidéo
    vw: Option<crate::vidwin::VideoWindow>,
    /// Sous-titres (nom du fichier, répliques), affichés ?, décalage (s), zone à l'écran
    subs: Option<(String, Vec<crate::subs::Cue>)>,
    subs_on: bool,
    sub_offset: f64,
    sub_rect: Option<Rect>,
    /// L1 : recherche d'une phrase dans les sous-titres (Ctrl+F) : barre ouverte, saisie,
    /// instants trouvés (recalculés quand la saisie change), autocomplétion
    sfind_open: bool,
    sfind: String,
    sfind_hits: Vec<f64>,
    sfind_last: String,
    sfind_focus: bool,
    sac: crate::editor::SearchAc,
    /// L2 : points forts (montées du son) calculés en arrière-plan, annulés à la fermeture
    moments: Option<Vec<crate::moments::Moment>>,
    moments_rx: Option<Receiver<Vec<crate::moments::Moment>>>,
    moments_job: Option<crate::moments::CancelOnDrop>,
    /// Annule le calcul de la forme d'onde si l'onglet se ferme
    _peaks_job: crate::moments::CancelOnDrop,
}

enum View {
    Image(Box<ImgV>),
    Text(Box<TxtV>),
    Pdf(Box<PdfV>),
    Docx(Vec<Block>),
    /// Document Word en cours de lecture (fil)
    DocxLoading(Receiver<Result<Vec<Block>, String>>),
    Media(Box<MedV>),
    Error(String),
}

struct Tab {
    id: u64,
    path: PathBuf,
    kind: Kind,
    size: u64,
    modified: Option<std::time::SystemTime>,
    view: View,
    sel: (Option<f64>, Option<f64>),
    confirm_close: bool,
    /// Fichiers voisins (date du dossier, liste) pour PgPréc / PgSuiv
    nb: Option<(Option<std::time::SystemTime>, Arc<Vec<PathBuf>>)>,
}

impl Tab {
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
    fn dirty(&self) -> bool {
        match &self.view {
            View::Text(t) => t.text != t.saved,
            View::Image(i) => !i.history.is_empty(),
            _ => false,
        }
    }
    fn range(&self) -> Option<(f64, f64)> {
        match self.sel {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            _ => None,
        }
    }
    fn player(&mut self) -> Option<&mut Player> {
        match &mut self.view {
            View::Media(m) => m.p.as_mut(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Rail {
    Edit,
    Convert,
    Info,
}

struct Job {
    label: String,
    progress: Arc<AtomicU32>,
    done: Arc<Mutex<Option<Result<Vec<PathBuf>, String>>>>,
}

pub struct App {
    pub s: Settings,
    pub pal: Palette,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    pub panels: bool,
    pub want_fullscreen: Option<bool>,
    pub fullscreen: bool,
    rail: Rail,
    target: Option<Target>,
    opts: Opts,
    batch: bool,
    job: Option<Job>,
    last_out: Vec<PathBuf>,
    /// Messages éphémères (socle) : dans la barre d'état, ou en bulle en vue épurée
    pub(crate) toasts: socle::ui::Toasts,
    pub drag_out: Option<Vec<PathBuf>>,
    pub drop_hover: bool,
    wake: Arc<dyn Fn() + Send + Sync>,
    last_move: Instant,
    view_w: f32,
    view_h: f32,
    pub title: String,
    /// Panneau de droite forcé quand la fenêtre est étroite
    rail_forced: bool,
    /// Couleur de fond de la fenêtre (bandes autour de la vidéo)
    pub clear: Color32,
    /// Compteur d'images vidéo nouvelles (mesure de fluidité)
    pub video_frames: u64,
    /// Fenêtre principale (parent des sous-fenêtres vidéo)
    pub parent_hwnd: Option<isize>,
    /// Zones d'interface posées sur la vidéo (barre du haut, toasts, menus), en points
    occl: Vec<Rect>,
    /// Mise à jour disponible (bandeau), vérification en cours, installation en cours
    update: Option<crate::update::Release>,
    upd_rx: Option<Receiver<(bool, Result<Option<crate::update::Release>, String>)>>,
    upd_hidden: bool,
    upd_installing: bool,
    /// Demande de fermeture (après installation d'une mise à jour)
    pub want_exit: bool,
    /// Aperçu rapide en cours (Espace dans l'Explorateur) : Échap le referme
    preview: Option<Preview>,
    /// Palette de commandes Ctrl+K et aide F1 (socle)
    palette: socle::ui::Palette,
    help_open: bool,
    /// Instance dédiée à l'aperçu (lancée par --apercu) : un seul fichier à la fois
    pub preview_only: bool,
    /// Une vidéo a été ouverte : le pilote graphique est chargé dans le processus (≈ 90 Mo
    /// que Windows ne rend jamais) → l'instance d'aperçu se recycle en se cachant.
    pub video_used: bool,
    /// Aperçu refermé : masquer la fenêtre (plus d'onglet) ou la réduire ; l'instance reste prête
    pub want_hide: bool,
    /// Écriture différée des réglages (2 s après un changement, sur un fil), et dernier état écrit
    saver: socle::settings::Saver,
    saved: Settings,
}

/// État à rétablir quand l'aperçu rapide se ferme.
struct Preview {
    path: PathBuf,
    /// Faux si le fichier était déjà ouvert : Échap ne ferme alors pas son onglet
    own_tab: bool,
    prev_active: usize,
    prev_panels: bool,
    /// La fenêtre était cachée ou réduite avant l'aperçu : elle le redevient
    hide_after: bool,
}

impl App {
    pub fn new(ctx: &egui::Context, s: Settings) -> App {
        let pal = theme::palette(s.dark);
        // Gras pour les titres (Word, Markdown, sous-titres) ; pas d'emoji (13 Mo de RAM)
        theme::install_fonts_with(ctx, theme::FontOptions { bold: true, ..Default::default() });
        if !crate::discret() {
            std::thread::spawn(crate::moments::prune_cache);
        }
        theme::apply(ctx, &pal);
        let c2 = ctx.clone();
        App {
            panels: true,
            pal,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            want_fullscreen: None,
            fullscreen: false,
            rail: Rail::Convert,
            target: None,
            opts: Opts::default(),
            batch: false,
            job: None,
            last_out: Vec::new(),
            toasts: socle::ui::Toasts::default(),
            drag_out: None,
            drop_hover: false,
            wake: Arc::new(move || c2.request_repaint()),
            last_move: Instant::now(),
            view_w: 1280.0,
            view_h: 700.0,
            rail_forced: false,
            clear: Color32::BLACK,
            video_frames: 0,
            parent_hwnd: None,
            occl: Vec::new(),
            update: None,
            upd_rx: None,
            upd_hidden: false,
            upd_installing: false,
            want_exit: false,
            preview: None,
            palette: { let mut pl = socle::ui::Palette::default(); pl.recent = s.palette_recent.clone(); pl },
            help_open: false,
            preview_only: false,
            video_used: false,
            want_hide: false,
            // écrit 300 ms après le dernier changement, au plus tard 2 s après le premier
            saver: socle::settings::Saver::debounced(std::time::Duration::from_millis(300), std::time::Duration::from_secs(2)),
            saved: s.clone(),
            title: "Lecteur universel".into(),
            s,
        }
    }

    /// Vérification des mises à jour en arrière-plan. `manual` : affiche le résultat même si à jour.
    pub fn check_updates(&mut self, manual: bool) {
        if self.upd_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            let _ = tx.send((manual, crate::update::check()));
            wake();
        });
        self.upd_rx = Some(rx);
        self.s.last_update_check = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    }

    /// Au démarrage : au plus une vérification par 24 h (sauf si désactivé ou en mode test).
    pub fn auto_update_check(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let test = std::env::var("LECTEUR_SHOT").is_ok() || std::env::var("LECTEUR_SOLO").is_ok();
        if !self.s.no_auto_update && !test && now.saturating_sub(self.s.last_update_check) >= 24 * 3600 {
            self.check_updates(false);
        }
    }

    fn install_update(&mut self) {
        let Some(r) = self.update.clone() else { return };
        self.upd_installing = true;
        self.start_job(format!("Mise à jour v{}", r.version), move |pr| crate::update::install(&r, pr).map(|p| vec![p]));
    }

    /// Réglages de test (captures automatiques) : LECTEUR_DARK, LECTEUR_RAIL=edit|info.
    pub fn test_env(&mut self, ctx: &egui::Context) {
        if let Ok(d) = std::env::var("LECTEUR_DARK") {
            self.set_theme(ctx, d == "1");
        }
        match std::env::var("LECTEUR_RAIL").as_deref() {
            Ok("edit") => self.rail = Rail::Edit,
            Ok("info") => self.rail = Rail::Info,
            _ => {}
        }
    }

    /// Image vidéo à recopier dans la fenêtre : (rectangle en points, largeur, hauteur, pixels).
    /// En affichage direct (GPU), pixels vides : la zone est seulement laissée libre.
    pub fn video_blit(&self) -> Option<(Rect, u32, u32, &[u32])> {
        match &self.tabs.get(self.active)?.view {
            View::Media(m) => {
                let r = m.vrect?;
                if m.p.as_ref().is_some_and(|p| p.hw_render) {
                    return Some((r, 0, 0, &[]));
                }
                let (w, h, px) = m.frame.as_ref()?;
                Some((r, *w, *h, px.as_slice()))
            }
            _ => None,
        }
    }

    /// Statistiques de l'onglet vidéo actif : (affichage direct ?, images affichées, abandonnées).
    pub fn video_stats(&self) -> Option<(bool, u64, u64)> {
        match &self.tabs.get(self.active)?.view {
            View::Media(m) => m.p.as_ref().map(|p| {
                let (r, d) = p.stats();
                (p.hw_render, r, d)
            }),
            _ => None,
        }
    }

    /// Place / masque les sous-fenêtres vidéo (après chaque image d'interface).
    pub fn sync_video(&mut self, ppp: f32) {
        let px = |r: &Rect| ((r.left() * ppp).round() as i32, (r.top() * ppp).round() as i32, (r.width() * ppp).round() as i32, (r.height() * ppp).round() as i32);
        let cuts: Vec<(i32, i32, i32, i32)> = self.occl.iter().map(px).collect();
        let active = self.active;
        let hide_all = self.drop_hover;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            if let View::Media(m) = &mut t.view {
                let Some(vw) = m.vw.as_mut() else { continue };
                match (i == active && !hide_all, m.vrect) {
                    (true, Some(r)) => {
                        let r = px(&r);
                        let mut cuts = cuts.clone();
                        if let Some(sr) = m.sub_rect {
                            cuts.push(px(&sr));
                        }
                        if vw.place(r, &cuts) {
                            if let Some(p) = &m.p {
                                p.set_output(r.2, r.3);
                            }
                        }
                    }
                    _ => vw.hide(),
                }
            }
        }
    }

    /// État de l'onglet actif (mode test) : type + détails, ou ERREUR.
    pub fn status_line(&self) -> String {
        let Some(t) = self.tabs.get(self.active) else { return "accueil".into() };
        let d = match &t.view {
            View::Error(e) => return format!("ERREUR {e}"),
            View::Image(v) => format!("{}x{} images={}{}", v.dims().0, v.dims().1, v.frames, if v.full { "" } else { " (chargement)" }),
            View::Text(v) => format!("{} lignes", v.text.lines().count()),
            View::Docx(b) => format!("{} blocs", b.len()),
            View::DocxLoading(_) => "lecture".into(),
            View::Pdf(v) => match &v.err {
                Some(e) => return format!("ERREUR {e}"),
                None => format!("{} pages, {} rendues", v.pages, v.tex.len()),
            },
            View::Media(m) => match (&m.err, &m.p) {
                (Some(e), _) => return format!("ERREUR {e}"),
                (None, Some(p)) => format!("durée {:.1}s video={} {}x{} t={:.1} image={}", p.duration, p.has_video, p.size.0, p.size.1, p.time(), m.frame.is_some()),
                _ => "ERREUR pas de lecteur".into(),
            },
        };
        format!("OK {} {d}", kind::label(t.kind))
    }

    fn say(&mut self, msg: impl Into<String>, err: bool) {
        if err {
            self.toasts.error(msg);
        } else {
            self.toasts.ok(msg);
        }
    }

    // ---------- Ouverture ----------

    pub fn open_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        for p in paths {
            let p = std::path::absolute(&p).map(|a| PathBuf::from(a.to_string_lossy().replace('/', "\\"))).unwrap_or(p);
            if p.is_dir() {
                continue;
            }
            if matches!(kind::ext(&p).as_str(), "srt" | "vtt") && self.attach_subs(&p) {
                continue;
            }
            if let Some(i) = self.tabs.iter().position(|t| t.path == p) {
                self.active = i;
                continue;
            }
            let tab = self.load(ctx, p, true);
            self.tabs.push(tab);
            self.active = self.tabs.len() - 1;
        }
        self.target = None;
        self.last_out.clear();
    }

    /// Aperçu rapide : ouvre le fichier en vue épurée ; Échap (ou Espace hors vidéo) le referme
    /// et rend la main à l'Explorateur. Un nouvel aperçu remplace le précédent.
    pub fn open_preview(&mut self, ctx: &egui::Context, path: PathBuf, hide_after: bool) {
        let (mut prev_active, mut prev_panels, mut hide_after) = (self.active, self.panels, hide_after);
        if let Some(old) = self.preview.take() {
            (prev_active, prev_panels, hide_after) = (old.prev_active, old.prev_panels, old.hide_after);
            if old.own_tab {
                if let Some(i) = self.tabs.iter().position(|t| t.path == old.path) {
                    if !self.tabs[i].dirty() {
                        self.close(i);
                    }
                }
            }
        }
        if self.preview_only {
            for i in (0..self.tabs.len()).rev() {
                if !self.tabs[i].dirty() {
                    self.close(i);
                }
            }
        }
        let had = self.tabs.len();
        self.open_paths(ctx, vec![path]);
        let Some(t) = self.tabs.get(self.active) else { return };
        self.preview = Some(Preview { path: t.path.clone(), own_tab: self.tabs.len() > had, prev_active, prev_panels, hide_after });
        self.panels = false;
    }

    pub fn close_preview(&mut self) {
        if self.preview_only {
            // Instance d'aperçu : tout fermer (fichiers voisins compris) puis cacher
            self.preview = None;
            for i in (0..self.tabs.len()).rev() {
                self.close(i);
            }
            self.want_hide = self.tabs.is_empty();
            return;
        }
        let Some(pv) = self.preview.take() else { return };
        if pv.own_tab {
            if let Some(i) = self.tabs.iter().position(|t| t.path == pv.path) {
                self.close(i);
                if self.tabs.iter().any(|t| t.path == pv.path) {
                    // modifications non enregistrées : on garde l'onglet (le toast l'explique)
                    self.preview = Some(pv);
                    return;
                }
            }
        }
        self.panels = pv.prev_panels;
        if pv.prev_active < self.tabs.len() {
            self.active = pv.prev_active;
        }
        self.want_hide = self.tabs.is_empty() || pv.hide_after;
    }

    /// Son ou vidéo de l'onglet actif, pour le panneau multimédia de Windows.
    pub fn media_now(&self) -> Option<crate::smtc::Now<'_>> {
        let t = self.tabs.get(self.active)?;
        let View::Media(m) = &t.view else { return None };
        let p = m.p.as_ref()?;
        Some(crate::smtc::Now { id: t.path.to_str()?, title: t.path.file_name()?.to_str()?, video: t.kind == Kind::Video, playing: p.playing() })
    }

    /// Touche multimédia (ou bouton du panneau Windows) : lecture/pause, fichier suivant/précédent.
    pub fn media_button(&mut self, ctx: &egui::Context, b: crate::smtc::Btn) {
        use crate::smtc::Btn;
        match b {
            Btn::PlayPause => {
                if let Some(View::Media(m)) = self.tabs.get_mut(self.active).map(|t| &mut t.view) {
                    if let Some(p) = m.p.as_mut() {
                        p.toggle();
                    }
                }
            }
            Btn::Next => self.neighbour(ctx, 1),
            Btn::Previous => self.neighbour(ctx, -1),
        }
    }

    /// Un travail (conversion, coupe…) est en cours
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    /// Attache des sous-titres à l'onglet vidéo actif. Faux s'il n'y a pas de vidéo active.
    fn attach_subs(&mut self, p: &Path) -> bool {
        let Some(t) = self.tabs.get_mut(self.active) else { return false };
        let View::Media(m) = &mut t.view else { return false };
        if t.kind != Kind::Video {
            return false;
        }
        match crate::subs::load(p) {
            Some(c) => {
                let n = c.len();
                m.subs = Some((p.file_name().unwrap_or_default().to_string_lossy().into_owned(), c));
                m.subs_on = true;
                m.sub_offset = 0.0;
                self.say(format!("Sous-titres chargés ({n} répliques)"), false);
            }
            None => self.say("Fichier de sous-titres illisible", true),
        }
        true
    }

    fn remember(&mut self, p: &Path) {
        let s = p.to_string_lossy().to_string();
        self.s.recents.retain(|r| *r != s);
        self.s.recents.insert(0, s);
        self.s.recents.truncate(12);
    }

    fn load(&mut self, _ctx: &egui::Context, path: PathBuf, autoplay: bool) -> Tab {
        self.remember(&path);
        let k = kind::of(&path);
        // Vidéo et PDF chargent le pilote graphique (rendu GPU / Direct2D) : l'aperçu se recycle ensuite
        self.video_used |= k == Kind::Video || k == Kind::Pdf;
        let meta = std::fs::metadata(&path).ok();
        let ps = path.to_string_lossy().to_string();
        let restore = self.s.positions.get(&ps).copied();
        // décodage d'image annulé si l'onglet est fermé ou remplacé (fichier voisin)
        let img_cancel = socle::Cancel::new();
        let view = match k {
            // Rien n'est décodé ici : l'onglet apparaît tout de suite, l'image arrive du fil.
            Kind::Image => View::Image(Box::new(ImgV {
                orig: (0, 0),
                img: image::RgbaImage::new(1, 1),
                tex: None,
                dirty: false,
                zoom: None,
                crop: false,
                crop_rect: None,
                undo: Vec::new(),
                history: Vec::new(),
                frames: 1,
                anim: Vec::new(),
                anim_tex: Vec::new(),
                anim_t0: Instant::now(),
                pending: Some(spawn_image_load(path.clone(), self.wake.clone(), img_cancel.clone())),
                _load: crate::moments::CancelOnDrop(img_cancel),
                full: false,
                disp: None,
            })),
            Kind::Text | Kind::Unknown => {
                const MAX: usize = 8 << 20;
                let read = std::fs::File::open(&path).and_then(|f| {
                    use std::io::Read;
                    let mut b = Vec::new();
                    f.take(MAX as u64 + 1).read_to_end(&mut b).map(|_| b)
                });
                match read {
                    Ok(mut b) => {
                        let truncated = b.len() > MAX;
                        b.truncate(MAX);
                        let crlf = b.windows(2).any(|w| w == b"\r\n");
                        let (text, lossy) = match String::from_utf8(b) {
                            Ok(s) => (s, false),
                            Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), true),
                        };
                        if lossy && text.chars().filter(|c| *c == '\u{FFFD}').count() > text.len() / 20 + 8 {
                            View::Error("Fichier binaire : pas d'aperçu possible.".into())
                        } else {
                            let text = text.replace("\r\n", "\n");
                            View::Text(Box::new(TxtV { saved: text.clone(), text, lang: kind::ext(&path), wrap: false, lossy, truncated, crlf, ed: Default::default() }))
                        }
                    }
                    Err(e) => View::Error(e.to_string()),
                }
            }
            Kind::Pdf => {
                let w = pdf::open(&ps, self.wake.clone());
                let page = restore.unwrap_or(0.0) as u32;
                View::Pdf(Box::new(PdfV {
                    w,
                    pages: 0,
                    sizes: vec![],
                    tex: HashMap::new(),
                    pending: HashSet::new(),
                    page,
                    zoom: 1.0,
                    err: None,
                    scroll_to: if page > 0 { Some(page) } else { None },
                    texts: None,
                    texts_rx: None,
                    find_open: false,
                    find: String::new(),
                    find_hit: 0,
                    focus_find: false,
                    sac: Default::default(),
                    copy_page: None,
                }))
            }
            // Word : lu dans un fil (un gros document ne bloque pas l'ouverture de l'onglet)
            Kind::Docx => {
                let (tx, rx) = std::sync::mpsc::channel();
                let (ps2, wake) = (ps.clone(), self.wake.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(docx::parse(&ps2));
                    wake();
                });
                View::DocxLoading(rx)
            }
            Kind::Audio | Kind::Video => {
                let audio = k == Kind::Audio;
                let vw = if audio { None } else { self.parent_hwnd.and_then(crate::vidwin::VideoWindow::new) };
                let hw = vw.as_ref().map(|v| v.hwnd.0 as isize);
                crate::trace("lecteur média : ouverture");
                let opened = Player::open(&ps, audio, self.wake.clone(), hw);
                crate::trace("lecteur média : ouvert");
                let (p, err) = match opened {
                    Ok(mut p) => {
                        p.set_volume(if crate::discret() { 0.0 } else { self.s.volume.unwrap_or(1.0) });
                        (Some(p), None)
                    }
                    Err(e) => (None, Some(e.message().to_string())),
                };
                // Forme d'onde lue en flux (mémoire indépendante de la durée), annulée à la fermeture
                let peaks_cancel = socle::Cancel::new();
                let peaks_rx = if audio {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let wake = self.wake.clone();
                    let (ps2, c) = (ps.clone(), peaks_cancel.clone());
                    std::thread::spawn(move || {
                        crate::media::init_thread();
                        if let Ok(pk) = crate::media::peaks_stream(&ps2, 1200, &c) {
                            let _ = tx.send(pk);
                            wake();
                        }
                    });
                    Some(rx)
                } else {
                    None
                };
                View::Media(Box::new(MedV {
                    p,
                    err,
                    peaks: None,
                    peaks_rx,
                    restore: restore.filter(|t| *t > 3.0),
                    autoplay,
                    thumbs: Vec::new(),
                    thumbs_rx: None,
                    thumbs_started: false,
                    loop_sel: true,
                    frame: None,
                    vrect: None,
                    want: (0, 0),
                    vw,
                    subs: if audio { None } else { crate::subs::find_sidecar(&path).and_then(|p| crate::subs::load(&p).map(|c| (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), c))) },
                    subs_on: true,
                    sub_offset: 0.0,
                    sub_rect: None,
                    sfind_open: false,
                    sfind: String::new(),
                    sfind_hits: Vec::new(),
                    sfind_last: String::new(),
                    sfind_focus: false,
                    sac: Default::default(),
                    moments: None,
                    moments_rx: None,
                    moments_job: None,
                    _peaks_job: crate::moments::CancelOnDrop(peaks_cancel),
                }))
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        Tab {
            id,
            path,
            kind: k,
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: meta.and_then(|m| m.modified().ok()),
            view,
            sel: (None, None),
            confirm_close: false,
            nb: None,
        }
    }

    fn store_position(&mut self, i: usize) {
        let Some(t) = self.tabs.get(i) else { return };
        let key = t.path.to_string_lossy().to_string();
        let v = match &t.view {
            View::Media(m) => m.p.as_ref().map(|p| {
                let x = p.time();
                if p.duration > 0.0 && x > p.duration - 5.0 { 0.0 } else { x }
            }),
            View::Pdf(p) => Some(p.page as f64),
            _ => None,
        };
        if let Some(v) = v {
            if v > 0.5 {
                self.s.positions_used.insert(key.clone(), now_secs());
                self.s.positions.insert(key, v);
            } else {
                self.s.positions_used.remove(&key);
                self.s.positions.remove(&key);
            }
            // au plus 300 : on retire la position utilisée le moins récemment
            while self.s.positions.len() > 300 {
                let Some(k) = self.s.positions.keys().min_by_key(|k| self.s.positions_used.get(*k).copied().unwrap_or(0)).cloned() else { break };
                self.s.positions.remove(&k);
                self.s.positions_used.remove(&k);
            }
        }
    }

    /// À chaque trame : si les réglages ont changé, écriture différée sur un fil ; une erreur
    /// d'écriture s'affiche en toast rouge. Jamais pour l'aperçu ni en test.
    fn autosave(&mut self, ctx: &egui::Context) {
        if self.preview_only || crate::discret() {
            return;
        }
        if self.s != self.saved {
            self.saved = self.s.clone();
            self.saver.touch();
        }
        if let Some(d) = self.saver.tick(crate::APP, &self.s) {
            ctx.request_repaint_after(d);
        }
        if let Some(e) = self.saver.take_error() {
            self.toasts.error(format!("Réglages non enregistrés : {e}"));
        }
    }

    /// Attend la fin d'une écriture en cours (avant l'enregistrement final à la fermeture).
    pub fn settings_flush(&mut self) {
        let _ = self.saver.flush(crate::APP, &self.s);
    }

    pub fn save_state(&mut self) -> Settings {
        for i in 0..self.tabs.len() {
            self.store_position(i);
        }
        self.s.clone()
    }

    fn close(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if self.tabs[i].dirty() && !self.tabs[i].confirm_close {
            self.tabs[i].confirm_close = true;
            self.say("Modifications non enregistrées : fermez à nouveau pour les abandonner.", true);
            return;
        }
        self.store_position(i);
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        self.target = None;
    }

    fn neighbour(&mut self, ctx: &egui::Context, dir: i32) {
        let Some(t) = self.tabs.get(self.active) else { return };
        if t.dirty() {
            self.say("Enregistrez d'abord vos modifications.", true);
            return;
        }
        let list = self.neighbours(self.active);
        let Some(t) = self.tabs.get(self.active) else { return };
        let Some(pos) = list.iter().position(|p| *p == t.path) else { return };
        let n = (pos as i32 + dir).rem_euclid(list.len() as i32) as usize;
        let next = list[n].clone();
        self.store_position(self.active);
        let tab = self.load(ctx, next, true);
        self.tabs[self.active] = tab;
        self.target = None;
    }

    /// Fichiers voisins de l'onglet `i`, gardés en mémoire tant que la date du dossier ne change pas.
    pub(crate) fn neighbours(&mut self, i: usize) -> Arc<Vec<PathBuf>> {
        let Some(t) = self.tabs.get_mut(i) else { return Arc::default() };
        let stamp = t.path.parent().and_then(|d| std::fs::metadata(d).ok()).and_then(|m| m.modified().ok());
        if let Some((s, l)) = &t.nb {
            if *s == stamp {
                return l.clone();
            }
        }
        let l = Arc::new(kind::neighbours(&t.path));
        t.nb = Some((stamp, l.clone()));
        l
    }

    fn set_theme(&mut self, ctx: &egui::Context, dark: bool) {
        self.s.dark = dark;
        self.pal = theme::palette(dark);
        theme::apply(ctx, &self.pal);
        for t in &mut self.tabs {
            if let View::Image(v) = &mut t.view {
                v.dirty = true;
            }
        }
    }

    // ---------- Boucle ----------

    fn tick(&mut self, ctx: &egui::Context) {
        // Mises à jour
        if let Some(rx) = &self.upd_rx {
            if let Ok((manual, r)) = rx.try_recv() {
                self.upd_rx = None;
                match r {
                    Ok(Some(rel)) => {
                        self.update = Some(rel);
                        self.upd_hidden = false;
                    }
                    Ok(None) if manual => self.say(format!("Le lecteur est à jour (v{}).", env!("CARGO_PKG_VERSION")), false),
                    Err(e) if manual => self.say(format!("Vérification impossible : {e}"), true),
                    _ => {}
                }
            }
        }
        if self.upd_installing {
            if let Some(j) = &self.job {
                let done = j.done.lock().ok().and_then(|mut d| d.take());
                if let Some(r) = done {
                    self.job = None;
                    self.upd_installing = false;
                    match r {
                        Ok(paths) => {
                            if let Some(exe) = paths.first() {
                                // l'aperçu en veille (ancienne version) quitte aussi
                                let _ = std::process::Command::new(exe).arg("--quitter-apercu").spawn();
                                crate::update::relaunch(exe);
                            }
                            self.want_exit = true;
                        }
                        Err(e) => self.say(format!("Mise à jour impossible : {e}"), true),
                    }
                }
            }
        }
        // Travail de conversion
        if let Some(j) = &self.job {
            let r = j.done.lock().ok().and_then(|mut d| d.take());
            if let Some(r) = r {
                let label = j.label.clone();
                self.job = None;
                match r {
                    Ok(paths) => {
                        let n = paths.len();
                        self.last_out = paths;
                        self.say(if n > 1 { format!("{label} : {n} fichiers créés") } else { format!("{label} : terminé") }, false);
                    }
                    Err(e) => self.say(format!("{label} : {e}"), true),
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        let mut new_frames = 0u64;
        let active = self.active;
        let preview_only = self.preview_only;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            match &mut t.view {
                View::Media(m) => {
                    if let Some(rx) = &m.peaks_rx {
                        if let Ok(pk) = rx.try_recv() {
                            m.peaks = Some(pk);
                            m.peaks_rx = None;
                        }
                    }
                    if let Some(rx) = &m.thumbs_rx {
                        while let Ok((i, img)) = rx.try_recv() {
                            let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                            if let Some(slot) = m.thumbs.get_mut(i) {
                                *slot = Some(ctx.load_texture(format!("thumb{i}"), ci, TextureOptions::LINEAR));
                            }
                        }
                    }
                    if let Some(rx) = &m.moments_rx {
                        if let Ok(v) = rx.try_recv() {
                            m.moments = Some(v);
                            m.moments_rx = None;
                        }
                    }
                    let Some(p) = m.p.as_mut() else { continue };
                    p.poll();
                    // L2 : points forts d'une vidéo de plus de 20 s (pas dans l'aperçu rapide, qui reste léger)
                    if t.kind == Kind::Video && p.ready && p.duration > 20.0 && m.moments_job.is_none() && !preview_only {
                        let c = socle::Cancel::new();
                        m.moments_rx = Some(crate::moments::spawn(t.path.clone(), self.wake.clone(), c.clone()));
                        m.moments_job = Some(crate::moments::CancelOnDrop(c));
                    }
                    if p.ready && p.has_video && !m.thumbs_started {
                        m.thumbs_started = true;
                        const N: usize = 40;
                        m.thumbs = vec![None; N];
                        let (tx, rx) = std::sync::mpsc::channel();
                        m.thumbs_rx = Some(rx);
                        let (ps, wake) = (t.path.to_string_lossy().to_string(), self.wake.clone());
                        std::thread::spawn(move || {
                            crate::media::init_thread();
                            let _ = crate::montage::video_thumbs(&ps, N, 200, &|i, img| {
                                let alive = tx.send((i, img)).is_ok();
                                wake();
                                alive
                            });
                        });
                    }
                    if p.ready {
                        if let Some(r) = m.restore.take() {
                            p.seek(r);
                        }
                        if m.autoplay && i == active {
                            m.autoplay = false;
                            p.play();
                        }
                    }
                    if let Some(e) = p.error.take() {
                        m.err = Some(e);
                    }
                    if i != active {
                        if p.playing() && p.has_video {
                            p.pause();
                        }
                        continue;
                    }
                    if p.has_video && m.want.0 > 1 {
                        // Windows met l'image à l'échelle d'affichage ; on la recopie telle quelle à l'écran.
                        let force = m.frame.as_ref().map(|f| (f.0, f.1) != m.want).unwrap_or(true);
                        if let Some(f) = p.frame_px(m.want.0, m.want.1, force) {
                            m.frame = Some(f);
                            new_frames += 1;
                        }
                    }
                    if p.playing() {
                        // Affichage direct : la vidéo s'affiche seule, l'interface (temps, frise) peut aller moins vite.
                        let ms = if p.hw_render { 40 } else if p.has_video { 12 } else { 120 };
                        ctx.request_repaint_after(std::time::Duration::from_millis(ms));
                    } else if p.has_video && m.frame.is_none() {
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }
                }
                View::Image(v) => {
                    let mut err = None;
                    if let Some(rx) = &v.pending {
                        while let Ok(m) = rx.try_recv() {
                            match m {
                                ImgMsg::Preview(ci, orig) => {
                                    v.disp = Some(ci);
                                    v.orig = orig;
                                    v.dirty = true;
                                }
                                ImgMsg::Full(img, disp, anim) => {
                                    v.orig = img.dimensions();
                                    v.img = img;
                                    v.disp = Some(disp);
                                    v.frames = anim.len().max(1);
                                    v.anim = anim;
                                    v.full = true;
                                    v.dirty = true;
                                }
                                ImgMsg::Err(e) => err = Some(e),
                            }
                        }
                        if v.full {
                            v.pending = None;
                        }
                    }
                    if let Some(e) = err {
                        t.view = View::Error(format!("Image illisible : {e}"));
                    }
                }
                View::DocxLoading(rx) => {
                    let got = rx.try_recv().ok();
                    if let Some(r) = got {
                        t.view = match r {
                            Ok(b) => View::Docx(b),
                            Err(e) => View::Error(format!("Document illisible : {e}")),
                        };
                    }
                }
                View::Pdf(v) => {
                    if let Some(rx) = &v.texts_rx {
                        if let Ok(r) = rx.try_recv() {
                            v.texts_rx = None;
                            match r {
                                Ok(t) => v.texts = Some(t),
                                Err(e) => {
                                    v.texts = Some(Vec::new());
                                    self.toasts.error(e);
                                }
                            }
                        }
                    }
                    if let (Some(pg), Some(t)) = (v.copy_page, v.texts.as_ref()) {
                        let txt = t.get(pg as usize).cloned().unwrap_or_default();
                        v.copy_page = None;
                        if txt.trim().is_empty() {
                            self.toasts.error("Pas de texte sur cette page (PDF scanné ?)");
                        } else {
                            ctx.copy_text(txt);
                            self.toasts.ok("Texte de la page copié");
                        }
                    }
                    if (v.find_open || v.copy_page.is_some() || std::env::var("LECTEUR_PDF_TEXTE").is_ok()) && v.texts.is_none() && v.texts_rx.is_none() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let (ps, wake) = (t.path.to_string_lossy().to_string(), self.wake.clone());
                        std::thread::spawn(move || {
                            let _ = tx.send(crate::pdf::page_texts(&ps));
                            wake();
                        });
                        v.texts_rx = Some(rx);
                    }
                    while let Ok(msg) = v.w.rx.try_recv() {
                        match msg {
                            pdf::Msg::Opened { pages, sizes } => {
                                v.pages = pages;
                                v.sizes = sizes;
                            }
                            pdf::Msg::Page { index, width, img } => {
                                v.pending.remove(&(index, width));
                                let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                                let keep = v.tex.get(&index).map(|(w, _)| *w > width).unwrap_or(false);
                                if !keep {
                                    v.tex.insert(index, (width, ctx.load_texture(format!("pdf{index}"), ci, TextureOptions::LINEAR)));
                                }
                            }
                            pdf::Msg::Error(e) => v.err = Some(e),
                            pdf::Msg::Dropped { index, width } => {
                                v.pending.remove(&(index, width));
                            }
                        }
                    }
                    // Libère la mémoire des pages loin de la page courante
                    let cur = v.page;
                    v.tex.retain(|k, _| k.abs_diff(cur) <= 6);
                }
                _ => {}
            }
        }
        self.video_frames += new_frames;
        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{}{} — Lecteur universel", if t.dirty() { "● " } else { "" }, t.name()),
            None => "Lecteur universel".into(),
        };
        self.title = title;
    }

    fn keys(&mut self, ctx: &egui::Context) {
        // Ctrl+K : palette de commandes ; F1 : tous les raccourcis. Ouvertes, elles gardent le clavier.
        self.palette.handle_shortcut(ctx);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F1)) {
            self.help_open = !self.help_open;
        }
        if self.palette.is_open() || self.help_open {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let (ctrl, shift) = ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if ctx.input(|i| i.pointer.delta() != Vec2::ZERO) {
            self.last_move = Instant::now();
        }
        if ctrl && pressed(Key::O) {
            let p = winx::open_dialog();
            self.open_paths(ctx, p);
        }
        if ctrl && pressed(Key::W) {
            self.close(self.active);
        }
        if ctrl && pressed(Key::Tab) && !self.tabs.is_empty() {
            self.active = if shift { (self.active + self.tabs.len() - 1) % self.tabs.len() } else { (self.active + 1) % self.tabs.len() };
        }
        if ctrl && pressed(Key::S) {
            let is_text = matches!(self.tabs.get(self.active).map(|t| &t.view), Some(View::Text(_)));
            self.save_current(if is_text { !shift } else { shift });
        }
        if ctrl && pressed(Key::Z) && !typing {
            self.undo_image();
        }
        if pressed(Key::F11) || (!typing && !ctrl && pressed(Key::F)) {
            self.want_fullscreen = Some(!self.fullscreen);
        }
        if pressed(Key::Escape) {
            let cropping = matches!(self.tabs.get(self.active).map(|t| &t.view), Some(View::Image(v)) if v.crop);
            if self.fullscreen {
                self.want_fullscreen = Some(false);
            } else if cropping {
                if let Some(View::Image(v)) = self.tabs.get_mut(self.active).map(|t| &mut t.view) {
                    v.crop = false;
                    v.crop_rect = None;
                }
            } else if (self.preview.is_some() || self.preview_only) && !typing {
                self.close_preview();
                return;
            }
        }
        if typing {
            return;
        }
        // Aperçu : Espace referme aussi (comme l'aperçu de macOS) ; la lecture se fait au clic ou par K.
        // ← → ouvrent le fichier voisin (J / L avancent dans une vidéo).
        if self.preview.is_some() || self.preview_only {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Space)) {
                self.close_preview();
                return;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowRight)) {
                self.neighbour(ctx, 1);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft)) {
                self.neighbour(ctx, -1);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Tab)) {
            self.panels = !self.panels;
        }
        if pressed(Key::PageDown) {
            self.neighbour(ctx, 1);
        }
        if pressed(Key::PageUp) {
            self.neighbour(ctx, -1);
        }
        let (mut no_subs, mut export) = (false, false);
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let kind = t.kind;
        match &mut t.view {
            View::Media(m) => {
                // L1 : Ctrl+F cherche une phrase dans les sous-titres
                if ctrl && pressed(Key::F) && kind == Kind::Video {
                    if m.subs.is_some() {
                        m.sfind_open = true;
                        m.sfind_focus = true;
                        m.sac.reset();
                    } else {
                        no_subs = true;
                    }
                }
                let moments: Vec<f64> = m.moments.iter().flatten().map(|x| x.t).collect();
                if m.subs.is_some() {
                    if pressed(Key::V) {
                        m.subs_on = !m.subs_on;
                    }
                    if pressed(Key::G) {
                        m.sub_offset -= 0.1;
                    }
                    if pressed(Key::H) {
                        m.sub_offset += 0.1;
                    }
                }
                let Some(p) = m.p.as_mut() else { return };
                if pressed(Key::Space) || pressed(Key::K) {
                    p.toggle();
                }
                let step = if shift { 1.0 } else { 5.0 };
                if pressed(Key::ArrowRight) {
                    p.seek(p.time() + step);
                }
                if pressed(Key::ArrowLeft) {
                    p.seek(p.time() - step);
                }
                if pressed(Key::ArrowUp) {
                    p.set_volume(p.volume + 0.1);
                    self.s.volume = Some(p.volume);
                }
                if pressed(Key::ArrowDown) {
                    p.set_volume(p.volume - 0.1);
                    self.s.volume = Some(p.volume);
                }
                if pressed(Key::Period) {
                    p.step(true);
                }
                if pressed(Key::Comma) {
                    p.step(false);
                }
                if pressed(Key::J) {
                    p.seek(p.time() - 10.0);
                }
                if pressed(Key::L) {
                    p.seek(p.time() + 10.0);
                }
                if pressed(Key::Home) && !shift {
                    p.seek(0.0);
                }
                let now = p.time();
                let dur = p.duration;
                // L2 : N / Maj+N = point fort suivant / précédent ; E = exporter ce moment (±10 s)
                if pressed(Key::N) && !ctrl {
                    let to = if shift { moments.iter().rev().find(|&&x| x < now - 1.0) } else { moments.iter().find(|&&x| x > now + 0.5) };
                    if let Some(&x) = to {
                        p.seek(x);
                    }
                }
                if pressed(Key::E) && !ctrl && !moments.is_empty() {
                    export = true;
                }
                if pressed(Key::OpenBracket) || pressed(Key::I) {
                    t.sel = (Some(now), Some(t.sel.1.filter(|b| *b > now + 0.05).unwrap_or(dur)));
                }
                if pressed(Key::CloseBracket) || pressed(Key::O) {
                    t.sel = (Some(t.sel.0.filter(|a| *a < now - 0.05).unwrap_or(0.0)), Some(now));
                }
                if pressed(Key::X) && !ctrl {
                    t.sel = (None, None);
                }
                if pressed(Key::Home) && shift {
                    // Revient au début de la sélection
                    if let Some((a, _)) = t.range() {
                        if let Some(p) = t.player() {
                            p.seek(a);
                        }
                    }
                }
                if kind == Kind::Video && pressed(Key::C) {
                    self.capture();
                }
                if no_subs {
                    self.say("Pas de sous-titres : glissez un fichier .srt ou .vtt sur la vidéo pour y chercher une phrase.", true);
                }
                if export {
                    self.export_moment();
                }
            }
            View::Pdf(v) => {
                if pressed(Key::ArrowRight) || pressed(Key::ArrowDown) && ctrl {
                    v.scroll_to = Some((v.page + 1).min(v.pages.saturating_sub(1)));
                }
                if pressed(Key::ArrowLeft) {
                    v.scroll_to = Some(v.page.saturating_sub(1));
                }
                if pressed(Key::Plus) || pressed(Key::Equals) {
                    v.zoom = (v.zoom * 1.2).min(4.0);
                }
                if pressed(Key::Minus) {
                    v.zoom = (v.zoom / 1.2).max(0.3);
                }
            }
            View::Image(v) => {
                if pressed(Key::Plus) || pressed(Key::Equals) {
                    v.zoom = Some(v.zoom.unwrap_or(1.0) * 1.25);
                }
                if pressed(Key::Minus) {
                    v.zoom = Some(v.zoom.unwrap_or(1.0) / 1.25);
                }
                if pressed(Key::Num0) {
                    v.zoom = None;
                }
                if pressed(Key::Num1) {
                    v.zoom = Some(1.0);
                }
                if pressed(Key::R) {
                    self.edit_image(if shift { "Pivoter à gauche" } else { "Pivoter à droite" });
                }
                if pressed(Key::ArrowRight) {
                    self.neighbour(ctx, 1);
                }
                if pressed(Key::ArrowLeft) {
                    self.neighbour(ctx, -1);
                }
            }
            _ => {}
        }
    }

    fn start_job(&mut self, label: String, f: impl FnOnce(&dyn Fn(f32)) -> Result<Vec<PathBuf>, String> + Send + 'static) {
        let progress = Arc::new(AtomicU32::new(0));
        let done = Arc::new(Mutex::new(None));
        let (p2, d2, wake) = (progress.clone(), done.clone(), self.wake.clone());
        std::thread::spawn(move || {
            crate::media::init_thread();
            let pr = |x: f32| {
                p2.store(x.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
                wake();
            };
            // Un panic (décodeur, encodeur…) devient une erreur affichée, pas un travail bloqué.
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&pr))).unwrap_or_else(|e| {
                let m = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                Err(format!("erreur interne {m}").trim().to_string())
            });
            if let Ok(mut d) = d2.lock() {
                *d = Some(r);
            }
            wake();
        });
        self.job = Some(Job { label, progress, done });
    }

    fn source_of(&self, i: usize) -> Source {
        let t = &self.tabs[i];
        let (image, text) = match &t.view {
            View::Image(v) if !v.history.is_empty() => (Some(v.img.clone()), None),
            View::Text(v) => (None, Some(v.text.clone())),
            _ => (None, None),
        };
        let (at, duration) = match &t.view {
            View::Media(m) => m.p.as_ref().map(|p| (p.time(), p.duration)).unwrap_or((0.0, 0.0)),
            _ => (0.0, 0.0),
        };
        Source { path: t.path.clone(), kind: t.kind, image, text, range: t.range(), at, duration }
    }

    fn start_convert(&mut self, tg: Target, tabs: Vec<usize>) {
        let sources: Vec<Source> = tabs.iter().map(|i| self.source_of(*i)).collect();
        let opts = self.opts.clone();
        let n = sources.len();
        self.last_out.clear();
        // Plusieurs images → un seul PDF (dans l'ordre des onglets)
        if tg == Target::Pdf && n > 1 && opts.pdf_merge && sources.iter().all(|s| s.kind == Kind::Image) {
            self.start_job(format!("→ PDF ({n} pages)"), move |pr| {
                let mut pages = Vec::new();
                for (i, s) in sources.iter().enumerate() {
                    let img = match &s.image {
                        Some(i) => i.clone(),
                        None => crate::wic::open_any(&s.path)?,
                    };
                    pages.push(crate::pdfgen::page_of(&img, opts.pdf_scan));
                    pr((i + 1) as f32 / (n + 1) as f32);
                }
                let first = &sources[0].path;
                let dir = first.parent().unwrap_or(Path::new(".")).to_path_buf();
                let stem = first.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let out = kind::free_path(&dir, &format!("{stem} (+{} pages)", n - 1), "pdf");
                crate::pdfgen::images_to_pdf(&pages, &out, 85)?;
                Ok(vec![out])
            });
            return;
        }
        self.start_job(format!("→ {}", tg.label()), move |pr| {
            let mut out = Vec::new();
            let mut errs = Vec::new();
            for (i, s) in sources.iter().enumerate() {
                let sub = |f: f32| pr((i as f32 + f) / n as f32);
                match convert::run(s, tg, &opts, &sub) {
                    Ok(mut v) => out.append(&mut v),
                    Err(e) => errs.push(format!("{} : {e}", s.path.file_name().unwrap_or_default().to_string_lossy())),
                }
            }
            if out.is_empty() && !errs.is_empty() { Err(errs.join(" ; ")) } else { Ok(out) }
        });
    }

    fn capture(&mut self) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let path = t.path.clone();
        let Some(pl) = t.player() else { return };
        let at = pl.time();
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        // Affichage direct : l'image est redécodée depuis le fichier, en pleine définition, dans un
        // travail de fond (environ 100 ms : l'interface ne se fige pas).
        if pl.hw_render {
            let out = kind::free_path(&dir, &format!("{stem} {}", hms(at).replace(':', "m")), "png");
            self.start_job("Capture".into(), move |_| {
                let img = crate::media::video_frames(&path.to_string_lossy(), at, at + 0.25, 8.0, 0, &|_| {})
                    .map_err(|e| e.message().to_string())?
                    .into_iter()
                    .next()
                    .ok_or("aucune image à cet instant")?;
                img.save(&out).map_err(|e| e.to_string())?;
                arboard_copy(img.as_raw(), img.width(), img.height());
                Ok(vec![out])
            });
            return;
        }
        let img = pl.capture();
        let Some(img) = img else {
            self.say("Capture impossible pour l'instant.", true);
            return;
        };
        let out = kind::free_path(&dir, &format!("{stem} {}", hms(at).replace(':', "m")), "png");
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        let (w, h) = (img.size[0] as u32, img.size[1] as u32);
        // Copie dans le presse-papiers (image) + fichier PNG
        let saved = image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8);
        let clip = arboard_copy(&rgba, w, h);
        match saved {
            Ok(_) => {
                self.last_out = vec![out.clone()];
                self.say(format!("Image capturée : {}{}", out.file_name().unwrap_or_default().to_string_lossy(), if clip { " (copiée)" } else { "" }), false);
            }
            Err(e) => self.say(e.to_string(), true),
        }
    }

    /// Exporte la sélection. Vidéo : `precise` = réencodage à l'image près,
    /// sinon copie sans réencodage (démarre sur l'image clé précédente).
    /// L2 : exporte ±10 s autour du point fort le plus proche (à 15 s près), sinon autour de
    /// l'instant courant. Coupe rapide (sans réencodage), à côté du fichier.
    pub(crate) fn export_moment(&mut self) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let View::Media(m) = &t.view else { return };
        let Some(p) = m.p.as_ref() else { return };
        let (now, dur) = (p.time(), p.duration);
        let c = m.moments.iter().flatten().map(|x| x.t).filter(|x| (x - now).abs() <= 15.0).min_by(|a, b| (a - now).abs().total_cmp(&(b - now).abs())).unwrap_or(now);
        t.sel = (Some((c - 10.0).max(0.0)), Some((c + 10.0).min(dur)));
        self.cut_media(false);
    }

    fn cut_media(&mut self, precise: bool) {
        let Some(t) = self.tabs.get(self.active) else { return };
        let Some((a, b)) = t.range() else {
            self.say("Choisissez d'abord une plage : rapprochez les poignées, ou touches I / O.", true);
            return;
        };
        let src = t.path.clone();
        let size = t.size;
        let kind = t.kind;
        let ext = kind::ext(&src);
        let dir = src.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = src.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let dur = match &t.view {
            View::Media(m) => m.p.as_ref().map(|p| p.duration).unwrap_or(0.0),
            _ => 0.0,
        };
        let label = if kind == Kind::Video { if precise { "Coupe précise" } else { "Coupe rapide" } } else { "Extrait" };
        self.start_job(label.into(), move |pr| {
            let s = src.to_string_lossy().to_string();
            if kind == Kind::Video {
                let out = kind::free_path(&dir, &format!("{stem} (extrait)"), "mp4");
                let os = out.to_string_lossy().to_string();
                let fast = if precise { Err(windows::core::Error::from(windows::Win32::Foundation::E_FAIL)) } else { crate::montage::remux(&s, &os, a, b, pr) };
                if fast.is_err() {
                    // Débit d'origine (borné) pour garder la qualité.
                    let kbps = if dur > 0.0 { ((size as f64 * 8.0 / dur / 1000.0) as u32).clamp(3000, 25000) } else { 8000 };
                    crate::montage::transcode(&s, &os, a, b, 0, kbps, pr).map_err(|e| e.message().to_string())?;
                }
                Ok(vec![out])
            } else if ext == "mp3" {
                let out = kind::free_path(&dir, &format!("{stem} (extrait)"), "mp3");
                crate::media::cut_mp3(&s, &out.to_string_lossy(), a, b)?;
                Ok(vec![out])
            } else {
                let pcm = crate::media::decode_pcm(&s, Some((a, b)), pr).map_err(|e| e.message().to_string())?;
                let out = kind::free_path(&dir, &format!("{stem} (extrait)"), "wav");
                crate::media::write_wav(&pcm, &out.to_string_lossy())?;
                Ok(vec![out])
            }
        });
    }

    fn undo_image(&mut self) {
        if let Some(View::Image(v)) = self.tabs.get_mut(self.active).map(|t| &mut t.view) {
            if let Some((img, _)) = v.undo.pop() {
                v.img = img;
                v.history.pop();
                v.dirty = true;
            }
        }
    }

    fn edit_image(&mut self, what: &str) {
        let Some(View::Image(v)) = self.tabs.get_mut(self.active).map(|t| &mut t.view) else { return };
        if !v.full {
            self.toasts.info("Image encore en chargement…");
            return;
        }
        v.disp = None;
        use image::imageops as op;
        let new = match what {
            "Pivoter à gauche" => op::rotate270(&v.img),
            "Pivoter à droite" => op::rotate90(&v.img),
            "Miroir horizontal" => op::flip_horizontal(&v.img),
            "Miroir vertical" => op::flip_vertical(&v.img),
            "Réduire 50 %" => op::resize(&v.img, (v.img.width() / 2).max(1), (v.img.height() / 2).max(1), op::FilterType::Lanczos3),
            "Limiter à 1920 px" => {
                let (w, h) = v.img.dimensions();
                let s = 1920.0 / w.max(h) as f32;
                if s >= 1.0 {
                    return;
                }
                op::resize(&v.img, (w as f32 * s) as u32, (h as f32 * s) as u32, op::FilterType::Lanczos3)
            }
            "Rogner" => {
                let Some((a, b)) = v.crop_rect else { return };
                let r = Rect::from_two_pos(a, b);
                let (x, y) = (r.min.x.max(0.0) as u32, r.min.y.max(0.0) as u32);
                let w = (r.width() as u32).min(v.img.width().saturating_sub(x));
                let h = (r.height() as u32).min(v.img.height().saturating_sub(y));
                if w < 2 || h < 2 {
                    return;
                }
                v.crop = false;
                v.crop_rect = None;
                op::crop_imm(&v.img, x, y, w, h).to_image()
            }
            _ => return,
        };
        let old = std::mem::replace(&mut v.img, new);
        v.undo.push((old, what.to_string()));
        // au plus 8 étapes et ~512 Mo d'images gardées pour Annuler (une photo 24 Mpx = 96 Mo)
        while v.undo.len() > 8 || (v.undo.len() > 1 && v.undo.iter().map(|(i, _)| i.as_raw().len()).sum::<usize>() > 512 << 20) {
            v.undo.remove(0);
        }
        v.history.push(what.to_string());
        v.dirty = true;
        v.zoom = None;
    }

    fn save_current(&mut self, replace: bool) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let path = t.path.clone();
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let ext = kind::ext(&path);
        let target = if replace { path.clone() } else { kind::free_path(&dir, &format!("{stem} (modifié)"), &ext) };
        // Rien à enregistrer (texte inchangé ou tronqué, image non modifiée) : pas de sauvegarde inutile
        let changed = match &t.view {
            View::Text(v) => !v.truncated && v.text != v.saved,
            View::Image(v) => !v.history.is_empty(),
            _ => false,
        };
        if !changed {
            return;
        }
        if replace {
            // L'original est copié dans Documents\Lecteur universel\Sauvegardes avant d'être
            // remplacé ; si la copie échoue, on ne remplace rien.
            if let Err(e) = backup_original(&path) {
                self.toasts.error(format!("Sauvegarde impossible, l'original n'est pas remplacé : {e}"));
                return;
            }
        }
        let res: Result<(), String> = match &mut t.view {
            View::Text(v) => {
                if v.truncated || v.text == v.saved {
                    return;
                }
                let out = if v.crlf { v.text.replace('\n', "\r\n") } else { v.text.clone() };
                let r = socle::paths::write_atomic(&target, out.as_bytes());
                if r.is_ok() && replace {
                    v.saved = v.text.clone();
                }
                r
            }
            View::Image(v) => {
                if v.history.is_empty() {
                    return;
                }
                let tg = match ext.as_str() {
                    "jpg" | "jpeg" => Target::Jpg,
                    "webp" => Target::Webp,
                    "bmp" => Target::Bmp,
                    "tif" | "tiff" => Target::Tiff,
                    "ico" => Target::Ico,
                    "gif" => Target::Gif,
                    _ => Target::Png,
                };
                let tmp = target.with_extension(format!("{ext}.lecteur-tmp"));
                let r = convert::save_raster(&v.img, &tmp, tg, &self.opts).and_then(|_| std::fs::rename(&tmp, &target).map_err(|e| e.to_string()));
                if r.is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
                if r.is_ok() && replace {
                    v.history.clear();
                    v.undo.clear();
                    v.orig = v.img.dimensions();
                }
                r
            }
            _ => return,
        };
        t.confirm_close = false;
        match res {
            Ok(_) => {
                if !replace {
                    self.last_out = vec![target.clone()];
                }
                self.say(format!("Enregistré : {}", target.file_name().unwrap_or_default().to_string_lossy()), false)
            }
            Err(e) => self.say(format!("Échec de l'enregistrement : {e}"), true),
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Dossier des sauvegardes, visible par l'utilisateur : Documents\Lecteur universel\Sauvegardes.
pub fn backup_dir() -> PathBuf {
    let docs = unsafe {
        use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
        SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
            PathBuf::from(s)
        })
    };
    docs.filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| socle::paths::config_dir(crate::APP)).join("Lecteur universel").join("Sauvegardes")
}

/// Copie `path` dans le dossier des sauvegardes (horodatée) et purge celles de plus de 30 jours.
fn backup_original(path: &Path) -> Result<(), String> {
    let dir = backup_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("{}-{}", now_secs(), path.file_name().unwrap_or_default().to_string_lossy());
    std::fs::copy(path, dir.join(name)).map_err(|e| e.to_string())?;
    let limit = std::time::Duration::from_secs(30 * 24 * 3600);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Dessin des vues (fonctions libres pour simplifier les emprunts)

/// Images d'un GIF animé (limité à ~200 Mo décodés pour rester léger).
fn load_gif(p: &Path) -> Vec<(ColorImage, f32)> {
    use image::AnimationDecoder;
    let Some(dec) = std::fs::File::open(p).ok().and_then(|f| image::codecs::gif::GifDecoder::new(std::io::BufReader::new(f)).ok()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut bytes = 0usize;
    for fr in dec.into_frames().flatten() {
        let (n, d) = fr.delay().numer_denom_ms();
        let secs = if d == 0 { 0.1 } else { (n as f32 / d as f32 / 1000.0).max(0.02) };
        let buf = fr.into_buffer();
        bytes += buf.len();
        if bytes > 200 << 20 {
            break;
        }
        out.push((ColorImage::from_rgba_unmultiplied([buf.width() as usize, buf.height() as usize], buf.as_raw()), secs));
    }
    if out.len() < 2 { Vec::new() } else { out }
}

fn fmt_date(secs: i64) -> String {
    // Date civile UTC → heure locale approximée via l'API Windows non nécessaire ici
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil(days);
    format!("{d:02}/{m:02}/{y} {:02}:{:02} UTC", rem / 3600, (rem % 3600) / 60)
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Copie une image dans le presse-papiers Windows (format DIB).
fn arboard_copy(rgba: &[u8], w: u32, h: u32) -> bool {
    crate::clip::copy_image(rgba, w, h)
}


// ---------------------------------------------------------------------------
// Auto-test des actions d'édition (lecteur-universel --selftest <dossier>)

#[cfg(feature = "dev")]
impl App {
    fn wait_job(&mut self) -> Result<Vec<PathBuf>, String> {
        for _ in 0..1200 {
            let Some(j) = &self.job else { return Err("aucun travail".into()) };
            if let Some(r) = j.done.lock().ok().and_then(|mut d| d.take()) {
                self.job = None;
                return r;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Err("délai dépassé".into())
    }

    pub fn selftest(ctx: &egui::Context, dir: &Path) -> Vec<(String, bool, String)> {
        let s = Settings { volume: Some(0.0), ..Default::default() };
        let mut a = App::new(ctx, s);
        let mut res = Vec::new();
        // 1. Image : pivoter + enregistrer une copie
        a.open_paths(ctx, vec![dir.join("i.png")]);
        for _ in 0..200 {
            a.tick(ctx);
            if matches!(a.tabs.get(a.active).map(|t| &t.view), Some(View::Image(v)) if v.full) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        a.edit_image("Pivoter à droite");
        a.save_current(false);
        let dims = image::image_dimensions(dir.join("i (modifié).png")).ok();
        res.push(("Image pivotée + copie".to_string(), dims == Some((1136, 640)), format!("{dims:?}")));
        // 2. Texte : modifier + copie
        a.open_paths(ctx, vec![dir.join("k.rs")]);
        if let Some(View::Text(v)) = a.tabs.get_mut(a.active).map(|t| &mut t.view) {
            v.text.push_str("\n// ajout test\n");
        }
        a.save_current(false);
        let txt = std::fs::read_to_string(dir.join("k (modifié).rs")).unwrap_or_default();
        res.push(("Texte modifié + copie".into(), txt.contains("// ajout test"), format!("{} octets", txt.len())));
        // 3. MP3 : extrait 2 s → 5 s sans réencodage
        a.open_paths(ctx, vec![dir.join("a.mp3")]);
        let i = a.active;
        a.tabs[i].sel = (Some(2.0), Some(5.0));
        a.cut_media(false);
        let r = a.wait_job();
        let d = r
            .as_ref()
            .ok()
            .and_then(|v| v.first().cloned())
            .and_then(|p| crate::media::decode_pcm(&p.to_string_lossy(), None, &|_| {}).ok())
            .map(|p| p.samples.len() as f64 / p.channels as f64 / p.rate as f64);
        res.push(("Coupe MP3 (3 s attendues)".into(), d.map(|x| (x - 3.0).abs() < 0.2).unwrap_or(false), format!("durée {d:?}")));
        // 4. Vidéo : capture d'image à 3 s
        a.open_paths(ctx, vec![dir.join("v.mp4")]);
        let mut seeked = false;
        for _ in 0..80 {
            a.tick(ctx);
            let i = a.active;
            if let Some(p) = a.tabs[i].player() {
                if p.ready && !seeked {
                    p.pause();
                    p.seek(3.0);
                    seeked = true;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        a.capture();
        let cap = a.last_out.first().cloned();
        // 4 bis. Coupe vidéo 3 s → 9 s : rapide (sans réencodage) et précise
        let i = a.active;
        a.tabs[i].sel = (Some(3.0), Some(9.0));
        for precise in [false, true] {
            a.cut_media(precise);
            let r = a.wait_job();
            let d = r.as_ref().ok().and_then(|v| v.first()).map(|p| crate::media::Player::probe_duration(&p.to_string_lossy()));
            let ok = d.map(|d| if precise { (d - 6.0).abs() < 0.3 } else { d >= 5.9 && d < 9.5 }).unwrap_or(false);
            res.push((format!("Coupe vidéo {}", if precise { "précise (6 s)" } else { "rapide (≥ 6 s, image clé)" }), ok, format!("durée {d:?}")));
        }
        let cd = cap.as_ref().and_then(|p| image::image_dimensions(p).ok());
        res.push(("Capture vidéo PNG".into(), cd == Some((1080, 1920)), format!("{cd:?}")));
        // 5. Conversion par lot : 2 images → WEBP
        a.open_paths(ctx, vec![dir.join("i.jpg")]);
        for _ in 0..200 {
            a.tick(ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
            if matches!(a.tabs.get(a.active).map(|t| &t.view), Some(View::Image(v)) if v.full) {
                break;
            }
        }
        let imgs: Vec<usize> = a.tabs.iter().enumerate().filter(|(_, t)| t.kind == Kind::Image).map(|(i, _)| i).collect();
        a.start_convert(Target::Webp, imgs.clone());
        let r = a.wait_job();
        res.push(("Lot de 2 images → WEBP".into(), r.as_ref().map(|v| v.len() == imgs.len()).unwrap_or(false), format!("{} fichiers", r.map(|v| v.len()).unwrap_or(0))));
        // 8. L2 : un niveau calme (-40 dB) avec une explosion à 30 s (-8 dB) → un point fort à 30 s
        let mut env = vec![-40.0f32; (60.0 / crate::moments::HOP) as usize];
        for i in 0..6 {
            env[(30.0 / crate::moments::HOP) as usize + i] = -8.0;
        }
        let ms = crate::moments::detect(&env, crate::moments::HOP);
        res.push(("Point fort détecté (L2)".into(), ms.len() == 1 && (ms[0].t - 30.0).abs() < 0.5, format!("{ms:?}")));
        // 9. L2 : niveau sonore lu en flux sur la vidéo de test
        let lv = crate::media::loudness(&dir.join("v.mp4").to_string_lossy(), crate::moments::HOP, &socle::Cancel::new());
        let n = lv.as_ref().map(|v| v.len()).unwrap_or(0);
        res.push(("Niveau sonore en flux (L2)".into(), n > 100, format!("{n} valeurs")));
        // 10. L1 : phrase trouvée dans les sous-titres, sans tenir compte des accents ni de la casse
        let cues = crate::subs::parse("1\n00:00:01,000 --> 00:00:02,000\nBonjour\n\n2\n00:00:05,500 --> 00:00:07,000\nOn a gagné la partie !\n");
        let hits = crate::subs::find(&cues, "GAGNE la");
        res.push(("Phrase dans les sous-titres (L1)".into(), hits == vec![5.5], format!("{hits:?}")));
        // 11. Sous-titres : BOM, virgule ou point, réglages VTT après l'horodatage
        let vtt = crate::subs::parse("\u{feff}WEBVTT\n\n00:00:01.500 --> 00:00:03.000 align:start position:10%\nUn\n\n00:00:04,000 --> 00:00:05,250\nDeux\n");
        let ok = vtt.len() == 2 && (vtt[0].start - 1.5).abs() < 1e-6 && (vtt[1].end - 5.25).abs() < 1e-6 && vtt[0].text == "Un";
        res.push(("Sous-titres BOM / VTT / virgule".into(), ok, format!("{} répliques", vtt.len())));
        // 12. Word piégé : 100 Mo de texte qui se compressent en quelques Ko → refusé
        let bomb = dir.join("bombe.docx");
        let made = (|| -> Result<(), String> {
            use std::io::Write;
            let f = std::fs::File::create(&bomb).map_err(|e| e.to_string())?;
            let mut z = zip::ZipWriter::new(f);
            z.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).map_err(|e| e.to_string())?;
            let chunk = vec![b' '; 1 << 20];
            for _ in 0..100 {
                z.write_all(&chunk).map_err(|e| e.to_string())?;
            }
            z.finish().map_err(|e| e.to_string())?;
            Ok(())
        })();
        let r = crate::docx::parse(&bomb.to_string_lossy());
        let _ = std::fs::remove_file(&bomb);
        res.push(("Word de 100 Mo refusé".into(), made.is_ok() && r.is_err(), format!("{:?}", r.err())));
        // 13. PDF maison : parenthèses et antislash relus par lopdf
        let pdfp = dir.join("echappement.pdf");
        let w = crate::pdfgen::text_to_pdf("a (b) \\ c", &pdfp);
        let back = crate::pdf::page_texts(&pdfp.to_string_lossy()).ok().and_then(|t| t.into_iter().next()).unwrap_or_default();
        let _ = std::fs::remove_file(&pdfp);
        res.push(("PDF : ( ) \\ relus".into(), w.is_ok() && back.contains("a (b) \\ c"), back.trim().to_string()));
        // 14. Un PDF ne se convertit ni en ICO ni en GIF
        let tg = crate::convert::targets(Kind::Pdf, "pdf");
        res.push(("PDF sans ICO ni GIF".into(), !tg.contains(&Target::Ico) && !tg.contains(&Target::Gif), format!("{} formats", tg.len())));
        res
    }
}
