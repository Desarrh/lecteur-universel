//! Éditeur de texte / code : recherche et remplacement, aller à la ligne,
//! numéros de ligne alignés sur les lignes affichées, aperçu Markdown.

use crate::icons::{self, Icon, Look};
use crate::theme::Palette;
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Align, Color32, CornerRadius, FontFamily, FontId, Frame, Key, Margin, Modifiers, Rect, RichText, Stroke, TextFormat, Ui, pos2, vec2};

use socle::autocomplete::{Autocomplete, History};
use std::sync::mpsc::Receiver;

/// Autocomplétion d'un champ de recherche (socle) : les mots du document, indexés dans un fil
/// à l'ouverture de la recherche, plus les recherches récentes (communes à tous les fichiers).
#[derive(Default)]
pub struct SearchAc {
    ac: Autocomplete,
    field: socle::ui::AutoField,
    rx: Option<Receiver<Vec<String>>>,
    built: bool,
}

impl SearchAc {
    /// Indexe (une fois) les mots de `text` dans un fil ; l'interface ne l'attend jamais.
    pub fn index(&mut self, text: &str, hist: &History) {
        if self.built {
            return;
        }
        self.built = true;
        self.ac.history = hist.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let t = text.to_owned();
        std::thread::spawn(move || {
            let _ = tx.send(words(&t));
        });
        self.rx = Some(rx);
    }

    pub fn is_built(&self) -> bool {
        self.built
    }

    /// À refaire la prochaine fois (le document a pu changer).
    pub fn reset(&mut self) {
        self.built = false;
    }

    /// Champ de recherche avec suggestions. Une recherche validée (Entrée) entre dans l'historique.
    #[allow(clippy::too_many_arguments)]
    pub fn field(&mut self, ui: &mut Ui, p: &Palette, id: egui::Id, text: &mut String, hint: &str, width: f32, hist: &mut History) -> socle::ui::AutoResponse {
        if let Some(rx) = &self.rx {
            if let Ok(w) = rx.try_recv() {
                self.ac.set_candidates(w);
                self.rx = None;
            }
        }
        let r = socle::ui::autocomplete_field(ui, p, id, &mut self.field, &self.ac, text, hint, width);
        if let Some(s) = r.submitted.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            hist.record(s);
            self.ac.history.record(s);
        }
        r
    }
}

/// Mots distincts d'un texte (3 lettres ou plus), les plus fréquents d'abord, au plus 20 000.
pub fn words(text: &str) -> Vec<String> {
    let mut count: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for w in text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '\'')) {
        let w = w.trim_matches(|c: char| c == '-' || c == '\'');
        if w.chars().count() >= 3 && w.len() <= 48 {
            *count.entry(w).or_default() += 1;
        }
    }
    let mut v: Vec<(&str, u32)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    v.into_iter().take(20_000).map(|(w, _)| w.to_string()).collect()
}

#[derive(Default)]
pub struct EditState {
    pub find_open: bool,
    pub replace_open: bool,
    pub find: String,
    pub repl: String,
    pub case: bool,
    pub hit: usize,
    pub goto_open: bool,
    pub goto: String,
    pub preview: bool,
    focus_find: bool,
    focus_goto: bool,
    /// Après « aller à la ligne » : rendre le focus à l'éditeur (pas après une recherche)
    focus_editor: bool,
    /// Sélection à appliquer (plage en caractères) et à faire défiler jusqu'à elle.
    jump: Option<(usize, usize)>,
    /// Autocomplétion du champ « Rechercher »
    sac: SearchAc,
}

/// Occurrences de `needle` dans `hay`, en indices de caractères (au plus 20 000).
pub fn find_all(hay: &str, needle: &str, case: bool) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let (h, n);
    let (hs, ns) = if case {
        (hay, needle)
    } else {
        h = hay.to_lowercase();
        n = needle.to_lowercase();
        // si la mise en minuscules change les longueurs, on cherche en respectant la casse
        if h.len() == hay.len() && n.len() == needle.len() { (h.as_str(), n.as_str()) } else { (hay, needle) }
    };
    let nlen = needle.chars().count();
    let mut out = Vec::new();
    let mut last_b = 0usize;
    let mut last_c = 0usize;
    for (b, _) in hs.match_indices(ns) {
        last_c += hay[last_b..b].chars().count();
        last_b = b;
        out.push((last_c, last_c + nlen));
        if out.len() >= 20_000 {
            break;
        }
    }
    out
}

fn byte_of(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

fn line_start(s: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut n = 1;
    for (ci, ch) in s.chars().enumerate() {
        if ch == '\n' {
            n += 1;
            if n == line {
                return ci + 1;
            }
        }
    }
    s.chars().count()
}

/// Barre d'outils + éditeur. Renvoie vrai si le texte a été modifié par un remplacement.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub fn show(ui: &mut Ui, text: &mut String, lang: &str, wrap: &mut bool, st: &mut EditState, read_only: bool, p: &Palette, hist: &mut History) {
    let id = egui::Id::unique("editor");
    let is_md = matches!(lang, "md" | "markdown");
    // Raccourcis (même quand l'éditeur a le focus)
    let (f, h, g, e, esc) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::CTRL, Key::F),
            i.consume_key(Modifiers::CTRL, Key::H),
            i.consume_key(Modifiers::CTRL, Key::G),
            i.consume_key(Modifiers::CTRL, Key::E),
            i.key_pressed(Key::Escape),
        )
    });
    let selected = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range()).filter(|r| !r.is_empty()).map(|r| {
        let (pi, si): (usize, usize) = (r.primary.index.into(), r.secondary.index.into());
        let (a, b) = (pi.min(si), pi.max(si));
        text.chars().skip(a).take(b - a).collect::<String>()
    });
    if f || h {
        if !st.find_open {
            st.sac.reset();
        }
        st.find_open = true;
        st.replace_open = h || st.replace_open;
        st.focus_find = true;
        if let Some(s) = selected.filter(|s| !s.contains('\n') && s.len() < 200) {
            st.find = s;
        }
    }
    if g {
        st.goto_open = true;
        st.focus_goto = true;
    }
    if e && is_md {
        st.preview = !st.preview;
    }
    if esc {
        st.find_open = false;
        st.goto_open = false;
    }
    let hits = if st.find_open { find_all(text, &st.find, st.case) } else { Vec::new() };
    if st.hit >= hits.len() {
        st.hit = 0;
    }

    // ---- Barre d'outils
    Frame::new().fill(p.surface).inner_margin(Margin::symmetric(10, 6)).stroke(Stroke::new(1.0, p.border)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let lk = Look { fg: p.text, bg: Color32::TRANSPARENT, hover: p.hover_bg, round: false };
            if !st.find_open {
                if ui.add(egui::Button::new(RichText::new("Rechercher   Ctrl+F").size(12.5))).clicked() {
                    st.find_open = true;
                    st.focus_find = true;
                }
                if !read_only && ui.add(egui::Button::new(RichText::new("Remplacer   Ctrl+H").size(12.5))).clicked() {
                    st.find_open = true;
                    st.replace_open = true;
                    st.focus_find = true;
                }
            } else {
                st.sac.index(text, hist);
                let fid = egui::Id::unique("find_field");
                let ar = st.sac.field(ui, p, fid, &mut st.find, "Rechercher", 240.0, hist);
                let r = ar.response.clone();
                if st.focus_find {
                    r.request_focus();
                    st.focus_find = false;
                }
                if r.changed() {
                    st.hit = 0;
                    if let Some(h0) = find_all(text, &st.find, st.case).first() {
                        st.jump = Some(*h0);
                    }
                }
                if ar.completed {
                    st.hit = 0;
                    if let Some(h0) = find_all(text, &st.find, st.case).first() {
                        st.jump = Some(*h0);
                    }
                }
                let enter = ar.submitted.is_some();
                let shift = ui.input(|i| i.modifiers.shift);
                let count = if st.find.is_empty() { String::new() } else if hits.is_empty() { "aucun résultat".into() } else { format!("{} / {}", st.hit + 1, hits.len()) };
                ui.label(RichText::new(count).size(12.0).color(if hits.is_empty() && !st.find.is_empty() { p.del } else { p.muted }));
                let mut go: i32 = 0;
                if icons::button(ui, Icon::StepBack, 28.0, lk, "Précédent (Maj+Entrée)").clicked() {
                    go = -1;
                }
                if icons::button(ui, Icon::StepFwd, 28.0, lk, "Suivant (Entrée)").clicked() {
                    go = 1;
                }
                if enter {
                    go = if shift { -1 } else { 1 };
                    st.focus_find = true;
                }
                if go != 0 && !hits.is_empty() {
                    st.hit = (st.hit as i64 + go as i64).rem_euclid(hits.len() as i64) as usize;
                    st.jump = Some(hits[st.hit]);
                }
                if ui.add(egui::Button::new(RichText::new("Aa").size(12.5).strong()).selected(st.case)).on_hover_text("Respecter la casse").clicked() {
                    st.case = !st.case;
                }
                if !read_only && ui.add(egui::Button::new(RichText::new("Remplacer").size(12.5)).selected(st.replace_open)).clicked() {
                    st.replace_open = !st.replace_open;
                }
                if icons::button(ui, Icon::Close, 28.0, lk, "Fermer (Échap)").clicked() {
                    st.find_open = false;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if is_md {
                    let lab = if st.preview { "Éditer   Ctrl+E" } else { "Aperçu   Ctrl+E" };
                    if ui.add(egui::Button::new(RichText::new(lab).size(12.5)).selected(st.preview)).clicked() {
                        st.preview = !st.preview;
                    }
                }
                if ui.add(egui::Button::new(RichText::new("Retour à la ligne").size(12.5)).selected(*wrap)).clicked() {
                    *wrap = !*wrap;
                }
                if st.goto_open {
                    let r = ui.add(egui::TextEdit::singleline(&mut st.goto).hint_text("n°").desired_width(60.0).id(egui::Id::unique("goto_field")));
                    if st.focus_goto {
                        r.request_focus();
                        st.focus_goto = false;
                    }
                    ui.label(RichText::new("Aller à la ligne").size(12.5).color(p.muted));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        if let Ok(n) = st.goto.trim().parse::<usize>() {
                            let c = line_start(text, n);
                            st.jump = Some((c, c));
                            st.focus_editor = true;
                        }
                        st.goto_open = false;
                    }
                } else if ui.add(egui::Button::new(RichText::new("Ligne…   Ctrl+G").size(12.5))).clicked() {
                    st.goto_open = true;
                    st.focus_goto = true;
                }
            });
        });
        if st.find_open && st.replace_open && !read_only {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.add(egui::TextEdit::singleline(&mut st.repl).hint_text("Remplacer par").desired_width(240.0));
                if ui.add_enabled(!hits.is_empty(), egui::Button::new(RichText::new("Remplacer").size(12.5))).clicked() {
                    let (a, b) = hits[st.hit];
                    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
                    text.replace_range(ba..bb, &st.repl);
                    let next = find_all(text, &st.find, st.case);
                    if let Some(h) = next.iter().find(|h| h.0 >= a + st.repl.chars().count()).or(next.first()) {
                        st.jump = Some(*h);
                    }
                }
                if ui.add_enabled(!hits.is_empty(), egui::Button::new(RichText::new(format!("Tout remplacer ({})", hits.len())).size(12.5))).clicked() {
                    let mut out = String::with_capacity(text.len());
                    let mut last = 0usize;
                    for (a, b) in &hits {
                        let (ba, bb) = (byte_of(text, *a), byte_of(text, *b));
                        out.push_str(&text[last..ba]);
                        out.push_str(&st.repl);
                        last = bb;
                    }
                    out.push_str(&text[last..]);
                    *text = out;
                }
            });
        }
    });

    if is_md && st.preview {
        markdown(ui, text, p);
        return;
    }

    // ---- Éditeur
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style());
    let lang_s = lang.to_string();
    let wrap_on = *wrap;
    let big = text.len() > 400_000;
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_w: f32| {
        let mut job = if big {
            LayoutJob::simple(buf.as_str().to_owned(), FontId::monospace(13.5), ui.visuals().text_color(), f32::INFINITY)
        } else {
            egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, buf.as_str(), &lang_s)
        };
        for sec in &mut job.sections {
            sec.format.font_id = FontId::monospace(13.5);
        }
        job.wrap.max_width = if wrap_on { wrap_w } else { f32::INFINITY };
        ui.painter().layout_job(job)
    };
    // Sélection demandée (recherche / aller à la ligne)
    let jump = st.jump.take();
    if let Some((a, b)) = jump {
        let mut s = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        s.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        s.store(ui.ctx(), id);
    }
    let lines = text.lines().count().max(1) + usize::from(text.ends_with('\n'));
    let digits = lines.to_string().len().max(3);
    let col_w = digits as f32 * 8.2 + 18.0;
    Frame::new().fill(p.code).inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 0 }).show(ui, |ui| {
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.add_space(col_w);
                let out = egui::TextEdit::multiline(text)
                    .code_editor()
                    .font(FontId::monospace(13.5))
                    .frame(Frame::NONE)
                    .interactive(!read_only)
                    .desired_width(if wrap_on { ui.available_width() } else { f32::INFINITY })
                    .desired_rows(40)
                    .lock_focus(true)
                    .id(id)
                    .layouter(&mut layouter)
                    .show(ui);
                let gp = out.galley_pos;
                let galley = &out.galley;
                let clip = ui.clip_rect();
                let painter = ui.painter();
                // Ligne courante
                if let Some(cr) = out.cursor_range {
                    let r = galley.pos_from_cursor(cr.primary);
                    let row = Rect::from_min_max(pos2(clip.left(), gp.y + r.top()), pos2(clip.right(), gp.y + r.bottom()));
                    painter.rect_filled(row, CornerRadius::ZERO, p.accent.gamma_multiply(0.07));
                }
                // Occurrences de la recherche
                for (i, (a, b)) in hits.iter().enumerate() {
                    let ra = galley.pos_from_cursor(CCursor::new(*a));
                    let rb = galley.pos_from_cursor(CCursor::new(*b));
                    let y = gp.y + ra.top();
                    if y > clip.bottom() || y + ra.height() < clip.top() {
                        continue;
                    }
                    let r = if (ra.top() - rb.top()).abs() < 1.0 { Rect::from_min_max(gp + ra.min.to_vec2(), gp + vec2(rb.max.x, ra.max.y)) } else { Rect::from_min_size(gp + ra.min.to_vec2(), vec2(30.0, ra.height())) };
                    let c = if i == st.hit { Color32::from_rgba_unmultiplied(242, 184, 75, 130) } else { Color32::from_rgba_unmultiplied(242, 184, 75, 55) };
                    painter.rect_filled(r.expand(1.0), CornerRadius::same(2), c);
                }
                // Numéros de ligne alignés sur les lignes affichées (justes même avec retour à la ligne)
                let mut line = 1usize;
                let font = FontId::monospace(13.5);
                let cur_row_y = out.cursor_range.map(|cr| galley.pos_from_cursor(cr.primary).top());
                for (i, row) in galley.rows.iter().enumerate() {
                    let starts = i == 0 || galley.rows[i - 1].ends_with_newline;
                    if starts {
                        let rr = row.rect();
                        let y = gp.y + rr.top();
                        if y + rr.height() >= clip.top() && y <= clip.bottom() {
                            let active = cur_row_y.is_some_and(|cy| (cy - rr.top()).abs() < 1.0);
                            painter.text(
                                pos2(gp.x - 12.0, y),
                                egui::Align2::RIGHT_TOP,
                                line.to_string(),
                                font.clone(),
                                if active { p.text } else { p.muted.gamma_multiply(0.6) },
                            );
                        }
                        line += 1;
                    }
                }
                if jump.is_some() {
                    if let Some(cr) = out.cursor_range {
                        let r = galley.pos_from_cursor(cr.primary);
                        ui.scroll_to_rect(Rect::from_min_size(gp + r.min.to_vec2(), vec2(40.0, r.height())), Some(Align::Center));
                    }
                    if std::mem::take(&mut st.focus_editor) {
                        out.response.response.request_focus();
                    }
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------
// Aperçu Markdown (titres, listes, citations, code, gras / italique / `code` / liens)

fn inline(job: &mut LayoutJob, s: &str, size: f32, color: Color32, accent: Color32, code_bg: Color32, base_bold: bool) {
    let chars: Vec<char> = s.chars().collect();
    let (mut bold, mut italic, mut code) = (base_bold, false, false);
    let mut buf = String::new();
    let flush = |job: &mut LayoutJob, buf: &mut String, bold: bool, italic: bool, code: bool, link: bool| {
        if buf.is_empty() {
            return;
        }
        let fam = if code { FontFamily::Monospace } else if bold { FontFamily::Name("bold".into()) } else { FontFamily::Proportional };
        let mut f = TextFormat::simple(FontId::new(if code { size * 0.92 } else { size }, fam), if link { accent } else { color });
        f.italics = italic;
        if code {
            f.background = code_bg;
        }
        if link {
            f.underline = Stroke::new(1.0, accent);
        }
        job.append(buf, 0.0, f);
        buf.clear();
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            flush(job, &mut buf, bold, italic, code, false);
            code = !code;
            i += 1;
            continue;
        }
        if !code && c == '*' && chars.get(i + 1) == Some(&'*') {
            flush(job, &mut buf, bold, italic, code, false);
            bold = !bold;
            i += 2;
            continue;
        }
        if !code && (c == '*' || c == '_') && chars.get(i + 1).is_some_and(|n| !n.is_whitespace()) || (!code && (c == '*' || c == '_') && italic) {
            flush(job, &mut buf, bold, italic, code, false);
            italic = !italic;
            i += 1;
            continue;
        }
        if !code && c == '!' && chars.get(i + 1) == Some(&'[') {
            if let Some(close) = chars[i..].iter().position(|&x| x == ']').map(|k| k + i) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close..].iter().position(|&x| x == ')').map(|k| k + close) {
                        flush(job, &mut buf, bold, italic, code, false);
                        let alt: String = chars[i + 2..close].iter().collect();
                        let mut f = TextFormat::simple(FontId::proportional(size * 0.85), color);
                        f.background = code_bg;
                        f.italics = true;
                        job.append(&format!(" 🖼 image : {alt} "), 0.0, f);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        if !code && c == '[' {
            if let Some(close) = chars[i..].iter().position(|&x| x == ']').map(|k| k + i) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close..].iter().position(|&x| x == ')').map(|k| k + close) {
                        flush(job, &mut buf, bold, italic, code, false);
                        buf = chars[i + 1..close].iter().collect();
                        flush(job, &mut buf, bold, italic, code, true);
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    flush(job, &mut buf, bold, italic, code, false);
}

pub fn markdown(ui: &mut Ui, text: &str, p: &Palette) {
    let paper = if p.dark { Color32::from_rgb(0x1A, 0x2C, 0x49) } else { Color32::WHITE };
    let ink = if p.dark { p.text } else { Color32::from_rgb(0x1B, 0x24, 0x33) };
    let code_bg = if p.dark { Color32::from_rgb(0x0D, 0x18, 0x2A) } else { Color32::from_rgb(0xF1, 0xEC, 0xE0) };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            let w = (ui.available_width() - 48.0).clamp(300.0, 820.0);
            Frame::new().fill(paper).inner_margin(Margin::symmetric(48, 36)).show(ui, |ui| {
                ui.set_width(w - 96.0);
                ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                    let mut in_code = false;
                    let mut code = String::new();
                    let mut table: Vec<Vec<String>> = Vec::new();
                    let flush_table = |ui: &mut Ui, table: &mut Vec<Vec<String>>| {
                        if table.is_empty() {
                            return;
                        }
                        let cols = table.iter().map(|r| r.len()).max().unwrap_or(1);
                        egui::Grid::new(ui.next_auto_id()).striped(true).num_columns(cols).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                            for (i, row) in table.iter().enumerate() {
                                for c in row {
                                    let mut job = LayoutJob::default();
                                    inline(&mut job, c, 14.0, ink, p.accent, code_bg, i == 0);
                                    ui.label(job);
                                }
                                ui.end_row();
                            }
                        });
                        ui.add_space(8.0);
                        table.clear();
                    };
                    for raw in text.lines() {
                        let tl = raw.trim();
                        if tl.starts_with('|') && !in_code {
                            let cells: Vec<String> = tl.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
                            let sep = cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'));
                            if !sep && cells.iter().any(|c| !c.is_empty()) {
                                table.push(cells);
                            }
                            continue;
                        }
                        flush_table(ui, &mut table);
                        let line = raw.trim_end();
                        if line.trim_start().starts_with("```") {
                            if in_code {
                                Frame::new().fill(code_bg).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(10)).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(code.trim_end()).monospace().size(13.0).color(ink));
                                });
                                ui.add_space(6.0);
                                code.clear();
                            }
                            in_code = !in_code;
                            continue;
                        }
                        if in_code {
                            code.push_str(raw);
                            code.push('\n');
                            continue;
                        }
                        let t = line.trim_start();
                        if t.is_empty() {
                            ui.add_space(8.0);
                            continue;
                        }
                        if t == "---" || t == "***" || t == "___" {
                            ui.separator();
                            continue;
                        }
                        let mut job = LayoutJob::default();
                        job.wrap.max_width = ui.available_width();
                        let hashes = t.chars().take_while(|c| *c == '#').count();
                        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
                            let size = [28.0, 23.0, 19.0, 17.0, 15.5, 15.0][hashes - 1];
                            ui.add_space(if hashes <= 2 { 12.0 } else { 6.0 });
                            inline(&mut job, t[hashes..].trim(), size, ink, p.accent, code_bg, true);
                            ui.label(job);
                            if hashes <= 2 {
                                ui.separator();
                            }
                            continue;
                        }
                        let indent = (line.len() - t.len()) as f32 * 6.0;
                        let (prefix, body) = if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
                            ("•  ".to_string(), r)
                        } else if let Some(k) = t.find(". ").filter(|k| *k <= 3 && t[..*k].chars().all(|c| c.is_ascii_digit())) {
                            (format!("{}.  ", &t[..k]), &t[k + 2..])
                        } else if let Some(r) = t.strip_prefix("> ") {
                            ("▍ ".to_string(), r)
                        } else {
                            (String::new(), t)
                        };
                        if !prefix.is_empty() {
                            job.append(&prefix, indent, TextFormat::simple(FontId::proportional(15.0), if prefix.starts_with('▍') { p.accent } else { ink }));
                        }
                        let col = if prefix.starts_with('▍') { p.muted } else { ink };
                        inline(&mut job, body, 15.0, col, p.accent, code_bg, false);
                        ui.label(job);
                        ui.add_space(3.0);
                    }
                    flush_table(ui, &mut table);
                    if in_code && !code.is_empty() {
                        ui.label(RichText::new(code.trim_end()).monospace().size(13.0).color(ink));
                    }
                });
            });
        });
        ui.add_space(20.0);
    });
}
