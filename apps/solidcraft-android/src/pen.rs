//! Pen mode, for a stylus. A phone without a pen layer in its screen (the Fold 8) only takes
//! capacitive styluses, which it sees as a finger, so the app can't tell pen from finger by
//! itself. Pen mode settles it by agreement: one touch is the pen, which selects, draws and drags
//! but never turns the view, and fingers navigate in twos and threes (two turn the view or
//! pinch-zoom, whichever they start with; three pan). A round button at the screen's edge switches
//! it; with it on, Undo and Redo buttons stand below it, since three fingers pan instead of
//! swiping. The buttons can be dragged anywhere, and egui keeps where they were left.

use egui::{Align2, Color32, CornerRadius, Pos2, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

/// The settings key for pen mode: "1" on.
pub const KEY: &str = "solidcraft_android_pen";

/// A button's size (points): a fingertip's.
const SIZE: f32 = 44.0;

/// What the buttons were tapped for this frame.
#[derive(Default)]
pub struct Asked {
    pub toggle: bool,
    pub undo: bool,
    pub redo: bool,
}

#[derive(Clone, Copy)]
enum Glyph {
    Pen,
    Undo,
    Redo,
}

/// The pen button (lit in pen mode), and Undo and Redo below it in pen mode.
pub fn buttons(ctx: &egui::Context, pen: bool) -> Asked {
    let mut asked = Asked::default();
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("solidcraft_android_pen"))
        .movable(true)
        .constrain(true)
        .pivot(Align2::RIGHT_TOP)
        .default_pos(pos2(screen.right() - 12.0, screen.top() + screen.height() * 0.45))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 8.0);
            ui.vertical(|ui| {
                asked.toggle = button(ui, pen, Glyph::Pen).clicked();
                if pen {
                    asked.undo = button(ui, false, Glyph::Undo).clicked();
                    asked.redo = button(ui, false, Glyph::Redo).clicked();
                }
            });
        });
    asked
}

fn button(ui: &mut egui::Ui, on: bool, glyph: Glyph) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SIZE), Sense::click());
    let visuals = ui.visuals();
    let (fill, ink) = if on {
        (visuals.selection.bg_fill, visuals.selection.stroke.color)
    } else if response.is_pointer_button_down_on() {
        (visuals.widgets.active.weak_bg_fill, visuals.widgets.active.fg_stroke.color)
    } else {
        (visuals.window_fill, visuals.text_color())
    };
    let painter = ui.painter();
    let round = CornerRadius::same((SIZE / 2.0) as u8);
    painter.rect_filled(rect, round, fill);
    painter.rect_stroke(rect, round, visuals.widgets.inactive.bg_stroke, StrokeKind::Inside);
    draw(painter, rect.center(), glyph, ink);
    response
}

/// The glyphs, drawn: a pen lying with its tip at the bottom left, and a bent arrow back to the
/// left (Undo) or on to the right (Redo).
fn draw(painter: &egui::Painter, c: Pos2, glyph: Glyph, ink: Color32) {
    match glyph {
        Glyph::Pen => {
            let back = c + vec2(9.0, -9.0);
            let neck = c + vec2(-4.0, 4.0);
            let tip = c + vec2(-10.0, 10.0);
            let across = vec2(1.0, 1.0).normalized() * 3.5;
            painter.add(Shape::convex_polygon(vec![back + across, back - across, neck - across, neck + across], ink, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![neck + across, neck - across, tip], ink, Stroke::NONE));
        }
        Glyph::Undo | Glyph::Redo => {
            let flip = if matches!(glyph, Glyph::Redo) { -1.0 } else { 1.0 };
            let centre = c + vec2(0.0, 3.0);
            let r = 8.0;
            // Over the top from one side to the other, then the head pointing down.
            let arc: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 12.0;
                    centre + vec2(flip * r * a.cos(), -r * a.sin())
                })
                .collect();
            painter.add(Shape::line(arc, Stroke::new(2.5, ink)));
            let end = centre + vec2(-flip * r, 0.0);
            painter.add(Shape::convex_polygon(vec![end + vec2(-4.5, -1.0), end + vec2(4.5, -1.0), end + vec2(0.0, 6.0)], ink, Stroke::NONE));
        }
    }
}
