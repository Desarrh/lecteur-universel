//! Dessin de l'interface (design A2) : onglets, vues, frise de montage,
//! barre de lecture, panneau Modifier / Convertir / Infos.
//! Mise en page adaptative : rien ne se chevauche, de 720 px à l'ultra-large.

use super::*;
use crate::icons::{self, Icon, Look};
use egui::text::TextWrapping;
use egui::{Galley, Popup, UiBuilder};

const LIGHT: Color32 = Color32::from_rgb(0xF5, 0xED, 0xDC);
const BAR: Color32 = Color32::from_rgb(0x0F, 0x1B, 0x2D);
const MUTED_DARK: Color32 = Color32::from_rgb(0x9F, 0xB0, 0xC8);
const GOLD: Color32 = Color32::from_rgb(0xF2, 0xB8, 0x4B);
/// En dessous, le panneau de droite se replie (bouton pour le rouvrir).
const NARROW: f32 = 900.0;

fn on_dark() -> Look {
    Look { fg: LIGHT, bg: Color32::TRANSPARENT, hover: Color32::from_white_alpha(26), round: true }
}
fn ghost(p: &Palette) -> Look {
    Look { fg: p.text, bg: Color32::TRANSPARENT, hover: p.hover_bg, round: false }
}

/// Texte sur une seule ligne, coupé avec « … ».
fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping { max_width: max_w.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

fn section(ui: &mut Ui, t: &str, p: &Palette) {
    ui.add_space(10.0);
    ui.label(RichText::new(t.to_uppercase()).size(11.0).strong().color(p.muted));
    ui.add_space(4.0);
}

fn dashed_rect(painter: &egui::Painter, r: Rect, c: Color32) {
    let s = Stroke::new(2.0, c);
    let rr = r.shrink(1.0);
    for (a, b) in [(rr.left_top(), rr.right_top()), (rr.right_top(), rr.right_bottom()), (rr.right_bottom(), rr.left_bottom()), (rr.left_bottom(), rr.left_top())] {
        let len = a.distance(b);
        let dir = (b - a) / len;
        let mut d = 0.0;
        while d < len {
            painter.line_segment([a + dir * d, a + dir * (d + 10.0).min(len)], s);
            d += 17.0;
        }
    }
}

fn sel_range(sel: (Option<f64>, Option<f64>)) -> Option<(f64, f64)> {
    match sel {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        _ => None,
    }
}

enum MediaAct {
    None,
    Capture,
    Cut(bool),
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        self.keys(&ctx);
        self.occl.clear();
        let p = self.pal;
        let win_w = ctx.content_rect().width();
        let narrow = win_w < NARROW;
        if self.panels {
            self.draw_tabs(ui, win_w);
            self.draw_update_bar(ui);
            self.draw_status(ui, win_w);
            let show_rail = !self.tabs.is_empty() && if narrow { self.rail_forced } else { !self.s.hide_rail };
            if show_rail {
                egui::Panel::right("rail")
                    .exact_size(if win_w >= 1300.0 { 320.0 } else { 290.0 })
                    .show_separator_line(false)
                    .frame(Frame::new().fill(p.surface).inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 10 }).stroke(Stroke::new(1.0, p.border)))
                    .show(ui, |ui| self.draw_rail(ui));
            }
        }
        let dark_media = p.dark || self.fullscreen;
        let letterbox = if self.fullscreen { Color32::BLACK } else if p.dark { Color32::from_rgb(0x0B, 0x15, 0x24) } else { p.soft };
        // La vidéo est recopiée sous l'interface : le panneau central reste transparent.
        self.clear = if self.tabs.get(self.active).map(|t| t.kind) == Some(Kind::Video) { letterbox } else { p.bg };
        let bg = match self.tabs.get(self.active).map(|t| t.kind) {
            Some(Kind::Video) => Color32::TRANSPARENT,
            Some(Kind::Pdf) | Some(Kind::Docx) | Some(Kind::Image) => if p.dark { p.code } else { p.soft },
            _ => p.bg,
        };
        egui::CentralPanel::default().frame(Frame::new().fill(bg)).show(ui, |ui| {
            self.view_w = ui.available_width();
            self.view_h = ui.available_height() - 170.0;
            if self.tabs.is_empty() {
                self.draw_home(ui);
            } else {
                self.draw_view(ui, dark_media);
            }
        });
        if !self.panels && !self.tabs.is_empty() {
            self.draw_overlay(&ctx);
        }
        if self.drop_hover {
            let r = ctx.content_rect().shrink(18.0);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(r, CornerRadius::same(14), p.accent.gamma_multiply(0.18));
            painter.rect_stroke(r, CornerRadius::same(14), Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
            painter.text(r.center(), Align2::CENTER_CENTER, "Lâchez pour ouvrir", FontId::proportional(26.0), p.text);
        }
        self.draw_toast(&ctx);
        self.draw_palette(&ctx);
    }

    // ---------- Barre d'onglets ----------

    fn draw_tabs(&mut self, ui: &mut Ui, win_w: f32) {
        let p = self.pal;
        let ctx = ui.ctx().clone();
        let mut activate = None;
        let mut close = None;
        let mut open_dialog = false;
        egui::Panel::top("tabs")
            .exact_size(42.0)
            .show_separator_line(false)
            .frame(Frame::new().fill(p.chrome).inner_margin(Margin { left: 10, right: 6, top: 6, bottom: 0 }))
            .show(ui, |ui| {
                // Positions exactes : logo | onglets (défilables) | commandes. Rien ne se chevauche.
                let bar = ui.max_rect();
                let h = 36.0;
                let y0 = bar.bottom() - h;
                let logo = Rect::from_min_size(pos2(bar.left() + 2.0, y0 + 7.0), vec2(22.0, 22.0));
                icons::paint(ui.painter(), logo, Icon::Logo, p.accent);
                let mut x = logo.right() + 8.0;
                if win_w > 760.0 {
                    let g = ui.painter().layout_no_wrap("Lecteur".into(), FontId::new(14.0, FontFamily::Name("bold".into())), p.accent);
                    let gw = g.size().x;
                    ui.painter().galley(pos2(x, logo.center().y - g.size().y / 2.0), g, p.accent);
                    x += gw + 14.0;
                }
                let right_w = if self.tabs.is_empty() { 2.0 * 34.0 } else { 4.0 * 34.0 };
                let right = Rect::from_min_max(pos2(bar.right() - right_w, y0 + 2.0), pos2(bar.right(), bar.bottom() - 2.0));
                let tabs_r = Rect::from_min_max(pos2(x, y0), pos2(right.left() - 8.0, bar.bottom()));
                let n = self.tabs.len().max(1) as f32;
                let tab_w = ((tabs_r.width() - 40.0) / n).clamp(118.0, 230.0);
                ui.scope_builder(UiBuilder::new().max_rect(tabs_r), |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("tabs_scroll")
                        .max_height(h)
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for (i, t) in self.tabs.iter().enumerate() {
                                    let (rect, r) = ui.allocate_exact_size(vec2(tab_w, h), Sense::click());
                                    let sel = i == self.active;
                                    let fill = if sel { p.surface } else if r.hovered() { p.hover_bg } else { p.chrome };
                                    ui.painter().rect_filled(rect, CornerRadius { nw: 9, ne: 9, sw: 0, se: 0 }, fill);
                                    ui.painter().circle_filled(pos2(rect.left() + 15.0, rect.center().y), 4.0, kind::color(t.kind));
                                    let g = elided(ui, &t.name(), FontId::proportional(13.5), if sel { p.text } else { p.muted }, tab_w - 58.0);
                                    ui.painter().galley(pos2(rect.left() + 27.0, rect.center().y - g.size().y / 2.0), g, p.text);
                                    let xr = Rect::from_center_size(pos2(rect.right() - 17.0, rect.center().y), vec2(22.0, 22.0));
                                    let xresp = ui.interact(xr, egui::Id::unique(("x", t.id)), Sense::click());
                                    if t.dirty() && !xresp.hovered() {
                                        ui.painter().circle_filled(xr.center(), 4.0, p.text);
                                    } else if sel || r.hovered() || xresp.hovered() {
                                        if xresp.hovered() {
                                            ui.painter().rect_filled(xr, CornerRadius::same(6), p.soft);
                                        }
                                        icons::paint(ui.painter(), Rect::from_center_size(xr.center(), Vec2::splat(13.0)), Icon::Close, p.muted);
                                    }
                                    if xresp.clicked() || r.middle_clicked() {
                                        close = Some(i);
                                    } else if r.clicked() {
                                        activate = Some(i);
                                    }
                                    r.on_hover_text(t.path.to_string_lossy());
                                }
                                ui.add_space(4.0);
                                if icons::button(ui, Icon::Plus, 30.0, ghost(&p), "Ouvrir… (Ctrl+O)").clicked() {
                                    open_dialog = true;
                                }
                            });
                        });
                });
                ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let more = icons::button(ui, Icon::More, 32.0, ghost(&p), "Plus d'options et raccourcis");
                    let menu = Popup::menu(&more).show(|ui| {
                        ui.set_min_width(270.0);
                        if ui.button("Ajouter à « Ouvrir avec » de Windows").clicked() {
                            match winx::register_open_with() {
                                Ok(n) => self.say(format!("Inscrit pour {n} extensions : clic droit › Ouvrir avec › Lecteur universel"), false),
                                Err(e) => self.say(e, true),
                            }
                        }
                        if ui.button("Ouvrir un fichier…   Ctrl+O").clicked() {
                            open_dialog = true;
                        }
                        ui.separator();
                        let checking = self.upd_rx.is_some();
                        if ui.add_enabled(!checking, egui::Button::new(if checking { "Vérification…" } else { "Vérifier les mises à jour" })).clicked() {
                            self.check_updates(true);
                        }
                        let mut auto = !self.s.no_auto_update;
                        if ui.checkbox(&mut auto, "Vérifier automatiquement (1 fois par jour)").changed() {
                            self.s.no_auto_update = !auto;
                        }
                        ui.label(RichText::new(format!("Lecteur universel v{} · github.com/{}", env!("CARGO_PKG_VERSION"), crate::update::REPO)).size(11.5).color(p.muted));
                        ui.separator();
                        ui.label(
                            RichText::new(
                                "Tab  vue épurée     F  plein écran\nPgPréc / PgSuiv  fichier voisin\nEspace  lecture     ← →  ±5 s     J  L  ±10 s\n,  .  image par image\nI  O  début / fin de coupe     X  tout garder\nC  capture vidéo\nCtrl+S  copie     Ctrl+Maj+S  remplacer",
                            )
                            .monospace()
                            .size(11.5)
                            .color(p.muted),
                        );
                    });
                    if let Some(m) = menu {
                        self.occl.push(m.response.rect);
                    }
                    let dark = self.s.dark;
                    if icons::button(ui, if dark { Icon::Sun } else { Icon::Moon }, 32.0, ghost(&p), "Thème clair / sombre").clicked() {
                        self.set_theme(&ctx, !dark);
                    }
                    if !self.tabs.is_empty() {
                        if icons::button(ui, Icon::Panel, 32.0, ghost(&p), "Panneau Modifier / Convertir / Infos").clicked() {
                            if win_w < NARROW {
                                self.rail_forced = !self.rail_forced;
                            } else {
                                self.s.hide_rail = !self.s.hide_rail;
                            }
                        }
                        if icons::button(ui, Icon::Clean, 32.0, ghost(&p), "Vue épurée (Tab) — F : plein écran").clicked() {
                            self.panels = false;
                        }
                    }
                });
            });
        if open_dialog {
            let v = winx::open_dialog();
            self.open_paths(&ctx, v);
        }
        if let Some(i) = close {
            self.close(i);
        } else if let Some(i) = activate {
            if i != self.active {
                if let Some(pl) = self.tabs.get_mut(self.active).and_then(|t| t.player()) {
                    if pl.has_video {
                        pl.pause();
                    }
                }
            }
            self.active = i;
            self.target = None;
        }
    }

    // ---------- Bandeau de mise à jour ----------

    fn draw_update_bar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let Some(r) = self.update.clone() else { return };
        if self.upd_hidden {
            return;
        }
        let installing = self.upd_installing;
        let mut install = false;
        let mut later = false;
        egui::Panel::top("update_bar").exact_size(34.0).show_separator_line(false).frame(Frame::new().fill(p.sel_bg).inner_margin(Margin::symmetric(14, 4))).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons::paint(ui.painter(), ir, Icon::Open, p.accent);
                ui.label(RichText::new(format!("Mise à jour v{} disponible", r.version)).color(p.text).strong());
                let first = r.notes.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("");
                if !first.is_empty() {
                    let g = elided(ui, first.trim_start_matches(['-', '*', ' ']), FontId::proportional(12.5), p.muted, (ui.available_width() - 260.0).max(40.0));
                    ui.label(g);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !installing && ui.add(egui::Button::new(RichText::new("Plus tard").color(p.muted)).frame(false)).clicked() {
                        later = true;
                    }
                    let lab = if installing { "Installation…" } else { "Installer et relancer" };
                    if ui.add_enabled(!installing, egui::Button::new(RichText::new(lab).color(p.on_accent).strong()).fill(p.accent)).clicked() {
                        install = true;
                    }
                });
            });
        });
        if later {
            self.upd_hidden = true;
        }
        if install {
            self.install_update();
        }
    }

    // ---------- Barre d'état ----------

    fn draw_status(&mut self, ui: &mut Ui, win_w: f32) {
        let p = self.pal;
        let info = self.tabs.get(self.active).map(|t| {
            let mut parts = vec![kind::label(t.kind).to_string(), human_size(t.size)];
            match &t.view {
                View::Image(v) => {
                    parts.push(format!("{} × {}", v.dims().0, v.dims().1));
                    if v.frames > 1 {
                        parts.push(format!("{} images", v.frames));
                    }
                }
                View::Pdf(v) => parts.push(format!("{} pages", v.pages)),
                View::Docx(b) => parts.push(format!("{} mots", docx::word_count(b))),
                View::Text(v) => {
                    parts.push(format!("{} lignes", v.text.lines().count()));
                    parts.push(if v.lossy { "non UTF-8".into() } else { "UTF-8".into() });
                }
                View::Media(m) => {
                    if let Some(pl) = &m.p {
                        parts.push(hms(pl.duration));
                        if pl.has_video {
                            parts.push(format!("{}×{}", pl.size.0, pl.size.1));
                        }
                    }
                }
                View::Error(_) => {}
            }
            parts.join("  ·  ")
        });
        egui::Panel::bottom("status").exact_size(26.0).show_separator_line(false).frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(14, 4))).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let job = self.job.as_ref().map(|j| (j.label.clone(), f32::from_bits(j.progress.load(Ordering::Relaxed))));
                let right_w = if job.is_some() { 260.0 } else if win_w > 1100.0 { 430.0 } else { 0.0 };
                if self.toasts.current().is_some() {
                    // message éphémère du socle à la place des infos du fichier
                    self.toasts.ui(ui, &p);
                } else {
                    let g = elided(ui, &info.unwrap_or_else(|| "Prêt".into()), FontId::monospace(11.5), p.muted, ui.available_width() - right_w - 10.0);
                    ui.label(g);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some((label, f)) = job {
                        ui.add(egui::ProgressBar::new(f).desired_width(120.0).desired_height(10.0).fill(p.accent));
                        ui.label(RichText::new(format!("{label} {:.0} %", f * 100.0)).size(12.0).color(p.accent));
                    } else if right_w > 0.0 {
                        ui.label(RichText::new("Glisser-déposer · Ctrl+O · Tab vue épurée · F plein écran · ⋯ raccourcis").size(12.0).color(p.muted));
                    }
                });
            });
        });
    }

    // ---------- Accueil ----------

    fn draw_home(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let ctx = ui.ctx().clone();
        let full = ui.available_rect_before_wrap();
        let pad = if full.width() > 900.0 { 40.0 } else { 18.0 };
        let has_recent = self.s.recents.iter().any(|r| Path::new(r).exists());
        let zone_h = if has_recent { (full.height() - 250.0).clamp(200.0, 520.0) } else { full.height() - 2.0 * pad };
        let zone = Rect::from_min_size(full.min + vec2(pad, pad * 0.8), vec2(full.width() - 2.0 * pad, zone_h));
        let r = ui.interact(zone, egui::Id::unique("drop_zone"), Sense::click());
        let hot = r.hovered() || self.drop_hover;
        ui.painter().rect_filled(zone, CornerRadius::same(16), if hot { p.accent.gamma_multiply(0.10) } else { p.surface });
        dashed_rect(ui.painter(), zone, if hot { p.accent } else { p.border });
        let c = zone.center();
        let badge = Rect::from_center_size(c - vec2(0.0, 50.0), vec2(72.0, 72.0));
        ui.painter().rect_filled(badge, CornerRadius::same(20), p.accent);
        icons::paint(ui.painter(), Rect::from_center_size(badge.center(), Vec2::splat(36.0)), Icon::Open, p.on_accent);
        ui.painter().text(c + vec2(0.0, 14.0), Align2::CENTER_CENTER, "Déposez un fichier, ou cliquez pour ouvrir", FontId::proportional(21.0), p.text);
        ui.painter().text(c + vec2(0.0, 44.0), Align2::CENTER_CENTER, "PDF · Word · images · MP3 · WAV · MP4 · texte et code", FontId::proportional(14.0), p.muted);
        if r.clicked() {
            let v = winx::open_dialog();
            self.open_paths(&ctx, v);
        }
        if !has_recent {
            return;
        }
        let mut open = None;
        let area = Rect::from_min_max(pos2(zone.left(), zone.bottom() + 20.0), pos2(zone.right(), full.bottom() - 10.0));
        ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
            ui.label(RichText::new("RÉCENTS").size(11.0).strong().color(p.muted));
            ui.add_space(6.0);
            let recents: Vec<PathBuf> = self.s.recents.iter().map(PathBuf::from).filter(|p| p.exists()).take(8).collect();
            let cols = ((area.width() / 250.0).floor() as usize).clamp(1, 4);
            let w = (area.width() - 10.0 * (cols as f32 - 1.0)) / cols as f32;
            for row in recents.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    for path in row {
                        let k = kind::of(path);
                        let (rect, r) = ui.allocate_exact_size(vec2(w, 52.0), Sense::click());
                        ui.painter().rect(rect, CornerRadius::same(9), if r.hovered() { p.hover_bg } else { p.surface }, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                        let b = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(32.0, 32.0));
                        ui.painter().rect_filled(b, CornerRadius::same(7), kind::color(k));
                        ui.painter().text(b.center(), Align2::CENTER_CENTER, kind::ext(path).to_uppercase(), FontId::monospace(9.5), Color32::WHITE);
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let g = elided(ui, &name, FontId::proportional(13.0), p.text, w - 62.0);
                        ui.painter().galley(pos2(rect.left() + 52.0, rect.top() + 9.0), g, p.text);
                        let pos = self.s.positions.get(&path.to_string_lossy().to_string()).map(|v| if k == Kind::Pdf { format!("page {}", *v as u32 + 1) } else { format!("reprendre à {}", hms(*v)) });
                        let g = elided(ui, &pos.unwrap_or_else(|| kind::label(k).into()), FontId::proportional(11.5), p.muted, w - 62.0);
                        ui.painter().galley(pos2(rect.left() + 52.0, rect.bottom() - 21.0), g, p.muted);
                        if r.clicked() {
                            open = Some(path.clone());
                        }
                        r.on_hover_text(path.to_string_lossy());
                    }
                });
                ui.add_space(8.0);
            }
        });
        if let Some(pth) = open {
            self.open_paths(&ctx, vec![pth]);
        }
    }

    // ---------- Vues ----------

    fn draw_view(&mut self, ui: &mut Ui, dark_media: bool) {
        let p = self.pal;
        let controls = self.panels || self.last_move.elapsed().as_secs_f32() < 2.5;
        let ctx = ui.ctx().clone();
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let kind = t.kind;
        let mut act = MediaAct::None;
        match &mut t.view {
            View::Error(e) => {
                ui.centered_and_justified(|ui| ui.label(RichText::new(e.as_str()).size(16.0).color(p.muted)));
            }
            View::Image(v) => draw_image(ui, v, &p),
            View::Text(v) => draw_text(ui, v, &p, &mut self.s.search_history),
            View::Docx(b) => draw_docx(ui, b, &p),
            View::Pdf(v) => draw_pdf(ui, v, &p, &mut self.s.search_history),
            View::Media(m) => {
                let playing = m.p.as_ref().map(|x| x.playing()).unwrap_or(false);
                let show = controls || !playing || kind == Kind::Audio;
                act = draw_media(ui, m, kind, &mut t.sel, &p, show, dark_media);
                if m.sfind_open {
                    if let Some(r) = subs_find_bar(&ctx, m, &p, &mut self.s.search_history) {
                        self.occl.push(r);
                        // la liste des suggestions passe aussi au-dessus de la vidéo
                        let list = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("subs_find_field").with("liste"));
                        if let Some(lr) = ctx.memory(|mem| mem.areas().is_visible(&list).then(|| mem.area_rect(list.id)).flatten()) {
                            self.occl.push(lr);
                        }
                    }
                }
                if !show {
                    ctx.request_repaint_after(std::time::Duration::from_millis(400));
                }
            }
        }
        match act {
            MediaAct::Capture => self.capture(),
            MediaAct::Cut(precise) => self.cut_media(precise),
            MediaAct::None => {}
        }
    }

    fn draw_overlay(&mut self, ctx: &egui::Context) {
        if self.last_move.elapsed().as_secs_f32() > 2.5 {
            return;
        }
        let Some(t) = self.tabs.get(self.active) else { return };
        let name = t.name();
        let list = kind::neighbours(&t.path);
        let pos = list.iter().position(|q| *q == t.path);
        let r = ctx.content_rect();
        let mut show_panels = false;
        let mut close = false;
        let mut fs = false;
        let mut nb = 0;
        let top = egui::Area::new(egui::Id::unique("overlay_top")).fixed_pos(r.min).order(egui::Order::Foreground).show(ctx, |ui| {
            Frame::new().fill(Color32::from_rgba_unmultiplied(11, 21, 36, 205)).inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
                ui.set_width(r.width() - 24.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if list.len() > 1 {
                        if icons::button(ui, Icon::StepBack, 30.0, on_dark(), "Fichier précédent (PgPréc)").clicked() {
                            nb = -1;
                        }
                    }
                    let g = elided(ui, &name, FontId::proportional(14.0), LIGHT, (r.width() * 0.45).max(80.0));
                    ui.label(g);
                    if let Some(i) = pos.filter(|_| list.len() > 1) {
                        ui.label(RichText::new(format!("{} / {}", i + 1, list.len())).color(MUTED_DARK).size(12.0));
                        if icons::button(ui, Icon::StepFwd, 30.0, on_dark(), "Fichier suivant (PgSuiv)").clicked() {
                            nb = 1;
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if icons::button(ui, Icon::Close, 30.0, on_dark(), "Fermer").clicked() {
                            close = true;
                        }
                        let fi = if self.fullscreen { Icon::Unfullscreen } else { Icon::Fullscreen };
                        if icons::button(ui, fi, 30.0, on_dark(), "Plein écran (F)").clicked() {
                            fs = true;
                        }
                        if icons::button(ui, Icon::Panel, 30.0, on_dark(), "Afficher les panneaux (Tab)").clicked() {
                            show_panels = true;
                        }
                    });
                });
            });
        });
        self.occl.push(top.response.rect);
        if fs {
            self.want_fullscreen = Some(!self.fullscreen);
        }
        if nb != 0 {
            self.neighbour(ctx, nb);
        }
        if show_panels {
            self.panels = true;
            if self.fullscreen {
                self.want_fullscreen = Some(false);
            }
        }
        if close {
            if self.fullscreen {
                self.want_fullscreen = Some(false);
            }
            self.close(self.active);
        }
    }

    /// En vue épurée (pas de barre d'état), le toast du socle s'affiche dans une bulle en bas.
    fn draw_toast(&mut self, ctx: &egui::Context) {
        if self.panels || self.toasts.current().is_none() {
            return;
        }
        let p = self.pal;
        let toast = egui::Area::new(egui::Id::unique("toast")).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -44.0)).order(egui::Order::Tooltip).show(ctx, |ui| {
            Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(14, 8))
                .shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(50) })
                .show(ui, |ui| self.toasts.ui(ui, &p));
        });
        self.occl.push(toast.response.rect);
    }

    // ---------- Panneau de droite ----------

    fn draw_rail(&mut self, ui: &mut Ui) {
        let p = self.pal;
        Frame::new().fill(p.soft).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(3)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let w = (ui.available_width() - 6.0) / 3.0;
            ui.horizontal(|ui| {
                for (r, label) in [(Rail::Edit, "Modifier"), (Rail::Convert, "Convertir"), (Rail::Info, "Infos")] {
                    let sel = self.rail == r;
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
                    if sel {
                        ui.painter().rect_filled(rect, CornerRadius::same(8), p.surface);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(8), p.hover_bg);
                    }
                    let f = if sel { FontId::new(13.5, FontFamily::Name("bold".into())) } else { FontId::proportional(13.5) };
                    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, f, if sel { p.text } else { p.muted });
                    if resp.clicked() {
                        self.rail = r;
                    }
                }
            });
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match self.rail {
                Rail::Edit => self.rail_edit(ui),
                Rail::Convert => self.rail_convert(ui),
                Rail::Info => self.rail_info(ui),
            }
            ui.add_space(12.0);
        });
    }

    fn rail_edit(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let w = ui.available_width();
        let half = (w - 8.0) / 2.0;
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        let kind = t.kind;
        let ext = kind::ext(&t.path);
        let mut action: Option<&'static str> = None;
        let Tab { view, sel, .. } = t;
        match view {
            View::Image(v) => {
                if v.frames > 1 {
                    ui.label(RichText::new("GIF animé : les modifications ne s'appliquent qu'à la première image.").size(12.0).color(p.live));
                }
                section(ui, "Transformer", &p);
                let grid = [
                    (Icon::RotL, "Pivoter ←", "Pivoter à gauche", "Maj+R"),
                    (Icon::RotR, "Pivoter →", "Pivoter à droite", "R"),
                    (Icon::FlipH, "Miroir H", "Miroir horizontal", ""),
                    (Icon::FlipV, "Miroir V", "Miroir vertical", ""),
                ];
                for pair in grid.chunks(2) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (ic, lab, what, key) in pair {
                            let tip = if key.is_empty() { what.to_string() } else { format!("{what} ({key})") };
                            if icons::wide(ui, *ic, lab, "", half, None, &p).on_hover_text(tip).clicked() {
                                action = Some(what);
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                let cl = if v.crop { "Rogner : tracez le cadre" } else { "Rogner" };
                let cr = icons::wide(ui, Icon::Crop, cl, "", w, if v.crop { Some((p.sel_bg, p.text)) } else { None }, &p);
                if cr.clicked() {
                    v.crop = !v.crop;
                    v.crop_rect = None;
                }
                if v.crop {
                    if let Some((a, b)) = v.crop_rect {
                        let r = Rect::from_two_pos(a, b);
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("{:.0} × {:.0} px", r.width(), r.height())).monospace().color(p.muted));
                        if icons::wide(ui, Icon::Check, "Appliquer le rognage", "Entrée", w, Some((p.accent, p.on_accent)), &p).clicked() {
                            action = Some("Rogner");
                        }
                    }
                }
                section(ui, "Taille", &p);
                ui.label(RichText::new(format!("{} × {} px", v.dims().0, v.dims().1)).monospace());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if icons::wide(ui, Icon::Resize, "50 %", "", half, None, &p).clicked() {
                        action = Some("Réduire 50 %");
                    }
                    if icons::wide(ui, Icon::Resize, "1920 px max", "", half, None, &p).clicked() {
                        action = Some("Limiter à 1920 px");
                    }
                });
                if !v.history.is_empty() {
                    section(ui, "Historique", &p);
                    ui.label(RichText::new(format!("Original {} × {}", v.orig.0, v.orig.1)).color(p.muted));
                    for h in &v.history {
                        ui.label(format!("• {h}"));
                    }
                    ui.add_space(4.0);
                    if icons::wide(ui, Icon::Undo, "Annuler", "Ctrl+Z", w, None, &p).clicked() {
                        action = Some("undo");
                    }
                }
                section(ui, "Enregistrer", &p);
                let dirty = !v.history.is_empty();
                ui.add_enabled_ui(dirty, |ui| {
                    if icons::wide(ui, Icon::Save, "Enregistrer une copie", "Ctrl+S", w, Some((p.accent, p.on_accent)), &p).clicked() {
                        action = Some("save_copy");
                    }
                    ui.add_space(4.0);
                    if icons::wide(ui, Icon::Copy, "Remplacer l'original", "Ctrl+Maj+S", w, None, &p).on_hover_text("Une sauvegarde est gardée dans %TEMP%").clicked() {
                        action = Some("save_replace");
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("Les métadonnées (EXIF, GPS) sont retirées à l'enregistrement.").size(12.0).color(p.muted));
            }
            View::Text(v) => {
                section(ui, "Texte", &p);
                ui.checkbox(&mut v.wrap, "Retour à la ligne automatique");
                ui.label(RichText::new("Ctrl+F rechercher · Ctrl+H remplacer · Ctrl+G aller à la ligne").size(12.0).color(p.muted));
                if matches!(v.lang.as_str(), "md" | "markdown") {
                    ui.checkbox(&mut v.ed.preview, "Aperçu Markdown (Ctrl+E)");
                }
                if v.truncated {
                    ui.label(RichText::new("Fichier tronqué à 8 Mo : enregistrement désactivé.").color(p.del));
                }
                if v.lossy {
                    ui.label(RichText::new("Encodage non UTF-8 : certains caractères sont remplacés.").color(p.live));
                }
                section(ui, "Enregistrer", &p);
                let dirty = v.text != v.saved && !v.truncated;
                ui.add_enabled_ui(dirty, |ui| {
                    if icons::wide(ui, Icon::Save, "Enregistrer", "Ctrl+S", w, Some((p.accent, p.on_accent)), &p)
                        .on_hover_text("Écrit le fichier ; l'ancienne version est gardée dans %LOCALAPPDATA%\\lecteur-universel\\sauvegardes")
                        .clicked()
                    {
                        action = Some("save_replace");
                    }
                    ui.add_space(4.0);
                    if icons::wide(ui, Icon::Copy, "Enregistrer une copie", "Ctrl+Maj+S", w, None, &p).clicked() {
                        action = Some("save_copy");
                    }
                    ui.add_space(4.0);
                    if icons::wide(ui, Icon::Reset, "Annuler les modifications", "", w, None, &p).clicked() {
                        v.text = v.saved.clone();
                    }
                });
            }
            View::Media(m) => {
                let (cur, dur) = m.p.as_ref().map(|p| (p.time(), p.duration)).unwrap_or((0.0, 0.0));
                section(ui, "Découpe", &p);
                let (a, b) = (sel.0.unwrap_or(0.0), sel.1.unwrap_or(dur));
                Frame::new().fill(p.code).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (label, val, ic, tip, is_in) in [("Début", a, Icon::MarkIn, "Début ici (I)", true), ("Fin", b, Icon::MarkOut, "Fin ici (O)", false)] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).color(p.muted));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if icons::button(ui, ic, 30.0, Look { fg: p.accent, bg: p.soft, hover: p.hover_bg, round: false }, tip).clicked() {
                                    if is_in {
                                        *sel = (Some(cur), Some(sel.1.filter(|x| *x > cur + 0.05).unwrap_or(dur)));
                                    } else {
                                        *sel = (Some(sel.0.filter(|x| *x < cur - 0.05).unwrap_or(0.0)), Some(cur));
                                    }
                                }
                                ui.label(RichText::new(hms(val)).monospace().size(14.0));
                            });
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Durée").color(p.muted));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if sel_range(*sel).is_some() && icons::button(ui, Icon::Reset, 26.0, ghost(&p), "Tout garder (X)").clicked() {
                                *sel = (None, None);
                            }
                            ui.label(RichText::new(hms((b - a).max(0.0))).monospace().strong());
                        });
                    });
                });
                ui.add_space(6.0);
                ui.label(RichText::new("Rapprochez les poignées de la frise, ou placez-vous image par image puis « Début ici » / « Fin ici ».").size(12.0).color(p.muted));
                ui.add_space(8.0);
                let has = sel_range(*sel).is_some();
                ui.add_enabled_ui(has, |ui| {
                    if kind == Kind::Video {
                        if icons::wide(ui, Icon::Scissors, "Couper — rapide", "sans réencodage", w, Some((p.accent, p.on_accent)), &p)
                            .on_hover_text("Instantané, qualité intacte. Démarre sur l'image clé la plus proche (quelques images plus tôt).")
                            .clicked()
                        {
                            action = Some("cut_fast");
                        }
                        ui.add_space(4.0);
                        if icons::wide(ui, Icon::Scissors, "Couper — à l'image près", "réencode", w, None, &p).on_hover_text("Coupe exacte, réencodée en H.264 (plus long).").clicked() {
                            action = Some("cut_exact");
                        }
                    } else {
                        let k = if ext == "mp3" { "sans réencodage" } else { "WAV" };
                        if icons::wide(ui, Icon::Scissors, "Enregistrer l'extrait", k, w, Some((p.accent, p.on_accent)), &p).clicked() {
                            action = Some("cut_fast");
                        }
                    }
                });
                section(ui, "Lecture", &p);
                ui.checkbox(&mut m.loop_sel, "Boucler sur la sélection");
                if kind == Kind::Video {
                    section(ui, "Sous-titres", &p);
                    match &m.subs {
                        Some((name, cues)) => {
                            let g = elided(ui, &format!("{name} · {} répliques", cues.len()), FontId::proportional(12.5), p.muted, w);
                            ui.label(g);
                            ui.checkbox(&mut m.subs_on, "Afficher   V");
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Décalage").color(p.muted));
                                if ui.small_button("−0,1 s  G").clicked() {
                                    m.sub_offset -= 0.1;
                                }
                                ui.label(RichText::new(format!("{:+.1} s", m.sub_offset).replace('.', ",")).monospace());
                                if ui.small_button("+0,1 s  H").clicked() {
                                    m.sub_offset += 0.1;
                                }
                            });
                        }
                        None => {
                            ui.label(RichText::new("Aucun fichier .srt / .vtt à côté de la vidéo. Glissez-en un sur la vidéo, ou :").size(12.0).color(p.muted));
                        }
                    }
                    ui.add_space(4.0);
                    if icons::wide(ui, Icon::Open, "Charger des sous-titres…", "", w, None, &p).clicked() {
                        action = Some("load_subs");
                    }
                }
                if kind == Kind::Video {
                    section(ui, "Image", &p);
                    if icons::wide(ui, Icon::Camera, "Capturer l'image", "C", w, None, &p).on_hover_text("PNG en pleine définition + copie dans le presse-papiers").clicked() {
                        action = Some("capture");
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if icons::wide(ui, Icon::StepBack, "Image préc.", "", half, None, &p).on_hover_text("Touche ,").clicked() {
                            action = Some("step_back");
                        }
                        if icons::wide(ui, Icon::StepFwd, "Image suiv.", "", half, None, &p).on_hover_text("Touche .").clicked() {
                            action = Some("step_fwd");
                        }
                    });
                }
            }
            View::Pdf(pv) => {
                section(ui, "Texte", &p);
                if icons::wide(ui, Icon::Open, "Rechercher dans le PDF", "Ctrl+F", w, None, &p).clicked() {
                    pv.find_open = true;
                    pv.focus_find = true;
                }
                ui.add_space(4.0);
                if icons::wide(ui, Icon::Copy, &format!("Copier le texte de la page {}", pv.page + 1), "", w, None, &p).clicked() {
                    let txt = match &pv.texts {
                        Some(t) => t.get(pv.page as usize).cloned().unwrap_or_default(),
                        None => crate::pdf::page_texts(&t.path.to_string_lossy()).ok().and_then(|t| t.get(pv.page as usize).cloned()).unwrap_or_default(),
                    };
                    if txt.trim().is_empty() {
                        action = Some("pdf_notext");
                    } else {
                        ui.ctx().copy_text(txt);
                        action = Some("pdf_copied");
                    }
                }
                ui.add_space(6.0);
                ui.label(RichText::new("Pour tout le texte : onglet Convertir › TXT.").size(12.0).color(p.muted));
            }
            View::Docx(_) => {
                ui.add_space(8.0);
                ui.label(RichText::new("Document en lecture seule. Convertissez-le depuis l'onglet Convertir (PDF, texte, Markdown, images…).").color(p.muted));
            }
            View::Error(_) => {}
        }
        match action {
            Some("save_copy") => self.save_current(false),
            Some("save_replace") => self.save_current(true),
            Some("capture") => self.capture(),
            Some("cut_fast") => self.cut_media(false),
            Some("cut_exact") => self.cut_media(true),
            Some("undo") => self.undo_image(),
            Some("load_subs") => {
                if let Some(p) = winx::open_dialog().into_iter().next() {
                    if !self.attach_subs(&p) {
                        self.say("Ouvrez d'abord une vidéo", true);
                    }
                }
            }
            Some("pdf_copied") => self.say("Texte de la page copié", false),
            Some("pdf_notext") => self.say("Pas de texte sur cette page (PDF scanné ?)", true),
            Some("step_back") | Some("step_fwd") => {
                let fwd = action == Some("step_fwd");
                if let Some(pl) = self.tabs.get_mut(self.active).and_then(|t| t.player()) {
                    pl.step(fwd)
                }
            }
            Some(a) => self.edit_image(a),
            None => {}
        }
    }

    fn rail_convert(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let Some(t) = self.tabs.get(self.active) else { return };
        let kind = t.kind;
        let ext = kind::ext(&t.path);
        let targets = convert::targets(kind, &ext);
        if targets.is_empty() {
            ui.label(RichText::new("Aucune conversion pour ce type de fichier.").color(p.muted));
            return;
        }
        if self.target.is_none_or(|x| !targets.contains(&x)) {
            self.target = targets.first().copied();
        }
        let same: Vec<usize> = self.tabs.iter().enumerate().filter(|(_, x)| x.kind == kind).map(|(i, _)| i).collect();
        let range = t.range();
        section(ui, "Vers", &p);
        let w = ui.available_width();
        let cw = (w - 8.0) / 2.0;
        for row in targets.chunks(2) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for tg in row {
                    let sel = self.target == Some(*tg);
                    let (rect, r) = ui.allocate_exact_size(vec2(cw, 52.0), Sense::click());
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(9),
                        if sel { p.sel_bg } else if r.hovered() { p.hover_bg } else { p.code },
                        Stroke::new(if sel { 2.0 } else { 1.0 }, if sel { p.accent } else { p.border }),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(rect.min + vec2(12.0, 17.0), Align2::LEFT_CENTER, tg.label(), FontId::new(14.5, FontFamily::Name("bold".into())), p.text);
                    let g = elided(ui, tg.hint(kind), FontId::proportional(11.5), p.muted, cw - 20.0);
                    ui.painter().galley(pos2(rect.left() + 12.0, rect.top() + 29.0), g, p.muted);
                    if r.clicked() {
                        self.target = Some(*tg);
                    }
                }
            });
            ui.add_space(8.0);
        }
        let Some(tg) = self.target else { return };
        let chips = |ui: &mut Ui, label: &str, cur: &mut u32, vals: &[(u32, &str)]| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(label).color(p.muted));
                for (v, l) in vals {
                    ui.selectable_value(cur, *v, *l);
                }
            });
        };
        let mut has_opts = true;
        match tg {
            Target::Jpg => {
                section(ui, "Options", &p);
                ui.add(egui::Slider::new(&mut self.opts.jpg_quality, 40..=100).text("qualité"));
            }
            Target::Webp => {
                section(ui, "Options", &p);
                ui.add(egui::Slider::new(&mut self.opts.webp_quality, 40..=100).text("qualité"));
                ui.label(RichText::new(if self.opts.webp_quality >= 100 { "100 = sans perte" } else { "avec perte : très léger" }).size(12.0).color(p.muted));
            }
            Target::Mp3 => {
                section(ui, "Options", &p);
                chips(ui, "Débit", &mut self.opts.mp3_kbps, &[(128, "128"), (192, "192"), (320, "320 kbit/s")]);
            }
            Target::Mp4 => {
                section(ui, "Options", &p);
                chips(ui, "Taille", &mut self.opts.mp4_height, &[(0, "native"), (1080, "1080p"), (720, "720p"), (480, "480p")]);
                chips(ui, "Qualité", &mut self.opts.mp4_kbps, &[(2500, "légère"), (6000, "bonne"), (12000, "haute")]);
            }
            Target::Pdf if kind == Kind::Image => {
                section(ui, "Options", &p);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.opts.pdf_scan, false, "Couleur");
                    ui.selectable_value(&mut self.opts.pdf_scan, true, "Document scanné");
                });
                ui.label(
                    RichText::new(if self.opts.pdf_scan {
                        "Fond blanchi, ombres retirées, texte renforcé (noir et blanc) : idéal pour une photo de papier officiel."
                    } else {
                        "Image telle quelle, centrée sur une page A4 avec 1 cm de marge."
                    })
                    .size(12.0)
                    .color(p.muted),
                );
            }
            Target::Gif if kind == Kind::Video => {
                section(ui, "Options", &p);
                chips(ui, "Taille max", &mut self.opts.gif_width, &[(320, "320"), (480, "480"), (720, "720 px")]);
                let mut fps = self.opts.gif_fps as u32;
                chips(ui, "Images/s", &mut fps, &[(10, "10"), (12, "12"), (15, "15"), (24, "24")]);
                self.opts.gif_fps = fps as f64;
            }
            _ => has_opts = false,
        }
        if matches!(kind, Kind::Audio | Kind::Video) && matches!(tg, Target::Mp3 | Target::Wav | Target::Gif | Target::Mp4) {
            if !has_opts {
                section(ui, "Plage", &p);
            }
            let txt = match (range, tg) {
                (Some((a, b)), _) => format!("Sélection : {} → {}", hms(a), hms(b)),
                (None, Target::Gif) => "5 s à partir de la position actuelle (réglez la sélection pour choisir)".into(),
                (None, _) => "Fichier entier (réglez la sélection pour un extrait)".into(),
            };
            ui.add_space(4.0);
            ui.label(RichText::new(txt).size(12.0).color(p.muted));
        }
        if kind == Kind::Pdf {
            ui.label(RichText::new("Les pages sont rendues à ~200 ppp dans un dossier à côté du PDF.").size(12.0).color(p.muted));
        }
        if same.len() > 1 {
            ui.add_space(6.0);
            if tg == Target::Pdf && kind == Kind::Image {
                ui.checkbox(&mut self.batch, format!("Prendre les {} images ouvertes", same.len()));
                if self.batch {
                    ui.checkbox(&mut self.opts.pdf_merge, "Dans un seul PDF (une page par image, ordre des onglets)");
                }
            } else {
                ui.checkbox(&mut self.batch, format!("Appliquer aux {} onglets « {} »", same.len(), kind::label(kind).to_lowercase()));
            }
        }
        ui.add_space(12.0);
        let busy = self.job.is_some();
        if let Some(j) = &self.job {
            let f = f32::from_bits(j.progress.load(Ordering::Relaxed));
            ui.add(egui::ProgressBar::new(f).fill(p.accent).text(format!("{} {:.0} %", j.label, f * 100.0)));
            ui.add_space(6.0);
        }
        ui.add_enabled_ui(!busy, |ui| {
            if icons::wide(ui, Icon::Convert, &format!("Convertir en {}", tg.label()), "", w, Some((p.accent, p.on_accent)), &p).clicked() {
                self.start_convert(tg, if self.batch { same } else { vec![self.active] });
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Enregistré à côté de l'original, sans l'écraser.").size(12.0).color(p.muted));
        self.results(ui);
    }

    /// Fichiers produits : glisser vers l'Explorateur / Discord, ou ouvrir ici.
    fn results(&mut self, ui: &mut Ui) {
        if self.last_out.is_empty() {
            return;
        }
        let p = self.pal;
        let w = ui.available_width();
        section(ui, "Résultat", &p);
        let outs = self.last_out.clone();
        for o in outs.iter().take(6) {
            let name = o.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (rect, r) = ui.allocate_exact_size(vec2(w, 38.0), Sense::click_and_drag());
            ui.painter().rect(rect, CornerRadius::same(8), if r.hovered() { p.add_bg.gamma_multiply(1.1) } else { p.add_bg }, Stroke::new(1.0, p.add.gamma_multiply(0.5)), egui::StrokeKind::Inside);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), Vec2::splat(16.0)), Icon::Drag, p.add);
            let g = elided(ui, &name, FontId::proportional(13.0), p.text, w - 44.0);
            ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - g.size().y / 2.0), g, p.text);
            let r = r.on_hover_text("Glissez vers l'Explorateur, Discord, un mail… — clic : ouvrir ici");
            if r.drag_started() {
                self.drag_out = Some(outs.clone());
            } else if r.clicked() && o.is_file() && kind::supported(o) {
                let ctx = ui.ctx().clone();
                self.open_paths(&ctx, vec![o.clone()]);
                return;
            }
            ui.add_space(4.0);
        }
        if icons::wide(ui, Icon::Folder, "Afficher dans l'Explorateur", "", w, None, &p).clicked() {
            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", outs[0].display())).spawn();
        }
    }

    fn rail_info(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let w = ui.available_width();
        let Some(t) = self.tabs.get(self.active) else { return };
        let mut rows: Vec<(String, String)> = vec![
            ("Nom".into(), t.name()),
            ("Dossier".into(), t.path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()),
            ("Type".into(), format!("{} (.{})", kind::label(t.kind), kind::ext(&t.path))),
            ("Taille".into(), format!("{} ({} octets)", human_size(t.size), t.size)),
        ];
        if let Some(m) = t.modified {
            if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                rows.push(("Modifié".into(), fmt_date(d.as_secs() as i64)));
            }
        }
        match &t.view {
            View::Image(v) => {
                rows.push(("Dimensions".into(), format!("{} × {} px", v.orig.0, v.orig.1)));
                rows.push(("Mégapixels".into(), format!("{:.1}", (v.orig.0 * v.orig.1) as f64 / 1e6)));
                if v.frames > 1 {
                    rows.push(("Animation".into(), format!("{} images", v.frames)));
                }
            }
            View::Pdf(v) => {
                rows.push(("Pages".into(), v.pages.to_string()));
                if let Some((w, h)) = v.sizes.first() {
                    rows.push(("Format".into(), format!("{:.0} × {:.0} mm", w / 72.0 * 25.4, h / 72.0 * 25.4)));
                }
                rows.push(("Rendu".into(), "Windows.Data.Pdf".into()));
            }
            View::Docx(b) => {
                rows.push(("Paragraphes".into(), b.iter().filter(|x| matches!(x, Block::Para { .. })).count().to_string()));
                rows.push(("Tableaux".into(), b.iter().filter(|x| matches!(x, Block::Table(_))).count().to_string()));
                rows.push(("Mots".into(), docx::word_count(b).to_string()));
            }
            View::Text(v) => {
                rows.push(("Lignes".into(), v.text.lines().count().to_string()));
                rows.push(("Caractères".into(), v.text.chars().count().to_string()));
                rows.push(("Encodage".into(), if v.lossy { "non UTF-8".into() } else { "UTF-8".into() }));
            }
            View::Media(m) => {
                if let Some(pl) = &m.p {
                    rows.push(("Durée".into(), hms(pl.duration)));
                    if pl.has_video {
                        rows.push(("Définition".into(), format!("{} × {}", pl.size.0, pl.size.1)));
                    }
                    if pl.duration > 0.0 {
                        rows.push(("Débit moyen".into(), format!("{:.0} kbit/s", t.size as f64 * 8.0 / pl.duration / 1000.0)));
                    }
                }
                rows.push(("Décodage".into(), "Media Foundation".into()));
            }
            View::Error(e) => rows.push(("Erreur".into(), e.clone())),
        }
        ui.add_space(6.0);
        for (k, v) in rows {
            ui.label(RichText::new(k).color(p.muted).size(11.5));
            ui.add(egui::Label::new(RichText::new(v).size(13.0)).wrap());
            ui.add_space(5.0);
        }
        ui.add_space(6.0);
        let path = t.path.clone();
        if icons::wide(ui, Icon::Copy, "Copier le chemin", "", w, None, &p).clicked() {
            ui.ctx().copy_text(path.to_string_lossy().into_owned());
            self.say("Chemin copié", false);
        }
        ui.add_space(4.0);
        if icons::wide(ui, Icon::Folder, "Afficher dans l'Explorateur", "", w, None, &p).clicked() {
            let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
        }
    }
}

// ---------------------------------------------------------------------------
// Vues (fonctions libres pour simplifier les emprunts)

fn draw_image(ui: &mut Ui, v: &mut ImgV, p: &Palette) {
    // GIF animé : une texture par image, créées au premier affichage.
    let animated = v.anim.len() > 1 && v.history.is_empty();
    if animated && v.anim_tex.is_empty() {
        v.anim_tex = v.anim.iter().enumerate().map(|(i, (ci, _))| ui.ctx().load_texture(format!("gif{i}"), ci.clone(), TextureOptions::LINEAR)).collect();
    }
    if v.orig.0 == 0 {
        let r = ui.available_rect_before_wrap();
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "Chargement…", FontId::proportional(15.0), p.muted);
        return;
    }
    if v.dirty || v.tex.is_none() {
        // Copie d'affichage préparée dans le fil (≤ 2560 px) ; après une modification, l'image elle-même
        let ci = match &v.disp {
            Some(ci) => ci.clone(),
            None => ColorImage::from_rgba_unmultiplied([v.img.width() as usize, v.img.height() as usize], v.img.as_raw()),
        };
        v.tex = Some(ui.ctx().load_texture("image", ci, TextureOptions::LINEAR));
        v.dirty = false;
    }
    let tex_id = if animated {
        let total: f32 = v.anim.iter().map(|a| a.1).sum();
        let mut t = v.anim_t0.elapsed().as_secs_f32() % total.max(0.01);
        let mut idx = 0;
        for (i, (_, d)) in v.anim.iter().enumerate() {
            if t < *d {
                idx = i;
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32((d - t).max(0.01)));
                break;
            }
            t -= d;
        }
        v.anim_tex[idx].id()
    } else {
        let Some(tex) = &v.tex else { return };
        tex.id()
    };
    let avail = ui.available_rect_before_wrap();
    let (iw, ih) = (v.dims().0 as f32, v.dims().1 as f32);
    let ppp = ui.ctx().pixels_per_point();
    let fit = ((avail.width() - 40.0) / iw).min((avail.height() - 40.0) / ih).min(2.0 / ppp).max(0.01);
    let scroll = ui.input(|i| if i.modifiers.ctrl { i.smooth_scroll_delta.y } else { 0.0 });
    if scroll != 0.0 && ui.rect_contains_pointer(avail) {
        v.zoom = Some((v.zoom.unwrap_or(fit) * (1.0 + scroll * 0.002)).clamp(0.02, 16.0));
    }
    let scale = v.zoom.unwrap_or(fit);
    let size = vec2(iw * scale, ih * scale);
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        let canvas = vec2(size.x.max(avail.width()), size.y.max(avail.height()));
        let (rect, resp) = ui.allocate_exact_size(canvas, Sense::click_and_drag());
        let img_rect = Rect::from_center_size(rect.center(), size);
        let painter = ui.painter_at(rect);
        // Damier discret pour la transparence
        let cell = 12.0;
        let (c1, c2) = if p.dark { (Color32::from_gray(52), Color32::from_gray(62)) } else { (Color32::from_gray(238), Color32::from_gray(226)) };
        painter.rect_filled(img_rect, CornerRadius::ZERO, c1);
        let vis = img_rect.intersect(ui.clip_rect());
        let (x0, y0) = (((vis.left() - img_rect.left()) / cell).floor() as i32, ((vis.top() - img_rect.top()) / cell).floor() as i32);
        let (x1, y1) = (((vis.right() - img_rect.left()) / cell).ceil() as i32, ((vis.bottom() - img_rect.top()) / cell).ceil() as i32);
        if (x1 - x0) * (y1 - y0) < 20000 {
            for yy in y0..y1 {
                for xx in x0..x1 {
                    if (xx + yy) % 2 == 0 {
                        let r = Rect::from_min_size(img_rect.min + vec2(xx as f32 * cell, yy as f32 * cell), Vec2::splat(cell)).intersect(img_rect);
                        painter.rect_filled(r, CornerRadius::ZERO, c2);
                    }
                }
            }
        }
        painter.image(tex_id, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let to_img = |q: Pos2| pos2(((q.x - img_rect.left()) / scale).clamp(0.0, iw), ((q.y - img_rect.top()) / scale).clamp(0.0, ih));
        let to_scr = |q: Pos2| pos2(img_rect.left() + q.x * scale, img_rect.top() + q.y * scale);
        if v.crop {
            if resp.drag_started() {
                if let Some(q) = resp.interact_pointer_pos() {
                    v.crop_rect = Some((to_img(q), to_img(q)));
                }
            } else if resp.dragged() {
                if let (Some(q), Some((a, _))) = (resp.interact_pointer_pos(), v.crop_rect) {
                    v.crop_rect = Some((a, to_img(q)));
                }
            }
            if let Some((a, b)) = v.crop_rect {
                let r = Rect::from_two_pos(to_scr(a), to_scr(b));
                let dim = Color32::from_rgba_unmultiplied(15, 27, 45, 150);
                painter.rect_filled(Rect::from_min_max(img_rect.min, pos2(img_rect.max.x, r.min.y)), CornerRadius::ZERO, dim);
                painter.rect_filled(Rect::from_min_max(pos2(img_rect.min.x, r.max.y), img_rect.max), CornerRadius::ZERO, dim);
                painter.rect_filled(Rect::from_min_max(pos2(img_rect.min.x, r.min.y), pos2(r.min.x, r.max.y)), CornerRadius::ZERO, dim);
                painter.rect_filled(Rect::from_min_max(pos2(r.max.x, r.min.y), pos2(img_rect.max.x, r.max.y)), CornerRadius::ZERO, dim);
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(2.0, LIGHT), egui::StrokeKind::Outside);
                for i in 1..3 {
                    let fx = r.left() + r.width() * i as f32 / 3.0;
                    let fy = r.top() + r.height() * i as f32 / 3.0;
                    painter.line_segment([pos2(fx, r.top()), pos2(fx, r.bottom())], Stroke::new(1.0, Color32::from_white_alpha(120)));
                    painter.line_segment([pos2(r.left(), fy), pos2(r.right(), fy)], Stroke::new(1.0, Color32::from_white_alpha(120)));
                }
                for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                    painter.rect_filled(Rect::from_center_size(c, Vec2::splat(10.0)), CornerRadius::same(2), LIGHT);
                }
            } else {
                painter.rect_filled(Rect::from_center_size(img_rect.center(), vec2(260.0, 40.0)), CornerRadius::same(20), Color32::from_black_alpha(150));
                painter.text(img_rect.center(), Align2::CENTER_CENTER, "Tracez le cadre à garder", FontId::proportional(15.0), Color32::WHITE);
            }
        } else if resp.double_clicked() {
            v.zoom = if v.zoom.is_some() { None } else { Some(1.0) };
        }
        if !v.crop && resp.dragged() {
            ui.scroll_with_delta(resp.drag_delta());
        }
    });
}

fn draw_text(ui: &mut Ui, v: &mut TxtV, p: &Palette, hist: &mut socle::autocomplete::History) {
    let TxtV { text, lang, wrap, ed, truncated, .. } = v;
    crate::editor::show(ui, text, lang, wrap, ed, *truncated, p, hist);
}

fn draw_docx(ui: &mut Ui, blocks: &[Block], p: &Palette) {
    let paper = if p.dark { Color32::from_rgb(0x1A, 0x2C, 0x49) } else { Color32::WHITE };
    let ink = if p.dark { p.text } else { Color32::from_rgb(0x1B, 0x24, 0x33) };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let w = (ui.available_width() - 48.0).clamp(300.0, 780.0);
            let m = if w < 520.0 { 24 } else { 56 };
            Frame::new().fill(paper).inner_margin(Margin::symmetric(m, 48)).shadow(egui::Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(40) }).show(ui, |ui| {
                ui.set_width(w - 2.0 * m as f32);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    for b in blocks {
                        match b {
                            Block::Para { level, list, runs } => {
                                let size = match level {
                                    1 => 28.0,
                                    2 => 22.0,
                                    3 => 18.0,
                                    4 => 16.0,
                                    _ => 15.0,
                                };
                                let mut job = LayoutJob::default();
                                job.wrap.max_width = ui.available_width();
                                if *list {
                                    job.append("•  ", 0.0, TextFormat::simple(FontId::proportional(size), ink));
                                }
                                for r in runs {
                                    let fam = if r.bold || *level > 0 { FontFamily::Name("bold".into()) } else { FontFamily::Proportional };
                                    let mut f = TextFormat::simple(FontId::new(size, fam), ink);
                                    f.italics = r.italic;
                                    job.append(&r.text, 0.0, f);
                                }
                                if runs.is_empty() {
                                    ui.add_space(8.0);
                                } else {
                                    if *level > 0 {
                                        ui.add_space(10.0);
                                    }
                                    ui.label(job);
                                    ui.add_space(6.0);
                                }
                            }
                            Block::Image(bytes) => {
                                // Texture mise en cache (clé = adresse des octets partagés)
                                let key = egui::Id::unique(("docx_img", std::sync::Arc::as_ptr(bytes) as usize));
                                let tex: Option<Option<TextureHandle>> = ui.ctx().data(|d| d.get_temp(key));
                                let tex = match tex {
                                    Some(t) => t,
                                    None => {
                                        let t = image::load_from_memory(bytes).ok().map(|i| {
                                            let i = i.to_rgba8();
                                            let ci = ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw());
                                            ui.ctx().load_texture("docx_img", ci, TextureOptions::LINEAR)
                                        });
                                        ui.ctx().data_mut(|d| d.insert_temp(key, t.clone()));
                                        t
                                    }
                                };
                                match tex {
                                    Some(t) => {
                                        let s = t.size_vec2();
                                        let w = s.x.min(ui.available_width());
                                        let size = vec2(w, s.y * w / s.x);
                                        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                                        ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                                        ui.add_space(8.0);
                                    }
                                    None => {
                                        ui.label(RichText::new("[image dans un format non pris en charge]").italics().color(p.muted));
                                    }
                                }
                            }
                            Block::Table(rows) => {
                                ui.add_space(6.0);
                                let cols = rows.iter().map(|r| r.len()).max().unwrap_or(1);
                                egui::Grid::new(ui.next_auto_id()).striped(true).num_columns(cols).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
                                    for (i, row) in rows.iter().enumerate() {
                                        for c in row {
                                            let t = RichText::new(c).color(ink).size(13.5);
                                            ui.label(if i == 0 { t.strong() } else { t });
                                        }
                                        ui.end_row();
                                    }
                                });
                                ui.add_space(10.0);
                            }
                        }
                    }
                });
            });
        });
        ui.add_space(24.0);
    });
}

fn draw_pdf(ui: &mut Ui, v: &mut PdfV, p: &Palette, hist: &mut socle::autocomplete::History) {
    if let Some(e) = &v.err {
        if v.pages == 0 {
            ui.centered_and_justified(|ui| ui.label(RichText::new(e.as_str()).color(p.del)));
            return;
        }
    }
    if v.pages == 0 {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Ouverture du PDF…").color(p.muted)));
        return;
    }
    // Recherche dans le texte (Ctrl+F)
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, Key::F)) {
        v.find_open = true;
        v.focus_find = true;
    }
    if v.find_open {
        let hits: Vec<(u32, usize)> = match &v.texts {
            Some(t) if !v.find.trim().is_empty() => {
                let n = v.find.to_lowercase();
                t.iter().enumerate().filter_map(|(i, s)| {
                    let c = s.to_lowercase().matches(n.as_str()).count();
                    (c > 0).then_some((i as u32, c))
                }).collect()
            }
            _ => Vec::new(),
        };
        if v.find_hit >= hits.len() {
            v.find_hit = 0;
        }
        Frame::new().fill(p.surface).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, p.border)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Mots du PDF proposés dès qu'ils sont extraits (index construit dans un fil)
                if let Some(t) = v.texts.as_ref().filter(|_| !v.sac.is_built()) {
                    v.sac.index(&t.join("\n"), hist);
                }
                let ar = v.sac.field(ui, p, egui::Id::unique("pdf_find"), &mut v.find, "Rechercher dans le PDF", 260.0, hist);
                let r = ar.response.clone();
                if v.focus_find {
                    r.request_focus();
                    v.focus_find = false;
                }
                let lk = Look { fg: p.text, bg: Color32::TRANSPARENT, hover: p.hover_bg, round: false };
                let mut go = 0i32;
                if r.changed() || ar.completed {
                    v.find_hit = 0;
                    go = 2;
                }
                if ar.submitted.is_some() {
                    go = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                    v.focus_find = true;
                }
                let status = match (&v.texts, hits.is_empty()) {
                    (None, _) => "extraction du texte…".to_string(),
                    (Some(t), _) if t.iter().all(|s| s.trim().is_empty()) => "pas de texte (PDF scanné)".into(),
                    (_, true) if !v.find.trim().is_empty() => "aucun résultat".into(),
                    (_, true) => String::new(),
                    (_, false) => format!("page {} · {} / {} pages · {} occurrences", hits[v.find_hit].0 + 1, v.find_hit + 1, hits.len(), hits.iter().map(|h| h.1).sum::<usize>()),
                };
                if icons::button(ui, Icon::StepBack, 28.0, lk, "Page précédente avec le mot").clicked() {
                    go = -1;
                }
                if icons::button(ui, Icon::StepFwd, 28.0, lk, "Page suivante avec le mot (Entrée)").clicked() {
                    go = 1;
                }
                ui.label(RichText::new(status).size(12.0).color(p.muted));
                if !hits.is_empty() && go != 0 {
                    if go != 2 {
                        v.find_hit = (v.find_hit as i64 + go as i64).rem_euclid(hits.len() as i64) as usize;
                    }
                    v.scroll_to = Some(hits[v.find_hit].0);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icons::button(ui, Icon::Close, 28.0, lk, "Fermer (Échap)").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        v.find_open = false;
                    }
                });
            });
            // Extrait autour de l'occurrence
            if let (Some(t), Some((pg, _))) = (&v.texts, hits.get(v.find_hit)) {
                let s = &t[*pg as usize];
                let low = s.to_lowercase();
                if let Some(b) = low.find(&v.find.to_lowercase()) {
                    let start = s.char_indices().map(|(i, _)| i).filter(|i| *i <= b).rev().nth(60).unwrap_or(0);
                    let end = s.char_indices().map(|(i, _)| i).find(|i| *i >= b + v.find.len() + 80).unwrap_or(s.len());
                    let snippet = s.get(start..end).unwrap_or("").replace('\n', " ");
                    ui.label(RichText::new(format!("… {snippet} …")).size(12.0).italics().color(p.muted));
                }
            }
        });
    }
    let scroll = ui.input(|i| if i.modifiers.ctrl { i.smooth_scroll_delta.y } else { 0.0 });
    if scroll != 0.0 {
        v.zoom = (v.zoom * (1.0 + scroll * 0.002)).clamp(0.3, 4.0);
    }
    let ppp = ui.ctx().pixels_per_point();
    let area = ui.available_rect_before_wrap();
    let base = (area.width() - 48.0).min(900.0);
    let w = base * v.zoom;
    let mut first_visible = None;
    let scroll_to = v.scroll_to.take();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            for i in 0..v.pages {
                let (pw, ph) = v.sizes.get(i as usize).copied().unwrap_or((612.0, 792.0));
                let h = w * ph / pw;
                let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                if scroll_to == Some(i) {
                    ui.scroll_to_rect(rect, Some(Align::TOP));
                }
                if ui.is_rect_visible(rect) {
                    if first_visible.is_none() && rect.bottom() > area.top() + area.height() * 0.4 {
                        first_visible = Some(i);
                    }
                    ui.painter().rect_filled(rect.translate(vec2(0.0, 2.0)).expand(1.0), CornerRadius::same(2), Color32::from_black_alpha(40));
                    ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);
                    let want = (((w * ppp) / 256.0).ceil() as u32 * 256).clamp(256, 4096);
                    match v.tex.get(&i) {
                        Some((tw, tex)) => {
                            ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            if *tw < want && !v.pending.contains(&(i, want)) {
                                v.pending.insert((i, want));
                                let _ = v.w.req.send((i, want));
                            }
                        }
                        None => {
                            ui.painter().text(rect.center(), Align2::CENTER_CENTER, format!("page {}", i + 1), FontId::proportional(14.0), Color32::GRAY);
                            if !v.pending.contains(&(i, want)) {
                                v.pending.insert((i, want));
                                let _ = v.w.req.send((i, want));
                            }
                        }
                    }
                }
                ui.add_space(16.0);
            }
            ui.add_space(50.0);
        });
    });
    if let Some(i) = first_visible {
        v.page = i;
    }
    // Pilule de navigation (icônes centrées)
    let pill = Rect::from_center_size(pos2(area.center().x, area.bottom() - 32.0), vec2(262.0, 44.0));
    ui.painter().rect(pill, CornerRadius::same(22), p.surface, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    ui.scope_builder(UiBuilder::new().max_rect(pill.shrink2(vec2(8.0, 6.0))).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let lk = Look { fg: p.text, bg: Color32::TRANSPARENT, hover: p.hover_bg, round: true };
        if icons::button(ui, Icon::StepBack, 32.0, lk, "Page précédente (←)").clicked() {
            v.scroll_to = Some(v.page.saturating_sub(1));
        }
        let (r, _) = ui.allocate_exact_size(vec2(62.0, 32.0), Sense::hover());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{} / {}", v.page + 1, v.pages), FontId::monospace(12.5), p.text);
        if icons::button(ui, Icon::StepFwd, 32.0, lk, "Page suivante (→)").clicked() {
            v.scroll_to = Some((v.page + 1).min(v.pages - 1));
        }
        let (sr, _) = ui.allocate_exact_size(vec2(9.0, 24.0), Sense::hover());
        ui.painter().line_segment([sr.center_top(), sr.center_bottom()], Stroke::new(1.0, p.border));
        if ui.add(egui::Button::new(RichText::new("−").size(16.0)).frame(false).min_size(vec2(26.0, 30.0))).on_hover_text("Zoom −").clicked() {
            v.zoom = (v.zoom / 1.2).max(0.3);
        }
        let (r, resp) = ui.allocate_exact_size(vec2(44.0, 32.0), Sense::click());
        ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{:.0} %", v.zoom * 100.0), FontId::monospace(12.0), p.text);
        if resp.on_hover_text("Clic : largeur").clicked() {
            v.zoom = 1.0;
        }
        if ui.add(egui::Button::new(RichText::new("+").size(16.0)).frame(false).min_size(vec2(26.0, 30.0))).on_hover_text("Zoom +").clicked() {
            v.zoom = (v.zoom * 1.2).min(4.0);
        }
    });
}

// ---------------------------------------------------------------------------
// Audio / vidéo : vue, frise de montage, barre de lecture

enum Strip<'a> {
    Thumbs(&'a [Option<TextureHandle>]),
    Wave(Option<&'a [f32]>),
}

#[allow(clippy::too_many_arguments)]
/// L1 : barre « chercher une phrase » posée en haut de la vidéo. Entrée = réplique suivante,
/// Maj+Entrée = précédente, Échap ferme. Renvoie sa zone (à découper de la vidéo).
fn subs_find_bar(ctx: &egui::Context, m: &mut MedV, p: &Palette, hist: &mut socle::autocomplete::History) -> Option<Rect> {
    let cues = &m.subs.as_ref()?.1;
    if !m.sac.is_built() {
        m.sac.index(&crate::subs::all_text(cues), hist);
    }
    if m.sfind_last != m.sfind {
        m.sfind_hits = crate::subs::find(cues, &m.sfind);
        m.sfind_last = m.sfind.clone();
    }
    let top = m.vrect.map(|r| r.top()).unwrap_or(60.0).max(8.0) + 12.0;
    let mut go = 0i32;
    let mut close = false;
    let area = egui::Area::new(egui::Id::unique("subs_find")).order(egui::Order::Foreground).anchor(Align2::CENTER_TOP, vec2(0.0, top)).show(ctx, |ui| {
        Frame::new()
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(10, 6))
            .shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(60) })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let ar = m.sac.field(ui, p, egui::Id::unique("subs_find_field"), &mut m.sfind, "Chercher une phrase dite…", 280.0, hist);
                    if m.sfind_focus {
                        ar.response.request_focus();
                        m.sfind_focus = false;
                    }
                    if ar.submitted.is_some() {
                        go = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                        m.sfind_focus = true;
                    }
                    if ar.response.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    let n = m.sfind_hits.len();
                    let label = if m.sfind.trim().is_empty() { String::new() } else if n == 0 { "aucune réplique".into() } else { format!("{n} réplique{}", if n > 1 { "s" } else { "" }) };
                    ui.label(RichText::new(label).size(12.0).color(if n == 0 && !m.sfind.trim().is_empty() { p.del } else { p.muted }));
                    let lk = Look { fg: p.text, bg: Color32::TRANSPARENT, hover: p.hover_bg, round: false };
                    if icons::button(ui, Icon::StepBack, 28.0, lk, "Réplique précédente (Maj+Entrée)").clicked() {
                        go = -1;
                    }
                    if icons::button(ui, Icon::StepFwd, 28.0, lk, "Réplique suivante (Entrée)").clicked() {
                        go = 1;
                    }
                    if icons::button(ui, Icon::Close, 28.0, lk, "Fermer (Échap)").clicked() {
                        close = true;
                    }
                });
            });
    });
    if go != 0 {
        if let Some(pl) = m.p.as_mut() {
            let now = pl.time() - m.sub_offset;
            let hits = &m.sfind_hits;
            let to = if go > 0 { hits.iter().find(|&&t| t > now + 0.3).or(hits.first()) } else { hits.iter().rev().find(|&&t| t < now - 0.8).or(hits.last()) };
            if let Some(&t) = to {
                pl.seek((t + m.sub_offset - 0.2).max(0.0));
            }
        }
    }
    if close {
        m.sfind_open = false;
    }
    Some(area.response.rect)
}

fn draw_media(ui: &mut Ui, m: &mut MedV, kind: Kind, sel: &mut (Option<f64>, Option<f64>), p: &Palette, controls: bool, dark: bool) -> MediaAct {
    m.vrect = None;
    if let Some(e) = &m.err {
        ui.centered_and_justified(|ui| ui.label(RichText::new(format!("Lecture impossible : {e}")).color(p.del)));
        return MediaAct::None;
    }
    if m.p.is_none() {
        return MediaAct::None;
    }
    let full = ui.available_rect_before_wrap();
    let narrow = full.width() < 640.0;
    let mut act = MediaAct::None;
    // Repères de la frise : points forts (L2) et phrases trouvées dans les sous-titres (L1)
    let mut marks: Vec<Mark> = m.moments.iter().flatten().map(|x| (x.t, p.live, format!("Point fort à {} · +{:.0} dB   (N suivant · E exporter ±10 s)", hms(x.t), x.gain))).collect();
    if m.sfind_open {
        marks.extend(m.sfind_hits.iter().map(|t| (*t + m.sub_offset, p.accent, format!("« {} » à {}", m.sfind.trim(), hms(*t + m.sub_offset)))));
    }
    // Boucle sur la sélection
    if let (Some(pl), true) = (m.p.as_mut(), m.loop_sel) {
        if let (Some(a), Some(b)) = *sel {
            let t = pl.time();
            if pl.playing() && b > a && t >= b && t < b + 0.6 {
                pl.seek(a);
            }
        }
    }
    if kind == Kind::Video {
        let ctrl_h = if controls { 166.0 } else { 0.0 };
        let area = Rect::from_min_max(full.min, pos2(full.max.x, full.max.y - ctrl_h));
        let r = ui.interact(area, egui::Id::unique("video_area"), Sense::click());
        let pl = m.p.as_mut().unwrap();
        let fg = if dark { LIGHT } else { p.text };
        if pl.size.0 > 0 {
            // Rectangle d'affichage (au pixel près) ; l'image y est recopiée directement par la fenêtre.
            let ppp = ui.ctx().pixels_per_point();
            let ns = vec2(pl.size.0 as f32, pl.size.1 as f32);
            let s = (area.width() / ns.x).min(area.height() / ns.y);
            let rect = Rect::from_center_size(area.center(), ns * s);
            let (w, h) = ((rect.width() * ppp).round() as u32, (rect.height() * ppp).round() as u32);
            let k = ((3840.0 * 2160.0) / (w.max(1) * h.max(1)) as f32).sqrt().min(1.0);
            m.want = (((w as f32 * k) as u32) & !1, ((h as f32 * k) as u32) & !1);
            m.vrect = Some(rect);
        }
        if m.frame.is_none() && !pl.hw_render {
            ui.painter().text(area.center(), Align2::CENTER_CENTER, "Chargement…", FontId::proportional(16.0), fg);
        }
        // Sous-titres (zone découpée de la vidéo en affichage direct pour rester visible)
        m.sub_rect = None;
        if let (true, Some((_, cues))) = (m.subs_on, &m.subs) {
            if let Some(txt) = crate::subs::at(cues, pl.time() - m.sub_offset) {
                let vr = m.vrect.unwrap_or(area);
                let size = (vr.height() * 0.05).clamp(15.0, 42.0);
                let mut job = LayoutJob::simple(txt.to_string(), FontId::new(size, FontFamily::Name("bold".into())), Color32::WHITE, vr.width() * 0.8);
                job.halign = Align::Center;
                let g = ui.painter().layout_job(job);
                let gs = g.size();
                let bx = Rect::from_center_size(pos2(vr.center().x, vr.bottom() - vr.height() * 0.07 - gs.y / 2.0), gs + vec2(28.0, 14.0));
                ui.painter().rect_filled(bx, CornerRadius::same(8), if pl.hw_render { Color32::from_rgb(18, 18, 18) } else { Color32::from_black_alpha(185) });
                ui.painter().galley(pos2(bx.center().x, bx.top() + 7.0), g, Color32::WHITE);
                m.sub_rect = Some(bx);
            }
        }
        if r.clicked() {
            pl.toggle();
        }
        if r.double_clicked() {
            pl.toggle();
            ui.ctx().input_mut(|i| i.events.push(egui::Event::Key { key: Key::F11, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }));
        }
        // Gros bouton lecture au centre quand c'est en pause
        if !pl.playing() && m.frame.is_some() {
            let c = Rect::from_center_size(area.center(), Vec2::splat(76.0));
            ui.painter().circle_filled(c.center(), 38.0, Color32::from_black_alpha(120));
            icons::paint(ui.painter(), Rect::from_center_size(c.center() + vec2(3.0, 0.0), Vec2::splat(38.0)), Icon::Play, LIGHT);
        }
        if !controls {
            return MediaAct::None;
        }
        let bar = Rect::from_min_max(pos2(full.min.x, full.max.y - ctrl_h), full.max);
        ui.painter().rect_filled(bar, CornerRadius::ZERO, if dark { BAR } else { p.surface });
        if !dark {
            ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, p.border));
        }
        let inner = bar.shrink2(vec2(if narrow { 12.0 } else { 24.0 }, 10.0));
        let row1 = Rect::from_min_size(inner.min, vec2(inner.width(), 30.0));
        let strip = Rect::from_min_size(pos2(inner.left(), row1.bottom() + 8.0), vec2(inner.width(), 62.0));
        let row3 = Rect::from_min_max(pos2(inner.left(), strip.bottom() + 10.0), inner.max);
        act = time_row(ui, row1, pl, sel, kind, dark, p);
        trim_strip(ui, strip, pl, sel, Strip::Thumbs(&m.thumbs), p, dark, &marks);
        if let Some(a) = transport(ui, row3, pl, &mut m.loop_sel, kind, p, dark, narrow) {
            act = a;
        }
        return act;
    }
    // Audio : grande lecture, forme d'onde + poignées, barre centrée
    let pl = m.p.as_mut().unwrap();
    let pad = if narrow { 16.0 } else { 40.0 };
    let inner = full.shrink2(vec2(pad, 24.0));
    let head = Rect::from_min_size(inner.min, vec2(inner.width(), 110.0));
    let art = Rect::from_min_size(head.min, vec2(104.0, 104.0));
    ui.painter().rect_filled(art, CornerRadius::same(14), BAR);
    icons::paint(ui.painter(), Rect::from_center_size(art.center(), Vec2::splat(48.0)), Icon::Volume, Color32::from_rgb(0x6F, 0xA8, 0xFF));
    ui.painter().text(pos2(art.right() + 22.0, art.top() + 40.0), Align2::LEFT_CENTER, hms(pl.time()), FontId::monospace(38.0), p.accent);
    ui.painter().text(pos2(art.right() + 24.0, art.top() + 80.0), Align2::LEFT_CENTER, format!("/ {}", hms(pl.duration)), FontId::monospace(16.0), p.muted);
    let wave_h = (inner.height() - 110.0 - 30.0 - 50.0 - 60.0).clamp(90.0, 240.0);
    let row1 = Rect::from_min_size(pos2(inner.left(), head.bottom() + 14.0), vec2(inner.width(), 30.0));
    let strip = Rect::from_min_size(pos2(inner.left(), row1.bottom() + 8.0), vec2(inner.width(), wave_h));
    let row3 = Rect::from_min_size(pos2(inner.left(), strip.bottom() + 16.0), vec2(inner.width(), 48.0));
    act = time_row(ui, row1, pl, sel, kind, p.dark, p);
    trim_strip(ui, strip, pl, sel, Strip::Wave(m.peaks.as_deref()), p, p.dark, &marks);
    if let Some(a) = transport(ui, row3, pl, &mut m.loop_sel, kind, p, false, narrow) {
        act = a;
    }
    let tip = Rect::from_min_size(pos2(inner.left(), row3.bottom() + 10.0), vec2(inner.width(), 20.0));
    if tip.bottom() < full.bottom() {
        ui.painter().text(
            tip.left_center(),
            Align2::LEFT_CENTER,
            "Glissez les poignées pour couper · I / O : début / fin ici · X : tout garder · Espace : lecture",
            FontId::proportional(12.0),
            p.muted,
        );
    }
    act
}

/// Ligne au-dessus de la frise : temps centré dans une pilule, sélection, export rapide.
fn time_row(ui: &mut Ui, r: Rect, pl: &mut Player, sel: &mut (Option<f64>, Option<f64>), kind: Kind, dark: bool, p: &Palette) -> MediaAct {
    let mut act = MediaAct::None;
    let range = match *sel {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        _ => None,
    };
    let txt = format!("{}  /  {}", hms(pl.time()), hms(pl.duration));
    let font = FontId::monospace(13.0);
    let tw = ui.painter().layout_no_wrap(txt.clone(), font.clone(), LIGHT).size().x;
    let pill = Rect::from_center_size(r.center(), vec2(tw + 28.0, 28.0));
    let (bg, fg) = if dark { (Color32::from_rgb(0x1C, 0x2D, 0x48), LIGHT) } else { (p.soft, p.text) };
    ui.painter().rect_filled(pill, CornerRadius::same(14), bg);
    ui.painter().text(pill.center(), Align2::CENTER_CENTER, txt, font, fg);
    if let Some((a, b)) = range {
        let lab = format!("✂  {}", hms(b - a));
        let left = Rect::from_min_max(r.min, pos2(pill.left() - 10.0, r.max.y));
        let gc = if dark { GOLD } else { p.live };
        let g = ui.painter().layout_no_wrap(lab, FontId::proportional(13.0), gc);
        if left.width() > g.size().x + 10.0 {
            ui.painter().galley(pos2(left.right() - g.size().x, r.center().y - g.size().y / 2.0), g, gc);
        }
        // Export rapide à droite
        let right = Rect::from_min_max(pos2(pill.right() + 10.0, r.min.y), r.max);
        let label = if kind == Kind::Video { "Couper" } else { "Extraire" };
        let bw = 104.0;
        if right.width() > bw + 40.0 {
            let br = Rect::from_min_size(pos2(right.left(), r.center().y - 14.0), vec2(bw, 28.0));
            let resp = ui.interact(br, egui::Id::unique("quick_cut"), Sense::click());
            ui.painter().rect_filled(br, CornerRadius::same(14), if resp.hovered() { GOLD } else { GOLD.gamma_multiply(0.85) });
            icons::paint(ui.painter(), Rect::from_center_size(pos2(br.left() + 18.0, br.center().y), Vec2::splat(15.0)), Icon::Scissors, BAR);
            ui.painter().text(pos2(br.left() + 32.0, br.center().y), Align2::LEFT_CENTER, label, FontId::new(13.0, FontFamily::Name("bold".into())), BAR);
            if resp.on_hover_text("Exporter la sélection (sans réencodage)").clicked() {
                act = MediaAct::Cut(false);
            }
            let xr = Rect::from_min_size(pos2(br.right() + 6.0, br.top()), vec2(28.0, 28.0));
            let xresp = ui.interact(xr, egui::Id::unique("sel_clear"), Sense::click());
            if xresp.hovered() {
                ui.painter().circle_filled(xr.center(), 14.0, Color32::from_white_alpha(26));
            }
            icons::paint(ui.painter(), Rect::from_center_size(xr.center(), Vec2::splat(12.0)), Icon::Close, if dark { MUTED_DARK } else { Color32::GRAY });
            if xresp.on_hover_text("Tout garder (X)").clicked() {
                *sel = (None, None);
            }
        }
    }
    act
}

/// Frise façon éditeur mobile : vignettes (ou forme d'onde), cadre à poignées
/// qu'on rapproche pour couper, tête de lecture. Glisser ailleurs = se déplacer.
/// Repère sur la frise : instant, couleur, infobulle (points forts, résultats de recherche).
type Mark = (f64, Color32, String);

#[allow(clippy::too_many_arguments)]
fn trim_strip(ui: &mut Ui, r: Rect, pl: &mut Player, sel: &mut (Option<f64>, Option<f64>), content: Strip, p: &Palette, dark: bool, marks: &[Mark]) {
    const HW: f32 = 16.0;
    let dur = pl.duration.max(0.001);
    let id = egui::Id::unique("trim_strip");
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let inner = r.shrink2(vec2(HW, 4.0));
    let x_of = |t: f64| inner.left() + (t / dur).clamp(0.0, 1.0) as f32 * inner.width();
    let t_of = |x: f32| (((x - inner.left()) / inner.width()).clamp(0.0, 1.0) as f64) * dur;
    let (a, b) = (sel.0.unwrap_or(0.0), sel.1.unwrap_or(dur));
    let frame_c = if dark { LIGHT } else { p.text };
    let track = if dark { Color32::from_rgb(0x1C, 0x2D, 0x48) } else { p.soft };
    ui.painter().rect_filled(inner.expand2(vec2(0.0, 0.0)), CornerRadius::same(6), track);
    // Contenu
    let painter = ui.painter_at(inner);
    match content {
        Strip::Thumbs(th) => {
            let n = th.len();
            let aspect = th.iter().flatten().next().map(|t| t.size_vec2().x / t.size_vec2().y).unwrap_or(16.0 / 9.0);
            // Tuiles presque carrées (recadrage au centre), comme sur un téléphone.
            let tile_aspect = aspect.min(1.25);
            let tile_w = (inner.height() * tile_aspect).max(24.0);
            let u = (tile_aspect / aspect).min(1.0);
            let uv = Rect::from_min_max(pos2(0.5 - u / 2.0, 0.0), pos2(0.5 + u / 2.0, 1.0));
            let tiles = (inner.width() / tile_w).ceil().max(1.0) as usize;
            for k in 0..tiles {
                let x0 = inner.left() + k as f32 * tile_w;
                let tr = Rect::from_min_size(pos2(x0, inner.top()), vec2(tile_w, inner.height()));
                let tc = ((x0 + tile_w / 2.0 - inner.left()) / inner.width()).clamp(0.0, 0.999);
                let idx = (tc * n as f32) as usize;
                // vignette dispo la plus proche
                let tex = (0..n).map(|d| [idx.saturating_sub(d), (idx + d).min(n.saturating_sub(1))]).flatten().find_map(|i| th.get(i).and_then(|t| t.as_ref()));
                if let Some(t) = tex {
                    painter.image(t.id(), tr.shrink2(vec2(0.5, 0.0)), uv, Color32::WHITE);
                } else {
                    painter.rect_filled(tr.shrink(1.0), CornerRadius::same(2), Color32::from_rgb(0x24, 0x37, 0x56));
                }
            }
        }
        Strip::Wave(Some(pk)) => {
            let n = pk.len().max(1);
            let bw = inner.width() / n as f32;
            let px = x_of(pl.time());
            let (played, rest) = if dark { (Color32::from_rgb(0x6F, 0xA8, 0xFF), Color32::from_rgb(0x2A, 0x3F, 0x66)) } else { (p.accent, p.border) };
            for (i, v) in pk.iter().enumerate() {
                let x = inner.left() + i as f32 * bw;
                let hh = (v.sqrt() * inner.height() * 0.92).max(2.0);
                let c = if x < px { played } else { rest };
                painter.rect_filled(Rect::from_center_size(pos2(x + bw / 2.0, inner.center().y), vec2((bw - 1.0).max(1.0), hh)), CornerRadius::ZERO, c);
            }
        }
        Strip::Wave(None) => {
            painter.text(inner.center(), Align2::CENTER_CENTER, "Analyse de la forme d'onde…", FontId::proportional(13.0), p.muted);
        }
    }
    // Zones hors sélection assombries
    let (xa, xb) = (x_of(a), x_of(b));
    let dim = Color32::from_black_alpha(if dark { 160 } else { 110 });
    if xa > inner.left() {
        painter.rect_filled(Rect::from_min_max(inner.min, pos2(xa, inner.max.y)), CornerRadius::ZERO, dim);
    }
    if xb < inner.right() {
        painter.rect_filled(Rect::from_min_max(pos2(xb, inner.min.y), inner.max), CornerRadius::ZERO, dim);
    }
    // Cadre + poignées
    let frame = Rect::from_min_max(pos2(xa - HW, r.top()), pos2(xb + HW, r.bottom()));
    let trimmed = sel.0.is_some() || sel.1.is_some();
    let fc = if trimmed { GOLD } else { frame_c };
    ui.painter().rect_stroke(frame, CornerRadius::same(10), Stroke::new(3.0, fc), egui::StrokeKind::Inside);
    let hl = Rect::from_min_max(frame.min, pos2(xa, frame.max.y));
    let hr = Rect::from_min_max(pos2(xb, frame.min.y), frame.max);
    for (h, left) in [(hl, true), (hr, false)] {
        let cr = if left { CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 } } else { CornerRadius { nw: 0, sw: 0, ne: 10, se: 10 } };
        ui.painter().rect_filled(h, cr, fc);
        ui.painter().rect_filled(Rect::from_center_size(h.center(), vec2(3.0, (h.height() * 0.36).max(10.0))), CornerRadius::same(2), if dark || trimmed { BAR } else { p.surface });
    }
    // Tête de lecture
    let x = x_of(pl.time());
    ui.painter().line_segment([pos2(x, r.top() - 5.0), pos2(x, r.bottom() + 5.0)], Stroke::new(3.0, Color32::WHITE));
    ui.painter().line_segment([pos2(x, r.top() - 5.0), pos2(x, r.bottom() + 5.0)], Stroke::new(1.0, Color32::from_black_alpha(90)));
    ui.painter().circle_filled(pos2(x, r.top() - 5.0), 4.0, Color32::WHITE);
    // Repères (L1 : phrases trouvées, L2 : points forts) : trait fin + petit triangle en haut
    let hover = ui.input(|i| i.pointer.hover_pos());
    let mut tip: Option<&str> = None;
    for (t, c, label) in marks {
        let mx = x_of(*t);
        ui.painter().line_segment([pos2(mx, inner.top()), pos2(mx, inner.bottom())], Stroke::new(2.0, c.gamma_multiply(0.85)));
        let tri = vec![pos2(mx - 5.0, r.top() - 1.0), pos2(mx + 5.0, r.top() - 1.0), pos2(mx, r.top() + 6.0)];
        ui.painter().add(egui::Shape::convex_polygon(tri, *c, Stroke::new(1.0, Color32::from_black_alpha(80))));
        if hover.is_some_and(|h| r.expand2(vec2(0.0, 6.0)).contains(h) && (h.x - mx).abs() < 6.0) {
            tip = Some(label.as_str());
        }
    }
    if let Some(tip) = tip {
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id.with("repere"), egui::PopupAnchor::Pointer).show(|ui| ui.label(tip));
    }
    // Interaction : 0 = déplacer la tête, 1 = poignée début, 2 = poignée fin
    let mode_id = id.with("mode");
    if resp.drag_started() || resp.clicked() {
        // Position de l'appui initial (au début du glisser, le pointeur a déjà bougé).
        let q = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()).unwrap_or_default();
        let mode: u8 = if hl.expand2(vec2(8.0, 4.0)).contains(q) {
            1
        } else if hr.expand2(vec2(8.0, 4.0)).contains(q) {
            2
        } else {
            0
        };
        ui.data_mut(|d| d.insert_temp(mode_id, mode));
    }
    let mode: u8 = ui.data(|d| d.get_temp(mode_id)).unwrap_or(0);
    if let Some(q) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
        let t = t_of(q.x);
        let min_len = (dur * 0.005).max(0.1);
        match mode {
            1 => {
                let t = t.min(b - min_len).max(0.0);
                *sel = (Some(t), Some(b));
                pl.seek(t);
            }
            2 => {
                let t = t.max(a + min_len).min(dur);
                *sel = (Some(a), Some(t));
                pl.seek(t);
            }
            _ => {
                // un clic près d'un repère y saute exactement
                let snap = marks.iter().map(|m| m.0).filter(|m| (x_of(*m) - q.x).abs() < 6.0).min_by(|a, b| (x_of(*a) - q.x).abs().total_cmp(&(x_of(*b) - q.x).abs()));
                pl.seek(snap.filter(|_| resp.clicked()).unwrap_or(t));
            }
        }
    }
    if resp.drag_stopped() {
        // Sélection = fichier entier : on revient à « pas de coupe »
        if let (Some(a), Some(b)) = *sel {
            if a <= 0.01 && b >= dur - 0.01 {
                *sel = (None, None);
            }
        }
    }
    if hl.expand(6.0).contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default()) || hr.expand(6.0).contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if let Some(q) = resp.hover_pos() {
        let t = match mode {
            1 if resp.dragged() => a,
            2 if resp.dragged() => b,
            _ => t_of(q.x),
        };
        resp.on_hover_text_at_pointer(hms(t));
    }
}

/// Barre de lecture : volume à gauche, commandes centrées, options à droite.
#[allow(clippy::too_many_arguments)]
fn transport(ui: &mut Ui, r: Rect, pl: &mut Player, loop_sel: &mut bool, kind: Kind, p: &Palette, dark: bool, narrow: bool) -> Option<MediaAct> {
    let video = kind == Kind::Video;
    let mut act = None;
    let look = if dark { on_dark() } else { Look { fg: p.text, bg: Color32::TRANSPARENT, hover: p.hover_bg, round: true } };
    let fg = look.fg;
    // Centre
    let n_small = if video { 4.0 } else { 2.0 };
    let cw = n_small * 40.0 + 52.0 + (n_small + 1.0) * 6.0;
    let center = Rect::from_center_size(r.center(), vec2(cw, 52.0));
    ui.scope_builder(UiBuilder::new().max_rect(center).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if icons::button(ui, Icon::Back5, 40.0, look, "−5 s (←)").clicked() {
            pl.seek(pl.time() - 5.0);
        }
        if video && icons::button(ui, Icon::StepBack, 40.0, look, "Image précédente ( , )").clicked() {
            pl.step(false);
        }
        let play = Look { fg: p.on_accent, bg: p.accent, hover: p.accent.gamma_multiply(0.85), round: true };
        let pi = if pl.playing() { Icon::Pause } else { Icon::Play };
        if icons::button(ui, pi, 52.0, play, "Lecture / pause (Espace)").clicked() {
            pl.toggle();
        }
        if video && icons::button(ui, Icon::StepFwd, 40.0, look, "Image suivante ( . )").clicked() {
            pl.step(true);
        }
        if icons::button(ui, Icon::Fwd5, 40.0, look, "+5 s (→)").clicked() {
            pl.seek(pl.time() + 5.0);
        }
    });
    // Gauche : volume
    let left = Rect::from_min_max(r.min, pos2(center.left() - 12.0, r.max.y));
    ui.scope_builder(UiBuilder::new().max_rect(left).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let muted = pl.volume <= 0.001;
        if icons::button(ui, if muted { Icon::Mute } else { Icon::Volume }, 36.0, look, "Couper le son").clicked() {
            pl.set_volume(if muted { 1.0 } else { 0.0 });
        }
        if left.width() > 150.0 && !narrow {
            ui.spacing_mut().slider_width = (left.width() - 60.0).min(110.0);
            let mut vol = pl.volume as f32;
            let old = ui.visuals().clone();
            if dark {
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(0x2A, 0x3F, 0x66);
                ui.visuals_mut().selection.bg_fill = Color32::from_rgb(0x6F, 0xA8, 0xFF);
            }
            if ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false).trailing_fill(true)).changed() {
                pl.set_volume(vol as f64);
            }
            *ui.visuals_mut() = old;
        }
    });
    // Droite : vitesse, boucle, capture, plein écran
    let right = Rect::from_min_max(pos2(center.right() + 12.0, r.min.y), r.max);
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if video {
            if icons::button(ui, Icon::Fullscreen, 36.0, look, "Plein écran (F)").clicked() {
                ui.ctx().input_mut(|i| i.events.push(egui::Event::Key { key: Key::F11, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }));
            }
            if icons::button(ui, Icon::Camera, 36.0, look, "Capturer l'image (C)").clicked() {
                act = Some(MediaAct::Capture);
            }
        }
        if right.width() > 130.0 {
            let ll = Look { fg: if *loop_sel { GOLD } else { fg }, ..look };
            if icons::button(ui, Icon::Loop, 36.0, ll, if *loop_sel { "Boucle sur la sélection : activée" } else { "Boucle sur la sélection : désactivée" }).clicked() {
                *loop_sel = !*loop_sel;
            }
        }
        // Vitesse : pilule + menu
        let label = format!("{}×", pl.rate.to_string().replace('.', ","));
        let (sr, sresp) = ui.allocate_exact_size(vec2(54.0, 30.0), Sense::click());
        let bg = if sresp.hovered() { look.hover } else if dark { Color32::from_rgb(0x1C, 0x2D, 0x48) } else { p.soft };
        ui.painter().rect_filled(sr, CornerRadius::same(15), bg);
        ui.painter().text(sr.center(), Align2::CENTER_CENTER, label, FontId::monospace(12.5), fg);
        let sresp = sresp.on_hover_text("Vitesse : clic = plus vite, clic droit = moins vite");
        const RATES: [f64; 7] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
        let cur = RATES.iter().position(|v| (pl.rate - v).abs() < 1e-3).unwrap_or(3);
        if sresp.clicked() {
            pl.set_rate(RATES[(cur + 1) % RATES.len()]);
        } else if sresp.secondary_clicked() {
            pl.set_rate(RATES[(cur + RATES.len() - 1) % RATES.len()]);
        } else if sresp.double_clicked() {
            pl.set_rate(1.0);
        }
    });
    act
}
