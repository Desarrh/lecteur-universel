#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clip;
mod convert;
mod docx;
mod editor;
pub use socle::icons;
mod kind;
mod media;
mod moments;
mod montage;
mod pdfgen;
mod smtc;
mod subs;
mod testgen;
mod update;
mod vidwin;
mod pdf;
pub use socle::theme;
mod wic;
mod winx;

/// Mode discret (règle n°0) : tests et captures ne dérangent jamais l'utilisateur. Fenêtre hors
/// écran, sans focus ni bouton de barre des tâches, son coupé, réglages jamais écrits, tube factice.
/// Automatique avec LECTEUR_SHOT / LECTEUR_SOLO, ou forcé par LECTEUR_DISCRET=1.
pub fn discret() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| ["LECTEUR_SHOT", "LECTEUR_SOLO", "LECTEUR_DISCRET", "SOCLE_TEST"].iter().any(|k| std::env::var_os(k).is_some()))
}

/// Mesure du démarrage (LECTEUR_TRACE=1) : ms depuis le lancement du processus.
static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
pub fn trace(what: &str) {
    // LECTEUR_TRACE=<fichier> : ajout direct dans le fichier (pas de console : trop lente à mesurer)
    if let Some(f) = std::env::var_os("LECTEUR_TRACE") {
        use std::io::Write;
        let ms = T0.get().map(|t| t.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0);
        if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
            let _ = writeln!(fh, "[{ms:7.1} ms] {what}");
        }
    }
}

use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

/// Rendu sans GPU, comme l'explorateur : egui dessine sur le processeur,
/// softbuffer copie l'image dans la fenêtre (GDI). Pas de contexte OpenGL/DirectX.
struct Gfx {
    window: Rc<Window>,
    egui: egui_winit::State,
    _context: softbuffer::Context<Rc<Window>>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    renderer: EguiSoftwareRender,
}

enum UserEvent {
    RepaintAt(Instant),
    /// Fichiers reçus par le tube ; vrai = aperçu rapide
    Open(Vec<PathBuf>, bool),
    /// Touche multimédia / panneau multimédia de Windows
    Media(smtc::Btn),
    /// « Fermer l'aperçu » reçu par le tube (--fermer-apercu)
    ClosePreview,
}

struct Runner {
    ctx: egui::Context,
    gfx: Option<Gfx>,
    app: Option<app::App>,
    settings: Option<app::Settings>,
    next_repaint: Option<Instant>,
    launch: Vec<PathBuf>,
    /// Lancé avec --apercu
    launch_preview: bool,
    /// --apercu-veille : instance d'aperçu neuve, fenêtre cachée dès le départ
    standby: bool,
    /// Fenêtre masquée après un aperçu : l'instance reste prête 10 min puis se ferme
    hidden_since: Option<Instant>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    /// Panneau multimédia de Windows, créé au premier son ou vidéo (rien à payer sinon)
    smtc: Option<smtc::Smtc>,
    smtc_tried: bool,
    smtc_soon: bool,
    title: String,
    started: Instant,
    /// Mode test : actions souris/clavier rejouées (LECTEUR_EVENTS), en points logiques.
    script: Vec<(u64, String)>,
    script_pos: egui::Pos2,
    fps: (Instant, u64, u64),
    first_paints: u32,
    prof: (f64, f64, f64),
    eng: (u64, u64),
}

impl Runner {
    fn paint(&mut self) {
        let (Some(gfx), Some(app)) = (self.gfx.as_mut(), self.app.as_mut()) else { return };
        let size = gfx.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };

        let t_start = Instant::now();
        let mut raw = gfx.egui.take_egui_input(&gfx.window);
        // Test : la fenêtre invisible n'est jamais active (règle n°0) ; le socle fait croire à egui
        // qu'elle a le focus pour que les champs gardent le clavier pendant un test scripté.
        socle::testmode::prepare_input(&mut raw);
        // Rejoue les actions de test arrivées à échéance.
        let now_ms = self.started.elapsed().as_millis() as u64;
        while self.script.first().is_some_and(|(t, _)| *t <= now_ms) {
            let (_, cmd) = self.script.remove(0);
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            let pos = self.script_pos;
            match parts.as_slice() {
                ["move", x, y] => {
                    self.script_pos = egui::pos2(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0));
                    raw.events.push(egui::Event::PointerMoved(self.script_pos));
                }
                ["down"] | ["up"] => raw.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: parts[0] == "down", modifiers: Default::default() }),
                ["key", k] | ["ctrl", k] | ["shift", k] => {
                    if let Some(key) = egui::Key::from_name(k) {
                        let modifiers = match parts[0] {
                            "ctrl" => egui::Modifiers { ctrl: true, command: true, ..Default::default() }, // comme egui-winit sous Windows
                            "shift" => egui::Modifiers::SHIFT,
                            _ => Default::default(),
                        };
                        // Modificateur tenu pendant cette image, relâché à l'image suivante.
                        raw.events.push(egui::Event::ModifiersChanged(modifiers));
                        raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
                        if modifiers != egui::Modifiers::default() {
                            self.script.insert(0, (now_ms + 40, "release".into()));
                        }
                    }
                }
                ["release"] => raw.events.push(egui::Event::ModifiersChanged(Default::default())),
                ["text", ..] => raw.events.push(egui::Event::Text(cmd["text ".len()..].to_string())),
                _ => {}
            }
        }
        if !self.script.is_empty() {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        let mut out = self.ctx.run_ui(raw, |ui| app.draw(ui));
        // epaint exige que les deltas de textures soient « consommés » (sinon panique en debug).
        let mut tdelta = std::mem::take(&mut out.textures_delta);
        app.sync_video(out.pixels_per_point);
        let now = app.media_now();
        // Créé à l'image suivante (≈ 45 ms dans un processus neuf) : la 1re image du fichier passe avant.
        if now.is_some() && !self.smtc_tried && !self.smtc_soon {
            self.smtc_soon = true;
            self.ctx.request_repaint();
        } else if now.is_some() && !self.smtc_tried {
            self.smtc_tried = true;
            if let Some(h) = app.parent_hwnd {
                let px = self.proxy.clone();
                self.smtc = smtc::Smtc::new(h, move |b| {
                    let _ = px.send_event(UserEvent::Media(b));
                });
                trace("panneau multimédia prêt");
            }
        }
        if let Some(s) = self.smtc.as_mut() {
            s.update(now);
        }
        gfx.egui.handle_platform_output(&gfx.window, out.platform_output);
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);

        if let Some(fs) = app.want_fullscreen.take() {
            gfx.window.set_fullscreen(if fs { Some(Fullscreen::Borderless(None)) } else { None });
            app.fullscreen = fs;
            app.panels = !fs && app.panels;
        }
        if app.title != self.title {
            self.title = app.title.clone();
            gfx.window.set_title(&self.title);
        }

        if gfx.surface.resize(w, h).is_err() {
            tdelta.clear();
            return;
        }
        let Ok(mut buf) = gfx.surface.buffer_mut() else {
            tdelta.clear();
            return;
        };
        let t_ui = t_start.elapsed().as_secs_f64();
        let bg = app.clear;
        let bgc = ((bg.r() as u32) << 16) | ((bg.g() as u32) << 8) | bg.b() as u32;
        // Vidéo : l'image (déjà à la taille d'affichage) est recopiée directement dans
        // le tampon, sous l'interface — pas de texture egui ni de mise à l'échelle CPU.
        // Le fond n'est rempli qu'autour de la vidéo.
        if let Some((r, fw, fh, px)) = app.video_blit() {
            let ppp = out.pixels_per_point;
            let (bw, bh) = (w.get() as i64, h.get() as i64);
            let (rx, ry) = ((r.left() * ppp).round() as i64, (r.top() * ppp).round() as i64);
            let (rw, rh) = ((r.width() * ppp).round() as i64, (r.height() * ppp).round() as i64);
            let (y0, y1) = (ry.clamp(0, bh), (ry + rh).clamp(0, bh));
            let (x0, x1) = (rx.clamp(0, bw), (rx + rw).clamp(0, bw));
            buf[..(y0 * bw) as usize].fill(bgc);
            buf[(y1 * bw) as usize..].fill(bgc);
            for y in y0..y1 {
                buf[(y * bw) as usize..(y * bw + x0) as usize].fill(bgc);
                buf[(y * bw + x1) as usize..((y + 1) * bw) as usize].fill(bgc);
            }
            let (fw0, fh0) = (fw, fh);
            if fw0 == 0 || fh0 == 0 {
                // affichage direct par le GPU : rien à copier
            } else {
            let (fw, fh) = (fw as i64, fh as i64);
            for y in ry.max(0)..(ry + rh).min(bh) {
                let sy = ((y - ry) * fh / rh.max(1)).min(fh - 1);
                let src = &px[(sy * fw) as usize..((sy + 1) * fw) as usize];
                let x0 = rx.max(0);
                let x1 = (rx + rw).min(bw);
                let dst = &mut buf[(y * bw + x0) as usize..(y * bw + x1) as usize];
                if fw == rw {
                    dst.copy_from_slice(&src[(x0 - rx) as usize..(x1 - rx) as usize]);
                } else {
                    // taille en cours de changement (redimensionnement) : plus proche voisin
                    for (i, d) in dst.iter_mut().enumerate() {
                        *d = src[(((x0 + i as i64 - rx) * fw / rw.max(1)).min(fw - 1)) as usize];
                    }
                }
            }
            }
        } else {
            buf.fill(bgc);
        }
        {
            let px: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buf[..]);
            let mut target = BufferMutRef::new(px, w.get() as usize, h.get() as usize);
            gfx.renderer.render(&mut target, &prims, &tdelta, out.pixels_per_point);
        }
        // Mode test : LECTEUR_SHOT=fichier.png enregistre l'écran rendu après LECTEUR_SHOT_MS puis quitte.
        if let Ok(shot) = std::env::var("LECTEUR_SHOT") {
            let ms: u64 = std::env::var("LECTEUR_SHOT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(2500);
            if self.started.elapsed().as_millis() as u64 >= ms {
                let rgba: Vec<u8> = buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
                let _ = image::save_buffer(&shot, &rgba, w.get(), h.get(), image::ColorType::Rgba8);
                let _ = std::fs::write(format!("{shot}.txt"), app.status_line());
                std::process::exit(0);
            }
            self.ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let t_draw = t_start.elapsed().as_secs_f64();
        tdelta.clear();
        let _ = buf.present();
        if self.first_paints < 3 {
            self.first_paints += 1;
            trace(&format!("image {} affichée", self.first_paints));
        }
        let t_all = t_start.elapsed().as_secs_f64();
        self.prof.0 += t_ui;
        self.prof.1 += t_draw - t_ui;
        self.prof.2 += t_all - t_draw;
        // Mesure : LECTEUR_FPS=1 affiche images peintes / images vidéo par seconde.
        self.fps.1 += 1;
        if self.fps.0.elapsed().as_secs_f32() >= 1.0 {
            if std::env::var("LECTEUR_FPS").is_ok() {
                let n = self.fps.1.max(1) as f64;
                let (tr, rd, nf) = media::PROF.with(|p| std::mem::take(&mut *p.borrow_mut()));
                let nf2 = nf.max(1) as f64;
                eprintln!(
                    "{} x {} : {} rendus/s, {} images vidéo/s | par rendu : UI+vidéo {:.1} ms, remplissage+egui {:.1} ms, affichage {:.1} ms | GPU transfert {:.1} ms, relecture {:.1} ms",
                    w, h, self.fps.1, app.video_frames - self.fps.2, self.prof.0 / n * 1e3, self.prof.1 / n * 1e3, self.prof.2 / n * 1e3, tr / nf2 * 1e3, rd / nf2 * 1e3
                );
                self.prof = (0.0, 0.0, 0.0);
                if let Some((hw, r, d)) = app.video_stats() {
                    if std::env::var("LECTEUR_FPS").is_ok() {
                        eprintln!("   moteur {} : {} images affichées/s, {} abandonnées/s", if hw { "GPU direct" } else { "relecture" }, r - self.eng.0, d - self.eng.1);
                    }
                    self.eng = (r, d);
                }
            }
            self.fps = (Instant::now(), 0, app.video_frames);
        }

        // Glisser le fichier converti vers l'Explorateur, Discord… (OLE)
        if let Some(paths) = app.drag_out.take() {
            let _ = drag::start_drag(&*gfx.window, drag::DragItem::Files(paths), drag::Image::Raw(Vec::new()), |_, _| {}, drag::Options::default());
            let input = gfx.egui.egui_input_mut();
            if let Some(pos) = self.ctx.input(|i| i.pointer.latest_pos()) {
                input.events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
            }
            input.events.push(egui::Event::PointerGone);
            gfx.window.request_redraw();
        }
    }

    fn save(&mut self) {
        if self.launch_preview || discret() {
            return; // l'instance d'aperçu ne touche pas aux réglages de la fenêtre de travail
        }
        if let Some(app) = self.app.as_mut() {
            let mut s = app.save_state();
            if let Some(g) = &self.gfx {
                if !app.fullscreen && !g.window.is_maximized() {
                    let sz = g.window.inner_size().to_logical::<f32>(g.window.scale_factor());
                    s.window = Some((sz.width, sz.height));
                }
            }
            s.save();
        }
    }
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let mut size = self.settings.as_ref().and_then(|s| s.window).unwrap_or((1280.0, 800.0));
        // Test : LECTEUR_SIZE=800x600
        if let Some((a, b)) = std::env::var("LECTEUR_SIZE").ok().and_then(|v| v.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280.0), b.parse().unwrap_or(800.0)))) {
            size = (a, b);
        }
        let (w, h) = size;
        let attrs = Window::default_attributes()
            .with_title("Lecteur universel")
            .with_inner_size(LogicalSize::new(w.max(640.0), h.max(420.0)))
            .with_min_inner_size(LogicalSize::new(720.0, 480.0))
            .with_window_icon(window_icon())
            .with_visible(!self.standby);
        // Mode discret (règle n°0) : créée invisible et sans focus, puis rendue transparente
        // par le socle (opacité 0, traversée par la souris, jamais active, hors barre des tâches).
        let attrs = if discret() { attrs.with_visible(false).with_active(false) } else { attrs };
        trace("resumed");
        let window = Rc::new(el.create_window(attrs).expect("création de la fenêtre"));
        if discret() && !self.standby {
            discret_show(&window);
        }
        trace("fenêtre créée");
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        let egui = egui_winit::State::new(self.ctx.clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), None, Some(8192));
        let renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);
        if self.app.is_none() {
            let s = self.settings.take().unwrap_or_default();
            trace("avant App::new");
            let mut a = app::App::new(&self.ctx, s);
            trace("App::new (polices, thème)");
            {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(h) = window.window_handle() {
                    if let RawWindowHandle::Win32(w) = h.as_raw() {
                        a.parent_hwnd = Some(w.hwnd.get());
                    }
                }
            }
            if !self.launch.is_empty() {
                // Ouvert depuis l'Explorateur : vue épurée, rien autour du contenu.
                a.panels = false;
                let mut l = std::mem::take(&mut self.launch);
                if self.launch_preview {
                    // Premier lancement en aperçu : Échap masquera la fenêtre (instance gardée)
                    a.open_preview(&self.ctx, l.remove(0), true);
                } else {
                    a.open_paths(&self.ctx, l);
                }
                trace("fichier chargé");
            }
            a.test_env(&self.ctx);
            a.preview_only = self.launch_preview;
            if !discret() {
                if let Some(msg) = socle::crash::pending(APP) {
                    a.toasts.error(msg);
                }
            }
            if !self.launch_preview && !discret() {
                a.auto_update_check();
            }
            if std::env::var("LECTEUR_PANELS").is_ok() {
                a.panels = true;
            }
            self.app = Some(a);
        }
        window.request_redraw();
        self.gfx = Some(Gfx { window, egui, _context: context, surface, renderer });
        if self.standby {
            // Instance de veille : une image dessinée fenêtre cachée prépare polices, atlas et
            // surface, pour que le prochain aperçu s'affiche aussi vite qu'une instance déjà servie.
            self.paint();
            self.paint();
            unsafe {
                use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
                let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
            }
            trace("veille : préchauffée");
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let resp = gfx.egui.on_window_event(&gfx.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                if self.launch_preview {
                    // Instance d'aperçu : fermer = cacher, prête pour l'appui suivant
                    if let Some(a) = self.app.as_mut() {
                        a.close_preview();
                    }
                    gfx.window.request_redraw();
                } else {
                    self.save();
                    el.exit();
                }
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::HoveredFile(_) => {
                if let Some(a) = self.app.as_mut() {
                    a.drop_hover = true;
                }
                gfx.window.request_redraw();
            }
            WindowEvent::HoveredFileCancelled => {
                if let Some(a) = self.app.as_mut() {
                    a.drop_hover = false;
                }
                gfx.window.request_redraw();
            }
            WindowEvent::DroppedFile(p) => {
                if let Some(a) = self.app.as_mut() {
                    a.drop_hover = false;
                    a.open_paths(&self.ctx, vec![p]);
                }
                gfx.window.request_redraw();
            }
            _ => {
                if resp.repaint {
                    gfx.window.request_redraw();
                }
            }
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::RepaintAt(t) => {
                if t <= Instant::now() {
                    if let Some(g) = &self.gfx {
                        g.window.request_redraw();
                    }
                } else {
                    self.next_repaint = Some(self.next_repaint.map_or(t, |n| n.min(t)));
                }
            }
            UserEvent::ClosePreview => {
                if let Some(a) = self.app.as_mut() {
                    a.close_preview();
                }
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
            UserEvent::Media(b) => {
                if let Some(a) = self.app.as_mut() {
                    a.media_button(&self.ctx, b);
                }
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
            UserEvent::Open(mut paths, preview) => {
                trace("tube : message reçu par la boucle");
                let hidden = self.gfx.as_ref().is_some_and(|g| !g.window.is_visible().unwrap_or(true) || g.window.is_minimized().unwrap_or(false));
                if let Some(a) = self.app.as_mut() {
                    if preview && !paths.is_empty() {
                        a.open_preview(&self.ctx, paths.remove(0), hidden);
                    } else {
                        a.open_paths(&self.ctx, paths);
                    }
                }
                trace("tube : fichier chargé");
                self.first_paints = 0;
                self.hidden_since = None;
                if let Some(g) = &self.gfx {
                    if discret() {
                        // Jamais d'activation en test : la fenêtre apparaît invisible, sans voler le focus
                        discret_show(&g.window);
                    } else {
                        g.window.set_visible(true);
                    }
                    if !discret() {
                        g.window.set_minimized(false);
                        g.window.focus_window();
                    }
                    g.window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.app.as_ref().is_some_and(|a| a.want_exit) {
            self.save();
            el.exit();
            return;
        }
        // Aperçu refermé : la fenêtre disparaît mais l'instance reste prête (ouverture suivante instantanée)
        if let Some(a) = self.app.as_mut().filter(|a| a.want_hide) {
            a.want_hide = false;
            let empty = !a.has_tabs();
            if let Some(g) = &self.gfx {
                if empty {
                    g.window.set_visible(false);
                    self.hidden_since = Some(Instant::now());
                } else {
                    g.window.set_minimized(true);
                }
            }
            if empty && self.launch_preview && self.app.as_ref().is_some_and(|a| a.video_used) {
                // Après une vidéo, le pilote graphique reste chargé (≈ 90 Mo que Windows ne rend
                // pas). On passe la main à une instance de veille neuve et légère, puis on quitte.
                if let Ok(exe) = std::env::current_exe() {
                    if std::process::Command::new(exe).arg("--apercu-veille").arg(std::process::id().to_string()).spawn().is_ok() {
                        trace("aperçu recyclé (vidéo vue)");
                        // Fin immédiate, sans démontage (Media Foundation et le pilote mettent 1 à 2 s) :
                        // le verrou d'instance unique est libéré tout de suite pour la relève.
                        // Rien à enregistrer : l'aperçu n'écrit jamais de réglages.
                        unsafe {
                            use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
                            let _ = TerminateProcess(GetCurrentProcess(), 0);
                        }
                        std::process::exit(0);
                    }
                }
            }
            if empty {
                // egui ne libère les textures qu'à l'image suivante : deux images « à vide » une fois
                // cachée, puis on rend la mémoire au système (veille légère).
                self.paint();
                self.paint();
                unsafe {
                    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
                    let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                }
                trace("aperçu caché, mémoire rendue");
            }
            self.save();
        }
        const RESIDENT: std::time::Duration = std::time::Duration::from_secs(600);
        if let Some(h) = self.hidden_since {
            if h.elapsed() >= RESIDENT {
                el.exit();
                return;
            }
            el.set_control_flow(ControlFlow::WaitUntil(self.next_repaint.map_or(h + RESIDENT, |n| n.min(h + RESIDENT))));
            if self.next_repaint.is_none_or(|t| t > Instant::now()) {
                return;
            }
        }
        match self.next_repaint {
            Some(t) if t <= Instant::now() => {
                self.next_repaint = None;
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }
}

fn window_icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../assets/icon64.png")).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// `lecteur-universel --convert <format> <fichiers…>` : conversion sans fenêtre
/// (pour un futur menu « Convertir en… » de l'explorateur).
fn cli_convert(args: &[String]) -> i32 {
    let Some(fmt) = args.first() else { return 2 };
    media::init_thread();
    let mut code = 0;
    for f in &args[1..] {
        let path = PathBuf::from(f);
        let k = kind::of(&path);
        let ext = kind::ext(&path);
        let Some(t) = convert::targets(k, &ext).into_iter().find(|t| t.ext() == fmt.to_lowercase()) else {
            eprintln!("{f} : pas de conversion vers {fmt}");
            code = 1;
            continue;
        };
        let duration = if matches!(k, kind::Kind::Video) { 1e9 } else { 0.0 };
        let src = convert::Source { path, kind: k, image: None, text: None, range: None, at: 0.0, duration };
        let opts = convert::Opts { pdf_scan: std::env::var("LECTEUR_SCAN").is_ok(), ..Default::default() };
        match convert::run(&src, t, &opts, &|_| {}) {
            Ok(v) => v.iter().for_each(|p| println!("{}", p.display())),
            Err(e) => {
                eprintln!("{f} : {e}");
                code = 1;
            }
        }
    }
    code
}

/// Nom de l'app pour le socle : dossier des réglages et du journal de plantage.
pub const APP: &str = "lecteur-universel";

/// Nom d'instance unique : la fenêtre de travail et l'aperçu ont chacun la leur ; en mode discret
/// (tests), des noms factices qui ne touchent jamais au vrai lecteur de l'utilisateur.
fn instance_name(preview: bool) -> String {
    let base = if preview { "LecteurApercu" } else { "Lecteur" };
    if discret() { format!("{base}Test{}", std::env::var("LECTEUR_TUBE").unwrap_or_default().replace(['\\', '/', ' '], "_")) } else { base.into() }
}

/// Affiche une fenêtre de test sans jamais déranger l'utilisateur (`socle::testmode::hide`).
fn discret_show(w: &Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(h) = w.window_handle() {
        if let RawWindowHandle::Win32(h) = h.as_raw() {
            socle::testmode::hide(h.hwnd.get());
        }
    }
}

/// Attend la fin d'un processus (au plus `ms`).
fn wait_process(pid: u32, ms: u32) {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            let _ = WaitForSingleObject(h, ms);
            let _ = windows::Win32::Foundation::CloseHandle(h);
        }
    }
}

fn main() {
    let _ = T0.set(Instant::now());
    socle::crash::install(APP, env!("CARGO_PKG_VERSION"));
    if discret() && !socle::testmode::enabled() {
        // Mode discret : le socle passe aussi en mode test (même chose pour les instances relancées).
        unsafe { std::env::set_var("SOCLE_TEST", "1") };
    }
    trace(&format!("main pid {}", std::process::id()));
    update::cleanup_old();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Relance après mise à jour : on laisse l'ancienne instance se fermer.
    let after_update = args.iter().any(|a| a == "--apres-maj");
    if after_update {
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    if args.first().map(|a| a == "--selftest").unwrap_or(false) && args.len() > 1 {
        media::init_thread();
        let ctx = egui::Context::default();
        let r = app::App::selftest(&ctx, std::path::Path::new(&args[1]));
        let ko = r.iter().filter(|x| !x.1).count();
        for (n, ok, d) in r {
            println!("{} {n} — {d}", if ok { "OK   " } else { "ÉCHEC" });
        }
        std::process::exit(if ko > 0 { 1 } else { 0 });
    }
    // lecteur-universel --cut rapide|precis|720p <fichier> <début s> <fin s> <sortie>
    if args.first().map(|a| a == "--cut").unwrap_or(false) && args.len() >= 6 {
        media::init_thread();
        let (t0, t1): (f64, f64) = (args[3].parse().unwrap_or(0.0), args[4].parse().unwrap_or(10.0));
        let r = match args[1].as_str() {
            "rapide" => montage::remux(&args[2], &args[5], t0, t1, &|_| {}),
            "720p" => montage::transcode(&args[2], &args[5], t0, t1, 720, 4000, &|_| {}),
            _ => montage::transcode(&args[2], &args[5], t0, t1, 0, 8000, &|_| {}),
        };
        println!("{r:?}");
        std::process::exit(if r.is_ok() { 0 } else { 1 });
    }
    // lecteur-universel --generer-tests <dossier> [largeur hauteur ips secondes]
    // --generer-partie <dossier> : seulement la vidéo « partie de jeu » + ses sous-titres (tests L1 / L2)
    if args.first().map(|a| a == "--generer-partie").unwrap_or(false) && args.len() > 1 {
        media::init_thread();
        let (a, b) = testgen::game_sample(std::path::Path::new(&args[1]));
        println!("vidéo : {a:?} · sous-titres : {b:?}");
        return;
    }
    if args.first().map(|a| a == "--generer-tests").unwrap_or(false) && args.len() > 1 {
        media::init_thread();
        let n = |i: usize, d: u32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
        testgen::generate(std::path::Path::new(&args[1]), (n(2, 5120), n(3, 1440), n(4, 60), n(5, 60)), &|l| println!("{l}"));
        return;
    }
    if args.first().map(|a| a == "--thumbs").unwrap_or(false) && args.len() > 1 {
        media::init_thread();
        let t0 = std::time::Instant::now();
        let r = montage::video_thumbs(&args[1], 40, 200, &|i, img| { println!("{i} {}x{} {:?}", img.width(), img.height(), t0.elapsed()); true });
        println!("{r:?} {:?}", t0.elapsed());
        return;
    }
    // lecteur-universel --assembler-pdf sortie.pdf [--scan] image1 image2 … (une page par image)
    if args.first().map(|a| a == "--assembler-pdf").unwrap_or(false) && args.len() > 2 {
        let scan = args.iter().any(|a| a == "--scan");
        let mut pages = Vec::new();
        for f in args[2..].iter().filter(|a| *a != "--scan") {
            match wic::open_any(std::path::Path::new(f)) {
                Ok(i) => pages.push(pdfgen::page_of(&i, scan)),
                Err(e) => eprintln!("{f} : {e}"),
            }
        }
        let r = pdfgen::images_to_pdf(&pages, std::path::Path::new(&args[1]), 85);
        println!("{} pages : {r:?}", pages.len());
        std::process::exit(if r.is_ok() { 0 } else { 1 });
    }
    if args.first().map(|a| a == "--convert").unwrap_or(false) {
        std::process::exit(cli_convert(&args[1..]));
    }
    // Chemins absolus : l'instance qui les reçoit n'a pas le même dossier courant.
    let launch: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).filter(|p| p.exists()).map(|p| std::path::absolute(&p).unwrap_or(p)).collect();
    // --apercu : aperçu rapide (Espace dans l'Explorateur), Échap le referme
    // --fermer-apercu : referme l'aperçu affiché (pour l'explorateur, et pour les tests sans clavier)
    if args.iter().any(|a| a == "--fermer-apercu") {
        let r = socle::instance::start(&instance_name(true), &["--fermer-apercu".to_string()]);
        std::process::exit(if matches!(r, socle::instance::Start::Forwarded) { 0 } else { 1 });
    }
    // --apercu-veille <pid> : instance d'aperçu neuve qui prend le relais de <pid> (recyclage après
    // une vidéo). Elle attend sa fin pour ne pas lui envoyer ses propres arguments.
    let standby = args.iter().any(|a| a == "--apercu-veille");
    if standby {
        if let Some(pid) = args.iter().skip_while(|a| *a != "--apercu-veille").nth(1).and_then(|p| p.parse::<u32>().ok()) {
            wait_process(pid, 3000);
        }
    }
    let launch_preview = (args.iter().any(|a| a == "--apercu") && !launch.is_empty()) || standby;
    // Instance unique (socle : mutex + tube nommé) : un 2ᵉ lancement ajoute un onglet dans la
    // fenêtre existante ; un aperçu va à l'instance d'aperçu, séparée de la fenêtre de travail.
    let test_mode = std::env::var("LECTEUR_SHOT").is_ok() || std::env::var("LECTEUR_SOLO").is_ok();
    let test_mode = test_mode && std::env::var_os("LECTEUR_TUBE").is_none();
    let mut primary = None;
    if !test_mode {
        let mut fwd: Vec<String> = Vec::new();
        if launch_preview && !standby {
            fwd.push("--apercu".into());
        }
        fwd.extend(launch.iter().map(|p| p.to_string_lossy().into_owned()));
        match socle::instance::start(&instance_name(launch_preview), &fwd) {
            socle::instance::Start::Forwarded => return,
            socle::instance::Start::Primary(i) => primary = Some(i),
            socle::instance::Start::Alone => {}
        }
        trace("instance unique");
    }
    trace("avant MFStartup");
    media::init_main();
    trace("MFStartup");
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("boucle d'événements");
    let proxy = event_loop.create_proxy();
    let ctx = egui::Context::default();
    let p2 = proxy.clone();
    ctx.set_request_repaint_callback(move |info| {
        let _ = p2.send_event(UserEvent::RepaintAt(Instant::now() + info.delay));
    });
    let p3 = proxy.clone();
    if let Some(inst) = primary {
        inst.listen(move |args| {
            trace("tube : message lu");
            let close = args.iter().any(|a| a == "--fermer-apercu");
            let preview = launch_preview || args.iter().any(|a| a == "--apercu");
            let paths: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(PathBuf::from).collect();
            let _ = p3.send_event(if close { UserEvent::ClosePreview } else { UserEvent::Open(paths, preview) });
        });
    }
    let mut runner = Runner { ctx, gfx: None, app: None, settings: Some(app::Settings::load()), next_repaint: None, launch, launch_preview, standby, hidden_since: standby.then(Instant::now), proxy: proxy.clone(), smtc: None, smtc_tried: false, smtc_soon: false,
        title: String::new(),
        started: Instant::now(),
        script: std::env::var("LECTEUR_EVENTS")
            .map(|v| v.split(';').filter_map(|e| e.split_once(':').map(|(t, c)| (t.trim().parse().unwrap_or(0), c.trim().to_string()))).collect())
            .unwrap_or_default(),
        script_pos: egui::Pos2::ZERO,
        fps: (Instant::now(), 0, 0),
        first_paints: 0,
        prof: (0.0, 0.0, 0.0),
        eng: (0, 0),
    };
    let _ = event_loop.run_app(&mut runner);
}
