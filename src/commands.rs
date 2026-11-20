//! Palette de commandes Ctrl+K (socle) et aide F1 : toutes les actions du lecteur au clavier,
//! filtrées en flou (« webp », « pivo », « sombre »…). La liste dépend de l'onglet actif et
//! n'est construite que quand la palette ou l'aide est ouverte.

use super::*;
use socle::icons::Icon;
use socle::ui::Command;

impl App {
    pub(crate) fn commands(&self) -> Vec<Command> {
        let mut v = Vec::new();
        let has = !self.tabs.is_empty();
        let kind = self.tabs.get(self.active).map(|t| t.kind);
        let f = "Fichier";
        v.push(Command::new("open", "Ouvrir un fichier…").group(f).keys("Ctrl+O").icon(Icon::Open));
        v.push(Command::new("close", "Fermer l'onglet").group(f).keys("Ctrl+W").icon(Icon::Close).enabled(has));
        v.push(Command::new("next", "Fichier suivant du dossier").group(f).keys("PgSuiv").icon(Icon::Next).enabled(has));
        v.push(Command::new("prev", "Fichier précédent du dossier").group(f).keys("PgPréc").icon(Icon::Back).enabled(has));
        v.push(Command::new("openwith", "Ajouter à « Ouvrir avec » de Windows").group(f));
        v.push(Command::new("update", "Vérifier les mises à jour").group(f).icon(Icon::Refresh));
        let g = "Vue";
        v.push(Command::new("theme", if self.s.dark { "Thème clair" } else { "Thème sombre" }).group(g).icon(if self.s.dark { Icon::Sun } else { Icon::Moon }));
        v.push(Command::new("clean", "Vue épurée / normale").group(g).keys("Tab").icon(Icon::Clean).enabled(has));
        v.push(Command::new("full", "Plein écran").group(g).keys("F").icon(Icon::Fullscreen).enabled(has));
        v.push(Command::new("rail", "Panneau Modifier / Convertir / Infos").group(g).icon(Icon::Panel).enabled(has));
        v.push(Command::new("help", "Tous les raccourcis").group(g).keys("F1").icon(Icon::Keyboard));
        match kind {
            Some(Kind::Image) => {
                let g = "Image";
                v.push(Command::new("img:Pivoter à gauche", "Pivoter à gauche").group(g).keys("Maj+R").icon(Icon::RotL));
                v.push(Command::new("img:Pivoter à droite", "Pivoter à droite").group(g).keys("R").icon(Icon::RotR));
                v.push(Command::new("img:Miroir horizontal", "Miroir horizontal").group(g).icon(Icon::FlipH));
                v.push(Command::new("img:Miroir vertical", "Miroir vertical").group(g).icon(Icon::FlipV));
                v.push(Command::new("crop", "Rogner").group(g).icon(Icon::Crop));
                v.push(Command::new("img:Réduire 50 %", "Réduire à 50 %").group(g));
                v.push(Command::new("img:Limiter à 1920 px", "Limiter à 1920 px").group(g));
                v.push(Command::new("undo", "Annuler la modification").group(g).keys("Ctrl+Z").icon(Icon::Undo));
                v.push(Command::new("savecopy", "Enregistrer une copie").group(g).keys("Ctrl+S").icon(Icon::Save));
                v.push(Command::new("replace", "Remplacer l'original").group(g).keys("Ctrl+Maj+S"));
            }
            Some(Kind::Text) => {
                v.push(Command::new("save", "Enregistrer").group("Texte").keys("Ctrl+S").icon(Icon::Save));
                v.push(Command::new("savecopy", "Enregistrer une copie").group("Texte").keys("Ctrl+Maj+S"));
            }
            Some(Kind::Audio) | Some(Kind::Video) => {
                let g = "Lecture";
                v.push(Command::new("play", "Lecture / pause").group(g).keys("Espace").icon(Icon::Play));
                v.push(Command::new("back10", "Reculer de 10 s").group(g).keys("J").icon(Icon::Back10));
                v.push(Command::new("fwd10", "Avancer de 10 s").group(g).keys("L").icon(Icon::Fwd10));
                v.push(Command::new("in", "Début de la coupe ici").group("Montage").keys("I").icon(Icon::MarkIn));
                v.push(Command::new("out", "Fin de la coupe ici").group("Montage").keys("O").icon(Icon::MarkOut));
                v.push(Command::new("keepall", "Tout garder (annuler la sélection)").group("Montage").keys("X"));
                v.push(Command::new("cut_fast", "Couper la sélection (rapide)").group("Montage").icon(Icon::Scissors));
                v.push(Command::new("cut_exact", "Couper la sélection (précis)").group("Montage").icon(Icon::Scissors));
                if kind == Some(Kind::Video) {
                    v.push(Command::new("capture", "Capturer l'image en PNG").group(g).keys("C").icon(Icon::Camera));
                    v.push(Command::new("subs", "Sous-titres oui / non").group(g).keys("V").icon(Icon::Subs));
                    v.push(Command::new("sfind", "Chercher une phrase dans les sous-titres").group(g).keys("Ctrl+F").icon(Icon::Search));
                    v.push(Command::new("mom_next", "Point fort suivant").group("Points forts").keys("N").icon(Icon::Bolt));
                    v.push(Command::new("mom_prev", "Point fort précédent").group("Points forts").keys("Maj+N").icon(Icon::Bolt));
                    v.push(Command::new("mom_export", "Exporter ce moment (±10 s)").group("Points forts").keys("E").icon(Icon::Scissors));
                }
            }
            _ => {}
        }
        // Conversions possibles pour l'onglet actif
        if let Some(t) = self.tabs.get(self.active) {
            for (i, tg) in convert::targets(t.kind, &kind::ext(&t.path)).iter().enumerate() {
                v.push(Command::new(format!("conv:{i}"), format!("Convertir en {}", tg.label())).group("Convertir").icon(Icon::Convert));
            }
        }
        v
    }

    pub(crate) fn run_command(&mut self, ctx: &egui::Context, id: &str) {
        match id {
            "open" => {
                let p = winx::open_dialog();
                self.open_paths(ctx, p);
            }
            "close" => self.close(self.active),
            "next" => self.neighbour(ctx, 1),
            "prev" => self.neighbour(ctx, -1),
            "openwith" => match winx::register_open_with() {
                Ok(n) => self.toasts.ok(format!("Inscrit pour {n} extensions : clic droit › Ouvrir avec › Lecteur universel")),
                Err(e) => self.toasts.error(e),
            },
            "update" => self.check_updates(true),
            "theme" => self.set_theme(ctx, !self.s.dark),
            "clean" => self.panels = !self.panels,
            "full" => self.want_fullscreen = Some(!self.fullscreen),
            "rail" => self.s.hide_rail = !self.s.hide_rail,
            "help" => self.help_open = true,
            "crop" => {
                self.panels = true;
                self.s.hide_rail = false;
                if let Some(View::Image(v)) = self.tabs.get_mut(self.active).map(|t| &mut t.view) {
                    v.crop = true;
                    v.crop_rect = None;
                }
            }
            "undo" => self.undo_image(),
            "save" => self.save_current(false),
            "savecopy" => {
                let is_text = matches!(self.tabs.get(self.active).map(|t| &t.view), Some(View::Text(_)));
                self.save_current(is_text);
            }
            "replace" => self.save_current(true),
            "capture" => self.capture(),
            "cut_fast" => self.cut_media(false),
            "cut_exact" => self.cut_media(true),
            "mom_export" => self.export_moment(),
            "sfind" => {
                if let Some(View::Media(m)) = self.tabs.get_mut(self.active).map(|t| &mut t.view) {
                    if m.subs.is_some() {
                        m.sfind_open = true;
                        m.sfind_focus = true;
                        m.sac.reset();
                        return;
                    }
                }
                self.toasts.error("Pas de sous-titres : glissez un fichier .srt ou .vtt sur la vidéo pour y chercher une phrase.");
            }
            _ => {
                if let Some(what) = id.strip_prefix("img:") {
                    self.edit_image(what);
                } else if let Some(i) = id.strip_prefix("conv:").and_then(|i| i.parse::<usize>().ok()) {
                    let tg = self.tabs.get(self.active).and_then(|t| convert::targets(t.kind, &kind::ext(&t.path)).get(i).copied());
                    if let Some(tg) = tg {
                        self.start_convert(tg, vec![self.active]);
                    }
                } else {
                    self.media_command(id);
                }
            }
        }
    }

    fn media_command(&mut self, id: &str) {
        let Some(t) = self.tabs.get_mut(self.active) else { return };
        if id == "subs" {
            if let View::Media(m) = &mut t.view {
                m.subs_on = !m.subs_on;
            }
            return;
        }
        if id == "keepall" {
            t.sel = (None, None);
            return;
        }
        let moments: Vec<f64> = match &t.view {
            View::Media(m) => m.moments.iter().flatten().map(|x| x.t).collect(),
            _ => Vec::new(),
        };
        let Some(p) = t.player() else { return };
        let (now, dur) = (p.time(), p.duration);
        match id {
            "mom_next" => {
                if let Some(&x) = moments.iter().find(|&&x| x > now + 0.5) {
                    p.seek(x);
                }
            }
            "mom_prev" => {
                if let Some(&x) = moments.iter().rev().find(|&&x| x < now - 1.0) {
                    p.seek(x);
                }
            }
            "play" => p.toggle(),
            "back10" => p.seek(now - 10.0),
            "fwd10" => p.seek(now + 10.0),
            "in" => t.sel = (Some(now), Some(t.sel.1.filter(|b| *b > now + 0.05).unwrap_or(dur))),
            "out" => t.sel = (Some(t.sel.0.filter(|a| *a < now - 0.05).unwrap_or(0.0)), Some(now)),
            _ => {}
        }
    }

    /// Ctrl+K (palette) et F1 (aide) : à appeler à chaque image, après le reste de l'interface.
    pub(crate) fn draw_palette(&mut self, ctx: &egui::Context) {
        if !self.palette.is_open() && !self.help_open {
            return;
        }
        let cmds = self.commands();
        let p = self.pal;
        if self.palette.is_open() {
            if let Some(id) = self.palette.show(ctx, &p, &cmds) {
                self.run_command(ctx, &id);
                self.s.palette_recent = self.palette.recent.clone();
            }
        }
        if self.help_open {
            socle::ui::help_window(ctx, &p, &mut self.help_open, &cmds);
        }
        // En lecture GPU, la vidéo est une sous-fenêtre au-dessus de l'interface : on y découpe
        // la palette et l'aide pour qu'elles restent visibles.
        for (id, open) in [("socle-palette", self.palette.is_open()), ("socle-aide", self.help_open)] {
            if let Some(r) = ctx.memory(|m| m.area_rect(egui::Id::unique(id))).filter(|_| open) {
                self.occl.push(r);
            }
        }
    }
}
