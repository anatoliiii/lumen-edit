//! Small vector icons painted directly, so they stay crisp at any scale and
//! do not depend on which symbols the fonts happen to have.

use eframe::egui::{Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use super::util;

const FOLDER: Color32 = Color32::from_rgb(214, 170, 92);

/// Folder centered at `center`; open folders tilt their front flap.
pub fn folder(painter: &Painter, center: Pos2, open: bool) {
    let left = center.x - 7.0;
    let right = center.x + 7.0;
    let top = center.y - 5.0;
    let bottom = center.y + 5.5;
    // Tab.
    painter.add(Shape::convex_polygon(
        vec![pos2(left, top), pos2(left + 5.0, top), pos2(left + 6.5, top + 1.8), pos2(left, top + 1.8)],
        FOLDER.gamma_multiply(0.85),
        Stroke::NONE,
    ));
    // Back.
    painter.rect_filled(
        Rect::from_min_max(pos2(left, top + 1.5), pos2(right, bottom)),
        CornerRadius::same(2),
        FOLDER.gamma_multiply(if open { 0.55 } else { 0.9 }),
    );
    if open {
        // Front flap, slanted.
        painter.add(Shape::convex_polygon(
            vec![pos2(left + 2.0, top + 4.0), pos2(right + 1.0, top + 4.0), pos2(right - 1.5, bottom), pos2(left, bottom)],
            FOLDER,
            Stroke::NONE,
        ));
    }
}

/// File page with a folded corner, tinted by type, with a short label.
pub fn file(painter: &Painter, center: Pos2, path: &str) {
    let (label, color) = util::path_icon(path);
    let left = center.x - 5.5;
    let right = center.x + 5.5;
    let top = center.y - 7.0;
    let bottom = center.y + 7.0;
    let fold = 3.5;
    let outline = vec![pos2(left, top), pos2(right - fold, top), pos2(right, top + fold), pos2(right, bottom), pos2(left, bottom)];
    painter.add(Shape::convex_polygon(outline.clone(), color.gamma_multiply(0.16), Stroke::NONE));
    painter.add(Shape::closed_line(outline, Stroke::new(1.0, color.gamma_multiply(0.85))));
    painter.add(Shape::line(
        vec![pos2(right - fold, top), pos2(right - fold, top + fold), pos2(right, top + fold)],
        Stroke::new(1.0, color.gamma_multiply(0.85)),
    ));
    let label = label.trim();
    if !label.is_empty() && label != "·" {
        let size = if label.chars().count() > 1 { 7.0 } else { 8.5 };
        painter.text(pos2(center.x - 0.3, center.y + 1.5), eframe::egui::Align2::CENTER_CENTER, label, FontId::monospace(size), color);
    } else {
        // Plain text lines for files without a known type.
        for i in 0..3 {
            let y = center.y - 1.0 + i as f32 * 2.6;
            painter.line_segment([pos2(left + 2.5, y), pos2(right - 2.5, y)], Stroke::new(1.0, color.gamma_multiply(0.7)));
        }
    }
}

/// Disclosure chevron: pointing right when closed, down when open.
pub fn chevron(painter: &Painter, center: Pos2, open: bool, color: Color32) {
    let points = if open {
        vec![center + vec2(-3.5, -1.8), center + vec2(3.5, -1.8), center + vec2(0.0, 2.6)]
    } else {
        vec![center + vec2(-1.8, -3.5), center + vec2(2.6, 0.0), center + vec2(-1.8, 3.5)]
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}
