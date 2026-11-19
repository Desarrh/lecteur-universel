//! Icônes vectorielles dessinées au pinceau egui : nettes à toutes les tailles,
//! toujours centrées (les glyphes Unicode/emoji ne le sont pas).

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Play,
    Pause,
    StepBack,
    StepFwd,
    Back5,
    Fwd5,
    Camera,
    Volume,
    Mute,
    Fullscreen,
    Unfullscreen,
    Scissors,
    RotL,
    RotR,
    FlipH,
    FlipV,
    Crop,
    Resize,
    Undo,
    Save,
    Folder,
    Plus,
    Close,
    Moon,
    Sun,
    More,
    Panel,
    Convert,
    MarkIn,
    MarkOut,
    Loop,
    Open,
    Copy,
    Check,
    Reset,
    Drag,
    Logo,
    Clean,
}

/// Peint `icon` centré dans `rect` (grille 24×24 mise à l'échelle).
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, c: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0 * s, 12.0 * s);
    let q = |x: f32, y: f32| -> Pos2 { o + vec2(x * s, y * s) };
    let st = Stroke::new((1.8 * s).max(1.2), c);
    let line = |a: (f32, f32), b: (f32, f32)| p.line_segment([q(a.0, a.1), q(b.0, b.1)], st);
    let poly = |pts: &[(f32, f32)], fill: bool| {
        let v: Vec<Pos2> = pts.iter().map(|(x, y)| q(*x, *y)).collect();
        if fill {
            p.add(Shape::convex_polygon(v, c, Stroke::NONE));
        } else {
            p.add(Shape::line(v, st));
        }
    };
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = 20;
        let v: Vec<Pos2> = (0..=n).map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
            q(cx + r * a.cos(), cy + r * a.sin())
        }).collect();
        p.add(Shape::line(v, st));
    };
    match icon {
        Icon::Play => poly(&[(8.0, 5.5), (19.0, 12.0), (8.0, 18.5)], true),
        Icon::Pause => {
            p.rect_filled(Rect::from_min_max(q(7.0, 5.5), q(10.5, 18.5)), CornerRadius::same((1.0 * s) as u8), c);
            p.rect_filled(Rect::from_min_max(q(13.5, 5.5), q(17.0, 18.5)), CornerRadius::same((1.0 * s) as u8), c);
        }
        Icon::StepBack => {
            p.rect_filled(Rect::from_min_max(q(6.0, 6.5), q(8.2, 17.5)), CornerRadius::ZERO, c);
            poly(&[(18.0, 6.5), (9.5, 12.0), (18.0, 17.5)], true);
        }
        Icon::StepFwd => {
            p.rect_filled(Rect::from_min_max(q(15.8, 6.5), q(18.0, 17.5)), CornerRadius::ZERO, c);
            poly(&[(6.0, 6.5), (14.5, 12.0), (6.0, 17.5)], true);
        }
        Icon::Back5 | Icon::Fwd5 => {
            let fwd = icon == Icon::Fwd5;
            if fwd { arc(12.0, 13.0, 8.0, -70.0, 200.0) } else { arc(12.0, 13.0, 8.0, -20.0, 250.0) }
            let tip = if fwd { (14.7, 5.4) } else { (9.3, 5.4) };
            let dir = if fwd { 1.0 } else { -1.0 };
            poly(&[(tip.0 - 3.2 * dir, 2.6), (tip.0 + 0.6 * dir, tip.1), (tip.0 - 3.0 * dir, 8.4)], false);
            p.text(q(12.0, 13.4), egui::Align2::CENTER_CENTER, "5", egui::FontId::new(9.5 * s, egui::FontFamily::Name("bold".into())), c);
        }
        Icon::Camera => {
            poly(&[(3.5, 8.0), (7.5, 8.0), (9.0, 5.5), (15.0, 5.5), (16.5, 8.0), (20.5, 8.0), (20.5, 18.5), (3.5, 18.5), (3.5, 8.0)], false);
            p.circle_stroke(q(12.0, 13.0), 3.4 * s, st);
        }
        Icon::Volume | Icon::Mute => {
            poly(&[(4.0, 9.5), (7.5, 9.5), (12.0, 5.5), (12.0, 18.5), (7.5, 14.5), (4.0, 14.5)], true);
            if icon == Icon::Volume {
                arc(13.0, 12.0, 3.5, -50.0, 50.0);
                arc(13.0, 12.0, 6.5, -50.0, 50.0);
            } else {
                line((15.0, 9.0), (21.0, 15.0));
                line((21.0, 9.0), (15.0, 15.0));
            }
        }
        Icon::Fullscreen => {
            for (a, b, cc) in [((4.0, 9.0), (4.0, 4.0), (9.0, 4.0)), ((15.0, 4.0), (20.0, 4.0), (20.0, 9.0)), ((20.0, 15.0), (20.0, 20.0), (15.0, 20.0)), ((9.0, 20.0), (4.0, 20.0), (4.0, 15.0))] {
                poly(&[a, b, cc], false);
            }
        }
        Icon::Unfullscreen => {
            for (a, b, cc) in [((4.0, 9.0), (9.0, 9.0), (9.0, 4.0)), ((15.0, 4.0), (15.0, 9.0), (20.0, 9.0)), ((20.0, 15.0), (15.0, 15.0), (15.0, 20.0)), ((9.0, 20.0), (9.0, 15.0), (4.0, 15.0))] {
                poly(&[a, b, cc], false);
            }
        }
        Icon::Scissors => {
            p.circle_stroke(q(6.5, 6.5), 2.8 * s, st);
            p.circle_stroke(q(6.5, 17.5), 2.8 * s, st);
            line((8.6, 8.4), (20.0, 18.5));
            line((8.6, 15.6), (20.0, 5.5));
        }
        Icon::RotL | Icon::RotR => {
            let r = icon == Icon::RotR;
            arc(12.0, 13.0, 7.0, if r { -80.0 } else { -100.0 + 0.0 }, if r { 200.0 } else { -380.0 });
            let (x, d) = if r { (12.5, 1.0) } else { (11.5, -1.0) };
            poly(&[(x - 0.5 * d, 2.8), (x + 3.5 * d, 6.0), (x - 0.5 * d, 9.2)], true);
        }
        Icon::FlipH => {
            line((12.0, 3.5), (12.0, 20.5));
            poly(&[(9.5, 6.0), (3.5, 18.0), (9.5, 18.0), (9.5, 6.0)], false);
            poly(&[(14.5, 6.0), (20.5, 18.0), (14.5, 18.0)], true);
        }
        Icon::FlipV => {
            line((3.5, 12.0), (20.5, 12.0));
            poly(&[(6.0, 9.5), (18.0, 3.5), (18.0, 9.5), (6.0, 9.5)], false);
            poly(&[(6.0, 14.5), (18.0, 20.5), (18.0, 14.5)], true);
        }
        Icon::Crop => {
            poly(&[(7.0, 2.5), (7.0, 17.0), (21.5, 17.0)], false);
            poly(&[(2.5, 7.0), (17.0, 7.0), (17.0, 21.5)], false);
        }
        Icon::Resize => {
            poly(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)], false);
            poly(&[(10.0, 20.0), (4.0, 20.0), (4.0, 14.0)], false);
            line((20.0, 4.0), (13.5, 10.5));
            line((4.0, 20.0), (10.5, 13.5));
        }
        Icon::Undo | Icon::Reset => {
            arc(13.0, 13.0, 6.5, 200.0, 450.0);
            poly(&[(3.0, 9.5), (6.5, 14.5), (10.5, 10.0)], true);
        }
        Icon::Save => {
            poly(&[(4.5, 4.5), (16.5, 4.5), (19.5, 7.5), (19.5, 19.5), (4.5, 19.5), (4.5, 4.5)], false);
            poly(&[(8.0, 4.5), (8.0, 9.0), (15.0, 9.0), (15.0, 4.5)], false);
            p.rect_stroke(Rect::from_min_max(q(8.0, 13.0), q(16.0, 19.5)), CornerRadius::ZERO, st, egui::StrokeKind::Middle);
        }
        Icon::Folder | Icon::Open => {
            poly(&[(3.5, 6.5), (9.5, 6.5), (11.5, 8.5), (20.5, 8.5), (20.5, 18.5), (3.5, 18.5), (3.5, 6.5)], false);
            if icon == Icon::Open {
                line((12.0, 16.5), (12.0, 11.0));
                poly(&[(9.5, 13.0), (12.0, 10.5), (14.5, 13.0)], false);
            }
        }
        Icon::Plus => {
            line((12.0, 5.0), (12.0, 19.0));
            line((5.0, 12.0), (19.0, 12.0));
        }
        Icon::Close => {
            line((6.5, 6.5), (17.5, 17.5));
            line((17.5, 6.5), (6.5, 17.5));
        }
        Icon::Moon => {
            // Croissant : arc extérieur hors du disque qui « mord », puis arc intérieur.
            let (c1, r1, c2, r2) = ((12.0f32, 12.5f32), 8.0f32, (16.5f32, 8.5f32), 7.0f32);
            let d = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            let arc_pts = |c: (f32, f32), r: f32, keep: &dyn Fn((f32, f32)) -> bool| -> Vec<Pos2> {
                let mut v: Vec<Pos2> = Vec::new();
                for i in 0..72 {
                    let a = (i as f32 * 5.0).to_radians();
                    let pt = (c.0 + r * a.cos(), c.1 + r * a.sin());
                    if keep(pt) {
                        v.push(q(pt.0, pt.1));
                    }
                }
                if let Some(k) = (1..v.len()).find(|&k| v[k].distance(v[k - 1]) > 3.0 * s) {
                    v.rotate_left(k);
                }
                v
            };
            let mut pts = arc_pts(c1, r1, &|pt| d(pt, c2) >= r2);
            let mut inner = arc_pts(c2, r2, &|pt| d(pt, c1) <= r1);
            if let (Some(last), Some(f), Some(l)) = (pts.last().copied(), inner.first().copied(), inner.last().copied()) {
                if last.distance(f) > last.distance(l) {
                    inner.reverse();
                }
            }
            pts.append(&mut inner);
            if let Some(f) = pts.first().copied() {
                pts.push(f);
            }
            p.add(Shape::line(pts, st));
        }
        Icon::Sun => {
            p.circle_stroke(q(12.0, 12.0), 4.0 * s, st);
            for i in 0..8 {
                let a = (i as f32 * 45.0f32).to_radians();
                line((12.0 + 6.8 * a.cos(), 12.0 + 6.8 * a.sin()), (12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin()));
            }
        }
        Icon::More => {
            for x in [6.0, 12.0, 18.0] {
                p.circle_filled(q(x, 12.0), 1.7 * s, c);
            }
        }
        Icon::Panel => {
            p.rect_stroke(Rect::from_min_max(q(3.5, 5.0), q(20.5, 19.0)), CornerRadius::same((2.0 * s) as u8), st, egui::StrokeKind::Middle);
            line((14.5, 5.0), (14.5, 19.0));
        }
        Icon::Convert => {
            line((4.0, 8.5), (18.0, 8.5));
            poly(&[(14.5, 5.0), (18.0, 8.5), (14.5, 12.0)], false);
            line((20.0, 15.5), (6.0, 15.5));
            poly(&[(9.5, 12.0), (6.0, 15.5), (9.5, 19.0)], false);
        }
        Icon::MarkIn => {
            poly(&[(10.0, 4.0), (6.0, 4.0), (6.0, 20.0), (10.0, 20.0)], false);
            poly(&[(11.0, 7.5), (18.0, 12.0), (11.0, 16.5)], true);
        }
        Icon::MarkOut => {
            poly(&[(14.0, 4.0), (18.0, 4.0), (18.0, 20.0), (14.0, 20.0)], false);
            poly(&[(13.0, 7.5), (6.0, 12.0), (13.0, 16.5)], true);
        }
        Icon::Loop => {
            poly(&[(5.0, 13.0), (5.0, 9.0), (17.5, 9.0)], false);
            poly(&[(15.0, 6.0), (18.5, 9.0), (15.0, 12.0)], false);
            poly(&[(19.0, 11.0), (19.0, 15.0), (6.5, 15.0)], false);
            poly(&[(9.0, 12.0), (5.5, 15.0), (9.0, 18.0)], false);
        }
        Icon::Copy => {
            p.rect_stroke(Rect::from_min_max(q(8.5, 8.5), q(19.5, 19.5)), CornerRadius::same((2.0 * s) as u8), st, egui::StrokeKind::Middle);
            poly(&[(5.0, 15.5), (4.5, 15.5), (4.5, 4.5), (15.5, 4.5), (15.5, 5.0)], false);
        }
        Icon::Check => poly(&[(5.0, 12.5), (10.0, 17.5), (19.5, 7.0)], false),
        Icon::Drag => {
            for (x, y) in [(9.0, 6.0), (15.0, 6.0), (9.0, 12.0), (15.0, 12.0), (9.0, 18.0), (15.0, 18.0)] {
                p.circle_filled(q(x, y), 1.6 * s, c);
            }
        }
        Icon::Logo => {
            p.rect_filled(Rect::from_min_max(q(3.0, 3.0), q(21.0, 21.0)), CornerRadius::same((5.0 * s) as u8), c);
            let v = vec![q(9.5, 7.5), q(17.0, 12.0), q(9.5, 16.5)];
            p.add(Shape::convex_polygon(v, Color32::from_rgb(0xF5, 0xED, 0xDC), Stroke::NONE));
        }
        Icon::Clean => {
            p.rect_stroke(Rect::from_min_max(q(3.5, 5.0), q(20.5, 19.0)), CornerRadius::same((2.0 * s) as u8), st, egui::StrokeKind::Middle);
            poly(&[(8.0, 10.0), (8.0, 8.0), (10.0, 8.0)], false);
            poly(&[(16.0, 14.0), (16.0, 16.0), (14.0, 16.0)], false);
        }
    }
}

/// Couleurs d'un bouton-icône.
#[derive(Clone, Copy)]
pub struct Look {
    pub fg: Color32,
    pub bg: Color32,
    pub hover: Color32,
    pub round: bool,
}

/// Bouton-icône carré (taille `size`), icône centrée à ~55 %.
pub fn button(ui: &mut Ui, icon: Icon, size: f32, look: Look, tip: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let bg = if r.is_pointer_button_down_on() { look.hover.gamma_multiply(1.4) } else if r.hovered() { look.hover } else { look.bg };
    if bg != Color32::TRANSPARENT {
        let rr = if look.round { CornerRadius::same((size / 2.0) as u8) } else { CornerRadius::same(7) };
        ui.painter().rect_filled(rect, rr, bg);
    }
    let k = if size >= 44.0 { 0.5 } else { 0.62 };
    paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size * k)), icon, look.fg);
    if tip.is_empty() { r } else { r.on_hover_text(tip) }
}

/// Bouton large : icône + libellé (+ raccourci à droite), hauteur 36.
pub fn wide(ui: &mut Ui, icon: Icon, label: &str, key: &str, w: f32, primary: Option<(Color32, Color32)>, p: &crate::theme::Palette) -> Response {
    let h = if primary.is_some() { 40.0 } else { 36.0 };
    let enabled = ui.is_enabled();
    let (rect, r) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let (bg, fg) = match primary {
        Some((bg, fg)) => (if r.hovered() { bg.gamma_multiply(0.9) } else { bg }, fg),
        None => (if r.hovered() { p.hover_bg } else { p.code }, p.text),
    };
    let (bg, fg) = if enabled { (bg, fg) } else if primary.is_some() { (p.soft, p.muted) } else { (bg, fg.gamma_multiply(0.45)) };
    ui.painter().rect(rect, CornerRadius::same(8), bg, if primary.is_some() { Stroke::NONE } else { Stroke::new(1.0, p.border) }, egui::StrokeKind::Inside);
    let ir = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(18.0));
    paint(ui.painter(), ir, icon, fg);
    let font = if primary.is_some() { egui::FontId::new(14.0, egui::FontFamily::Name("bold".into())) } else { egui::FontId::proportional(13.5) };
    let key_w = if key.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(key.to_string(), egui::FontId::monospace(11.0), fg).size().x + 14.0 };
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), font, fg);
    job.wrap = egui::text::TextWrapping { max_width: (w - 48.0 - key_w).max(20.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(rect.left() + 38.0, rect.center().y - g.size().y / 2.0), g, fg);
    if !key.is_empty() {
        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, key, egui::FontId::monospace(11.0), fg.gamma_multiply(0.6));
    }
    r
}
