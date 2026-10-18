//! Window chrome for the frameless window: drag areas, caption buttons and
//! resize borders.

use eframe::egui::{
    self, Color32, CursorIcon, Rect, ResizeDirection, Sense, Ui, ViewportCommand, pos2, vec2,
};

use super::icons;
use crate::theme::Theme;

/// Height of the title bar row (sidebar header, tabs, caption buttons).
pub const TITLEBAR_H: f32 = 44.0;
const CAPTION_W: f32 = 46.0;
const RESIZE_BORDER: f32 = 5.0;

/// Makes `resp` behave like a native title bar: drag to move, double-click to maximize.
pub fn window_drag(ui: &Ui, resp: &egui::Response) {
    if resp.double_clicked() {
        let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

pub fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Width needed for the minimize/maximize/close buttons.
pub fn caption_buttons_width() -> f32 {
    CAPTION_W * 3.0
}

/// Windows-style caption buttons, right-aligned in `bar`.
pub fn caption_buttons(ui: &mut Ui, theme: &Theme, bar: Rect) {
    let ctx = ui.ctx().clone();
    let maximized = is_maximized(&ctx);
    let mut x = bar.right() - CAPTION_W * 3.0;
    for kind in 0..3 {
        let rect = Rect::from_min_size(pos2(x, bar.top()), vec2(CAPTION_W, bar.height()));
        x += CAPTION_W;
        let resp = ui.interact(rect, egui::Id::unique(("caption", kind)), Sense::click());
        let painter = ui.painter();
        let mut hovered = resp.hovered();
        if kind == 1 {
            // Windows handles this button itself (snap layouts), see win32.rs.
            crate::win32::set_maximize_button(rect, ctx.pixels_per_point());
            hovered |= crate::win32::maximize_hovered();
        }
        let (bg, fg) = match (kind, hovered) {
            (2, true) => (Color32::from_rgb(196, 43, 28), Color32::WHITE),
            (_, true) => (theme.hover.gamma_multiply(1.5), theme.foreground),
            _ => (Color32::TRANSPARENT, theme.text_muted),
        };
        painter.rect_filled(rect, 0.0, bg);
        let c = rect.center();
        match kind {
            0 => icons::minimize(painter, c, fg),
            1 => icons::maximize(painter, c, fg, maximized),
            _ => icons::close(painter, c, 10.0, fg),
        }
        if resp.clicked() {
            match kind {
                0 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                1 => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                _ => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
        resp.on_hover_text(["Minimieren", if maximized { "Verkleinern" } else { "Maximieren" }, "Schließen"][kind]);
    }
}

/// Resizing from the window edges, since the frameless window has no border.
pub fn resize_borders(ctx: &egui::Context) {
    if is_maximized(ctx) {
        return;
    }
    let screen = ctx.content_rect();
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let b = RESIZE_BORDER;
    let left = pos.x <= screen.left() + b;
    let right = pos.x >= screen.right() - b;
    let top = pos.y <= screen.top() + b;
    let bottom = pos.y >= screen.bottom() - b;
    let corner = 12.0;
    let near_left = pos.x <= screen.left() + corner;
    let near_right = pos.x >= screen.right() - corner;
    let near_top = pos.y <= screen.top() + corner;
    let near_bottom = pos.y >= screen.bottom() - corner;
    let dir = match () {
        _ if (top && near_left) || (left && near_top) => ResizeDirection::NorthWest,
        _ if (top && near_right) || (right && near_top) => ResizeDirection::NorthEast,
        _ if (bottom && near_left) || (left && near_bottom) => ResizeDirection::SouthWest,
        _ if (bottom && near_right) || (right && near_bottom) => ResizeDirection::SouthEast,
        _ if left => ResizeDirection::West,
        _ if right => ResizeDirection::East,
        _ if top => ResizeDirection::North,
        _ if bottom => ResizeDirection::South,
        _ => return,
    };
    let icon = match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}
