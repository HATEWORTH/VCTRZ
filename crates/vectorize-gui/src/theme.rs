//! Vectorize GUI Theme — Trilithium / brushed-metal style
//!
//! All colors, bevel drawing, and widget helpers live here.
//! To reskin the app, edit the palette constants and widget functions below.
//! The rest of the app only references items from this module.

use eframe::egui;

// ─────────────────────────────────────────────────────────────────────────────
// PALETTE
//
// Change these constants to reskin the entire UI.
// ─────────────────────────────────────────────────────────────────────────────

/// Main background (toolbar, side panel, status bar).
pub const BG_METAL:   egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x38);
/// Darker background (title bar, pressed buttons).
pub const BG_DARK:    egui::Color32 = egui::Color32::from_rgb(0x2A, 0x2A, 0x28);
/// Dark panel / inset fill (settings sections, log area).
pub const PANEL_BG:   egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1C, 0x1A);

/// Grid cell background.
pub const CELL_BG:    egui::Color32 = egui::Color32::from_rgb(0x2e, 0x32, 0x2e);
/// Grid cell fill (with tile loaded).
pub const CELL_FILL:  egui::Color32 = egui::Color32::from_rgb(0x34, 0x38, 0x34);
/// Grid cell selected highlight.
pub const CELL_SEL:   egui::Color32 = egui::Color32::from_rgb(0x4a, 0x5a, 0x4a);

/// Button face (default state).
pub const BTN_FACE:   egui::Color32 = egui::Color32::from_rgb(0x58, 0x58, 0x56);
/// Button highlight / bevel light edge.
pub const BTN_LIGHT:  egui::Color32 = egui::Color32::from_rgb(0x70, 0x70, 0x6E);
/// Button shadow / bevel dark edge.
pub const BTN_SHADOW: egui::Color32 = egui::Color32::from_rgb(0x28, 0x28, 0x26);
/// Button text — light, for dark button backgrounds.
pub const BTN_TEXT:   egui::Color32 = egui::Color32::from_rgb(0xC0, 0xC0, 0xB8);

/// Dim text (title bar, status bar, section headers) — soft white.
pub const TEXT_DIM:   egui::Color32 = egui::Color32::from_rgb(0xD0, 0xD0, 0xC8);
/// LCD / light text on dark panels — soft white.
pub const TEXT_LCD:   egui::Color32 = egui::Color32::from_rgb(0xD0, 0xD0, 0xC8);

/// Text highlight (top-left emboss edge) — medium gray.
pub const TEXT_HI:    egui::Color32 = egui::Color32::from_rgb(0x6A, 0x6A, 0x6A);
/// Text shadow (bottom-right emboss edge) — white.
pub const TEXT_SH:    egui::Color32 = egui::Color32::from_rgb(0xFF, 0xFF, 0xFF);

/// Inset border color.
pub const BORDER_IN:  egui::Color32 = egui::Color32::from_rgb(0x60, 0x60, 0x60);

// ── Late-80s Apple palette ────────────────────────────────────────────────
// Desaturated versions of the classic 6-stripe Apple logo colors,
// tuned for the platinum grey UI of System 6 / Mac II era.

/// Apple Orange (muted) — slider handles and active fills.
pub const RETRO_AMBER:  egui::Color32 = egui::Color32::from_rgb(0xC4, 0x88, 0x4D);
/// Apple Green (muted) — checkmarks and active indicators.
pub const RETRO_TEAL:   egui::Color32 = egui::Color32::from_rgb(0x6B, 0x9B, 0x5A);
/// Apple Red (muted coral) — section headers.
pub const RETRO_ROSE:   egui::Color32 = egui::Color32::from_rgb(0xB8, 0x6B, 0x6B);
/// Warm cream — label text, values. Classic Mac "warm white" feel.
pub const RETRO_CREAM:  egui::Color32 = egui::Color32::from_rgb(0xD8, 0xC8, 0xA0);
/// Apple Blue (muted) — selected toggle/button text.
pub const RETRO_PURPLE: egui::Color32 = egui::Color32::from_rgb(0x5B, 0x7B, 0xA8);
/// Apple Purple (muted) — secondary accent.
pub const RETRO_VIOLET: egui::Color32 = egui::Color32::from_rgb(0x8A, 0x6B, 0x96);
/// Apple Yellow (muted) — slider rail fill.
pub const RETRO_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xC8, 0xA8, 0x58);

/// Title bar grip dot highlight.
pub const GRIP_HI:    egui::Color32 = egui::Color32::from_rgb(0x50, 0x50, 0x4E);
/// Title bar grip dot shadow.
pub const GRIP_LO:    egui::Color32 = egui::Color32::from_rgb(0x1A, 0x1A, 0x18);

// ─────────────────────────────────────────────────────────────────────────────
// BEVEL PRIMITIVES
//
// Low-level helpers for drawing raised and sunken 3D borders.
// ─────────────────────────────────────────────────────────────────────────────

/// Draw a raised bevel (light top-left, dark bottom-right).
pub fn bevel_raised(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().line_segment([rect.left_top(), rect.right_top()],   egui::Stroke::new(1.0, BTN_LIGHT));
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], egui::Stroke::new(1.0, BTN_LIGHT));
    ui.painter().line_segment([rect.right_top(), rect.right_bottom()],  egui::Stroke::new(1.0, BTN_SHADOW));
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, BTN_SHADOW));
}

/// Draw a sunken bevel (dark top-left, light bottom-right).
pub fn bevel_sunken(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().line_segment([rect.left_top(), rect.right_top()],   egui::Stroke::new(1.0, BTN_SHADOW));
    ui.painter().line_segment([rect.left_top(), rect.left_bottom()], egui::Stroke::new(1.0, BTN_SHADOW));
    ui.painter().line_segment([rect.right_top(), rect.right_bottom()],  egui::Stroke::new(1.0, BTN_LIGHT));
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, BTN_LIGHT));
}

// ─────────────────────────────────────────────────────────────────────────────
// WIDGET HELPERS
//
// Pre-styled widgets that match the theme. All drawing is done here so that
// the app module stays layout-only.
// ─────────────────────────────────────────────────────────────────────────────

/// Raised beveled button (toolbar style).
pub fn metal_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    let desired = egui::vec2(
        ui.painter().layout_no_wrap(text.to_string(), egui::FontId::monospace(11.0), BTN_TEXT).size().x + 20.0,
        20.0,
    );
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let hovered = response.hovered() && enabled;
    let pressed = response.is_pointer_button_down_on() && enabled;

    let bg = if pressed { BG_DARK } else if hovered { BTN_LIGHT } else { BTN_FACE };
    let fg = if enabled { BTN_TEXT } else { egui::Color32::from_rgb(0xAA, 0xAA, 0xAA) };

    ui.painter().rect_filled(rect, 1.0, bg);
    if pressed { bevel_sunken(ui, rect); } else { bevel_raised(ui, rect); }

    let font = egui::FontId::monospace(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let text_pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0);
    embossed_text(ui, text_pos, text, font, fg);

    response.clicked() && enabled
}

/// Colored button variant — custom base color.
/// When enabled: base_color is the background, dark text.
/// When disabled: normal background, base_color is the text (e.g., candy red).
pub fn metal_button_colored(ui: &mut egui::Ui, text: &str, enabled: bool, base_color: egui::Color32) -> bool {
    let desired = egui::vec2(
        ui.painter().layout_no_wrap(text.to_string(), egui::FontId::monospace(11.0), BTN_TEXT).size().x + 20.0,
        20.0,
    );
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let hovered = response.hovered() && enabled;
    let pressed = response.is_pointer_button_down_on() && enabled;

    let bg = if !enabled {
        BTN_FACE
    } else if pressed {
        egui::Color32::from_rgb(
            (base_color.r() as u16 * 3 / 4) as u8,
            (base_color.g() as u16 * 3 / 4) as u8,
            (base_color.b() as u16 * 3 / 4) as u8,
        )
    } else if hovered {
        egui::Color32::from_rgb(
            (base_color.r() as u16 + 20).min(255) as u8,
            (base_color.g() as u16 + 20).min(255) as u8,
            (base_color.b() as u16 + 20).min(255) as u8,
        )
    } else {
        base_color
    };
    let fg = if enabled { BTN_TEXT } else { base_color };

    ui.painter().rect_filled(rect, 1.0, bg);
    if pressed { bevel_sunken(ui, rect); } else { bevel_raised(ui, rect); }

    let font = egui::FontId::monospace(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let text_pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0);
    embossed_text(ui, text_pos, text, font, fg);

    response.clicked() && enabled
}

/// Action button — identical style to `metal_toggle_sized` preset buttons.
/// Used in the Actions panel for OPEN, EXPORT, VECTORIZE, etc.
pub fn metal_action_button(ui: &mut egui::Ui, text: &str, enabled: bool, width: f32) -> bool {
    // Same height (18px), font (monospace 10), and bevel style as metal_toggle_sized
    let desired = egui::vec2(width, 18.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    let hovered = response.hovered() && enabled;
    let pressed = response.is_pointer_button_down_on() && enabled;

    let fg = BTN_TEXT;

    if pressed {
        ui.painter().rect_filled(rect, 0.0, BG_DARK);
        bevel_sunken(ui, rect);
    } else {
        ui.painter().rect_filled(rect, 0.0, BTN_FACE);
        bevel_raised(ui, rect);
    }

    if hovered && !pressed {
        ui.painter().rect_filled(rect.shrink(1.0), 0.0, BTN_LIGHT);
    }

    let font = egui::FontId::monospace(10.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let text_pos = egui::pos2(
        rect.center().x - galley.size().x / 2.0,
        rect.center().y - galley.size().y / 2.0,
    );
    embossed_text(ui, text_pos, text, font, fg);

    response.clicked() && enabled
}

/// Toggle / radio button (sunken when selected, raised otherwise).
/// If `fixed_width` is provided, the button uses that width instead of auto-sizing.
pub fn metal_toggle_sized(ui: &mut egui::Ui, text: &str, selected: bool, fixed_width: Option<f32>) -> bool {
    let w = fixed_width.unwrap_or_else(||
        ui.painter().layout_no_wrap(text.to_string(), egui::FontId::monospace(10.0), BTN_TEXT).size().x + 14.0
    );
    let desired = egui::vec2(w, 18.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let font = egui::FontId::monospace(10.0);
    let fg = if selected { RETRO_TEAL } else { BTN_TEXT };
    if selected {
        ui.painter().rect_filled(rect, 0.0, BG_DARK);
        bevel_sunken(ui, rect);
    } else {
        ui.painter().rect_filled(rect, 0.0, BTN_FACE);
        bevel_raised(ui, rect);
    }
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let text_pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0);
    embossed_text(ui, text_pos, text, font, fg);

    response.clicked()
}

/// Toggle / radio button with auto-sized width.
pub fn metal_toggle(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    metal_toggle_sized(ui, text, selected, None)
}

/// Toggle button with custom active text color.
pub fn metal_toggle_colored(ui: &mut egui::Ui, text: &str, selected: bool, active_color: egui::Color32) -> bool {
    let w = ui.painter().layout_no_wrap(text.to_string(), egui::FontId::monospace(10.0), BTN_TEXT).size().x + 14.0;
    let desired = egui::vec2(w, 18.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let font = egui::FontId::monospace(10.0);
    let fg = if selected { active_color } else { BTN_TEXT };
    if selected {
        ui.painter().rect_filled(rect, 0.0, BG_DARK);
        bevel_sunken(ui, rect);
    } else {
        ui.painter().rect_filled(rect, 0.0, BTN_FACE);
        bevel_raised(ui, rect);
    }
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let text_pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0);
    embossed_text(ui, text_pos, text, font, fg);

    response.clicked()
}

/// Draw a row of toggle buttons that evenly divide the available width.
/// Returns the index of the clicked button (if any).
pub fn metal_toggle_row(ui: &mut egui::Ui, labels: &[&str], selected_idx: usize) -> Option<usize> {
    let avail = ui.available_width();
    let spacing = 2.0;
    let total_spacing = spacing * (labels.len() as f32 - 1.0);
    let btn_w = (avail - total_spacing) / labels.len() as f32;
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = spacing;
        for (i, label) in labels.iter().enumerate() {
            if metal_toggle_sized(ui, label, i == selected_idx, Some(btn_w)) {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// Section header with retro rose accent.
pub fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    let font = egui::FontId::monospace(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), BTN_TEXT);
    let desired = egui::vec2(ui.available_width(), galley.size().y + 2.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let pos = egui::pos2(rect.min.x, rect.min.y);
    embossed_text(ui, pos, text, font, BTN_TEXT);
    ui.add_space(2.0);
}

/// Draw flat text (no emboss effect).
pub fn embossed_text(ui: &egui::Ui, pos: egui::Pos2, text: &str, font: egui::FontId, color: egui::Color32) {
    ui.painter().text(pos, egui::Align2::LEFT_TOP, text, font, color);
}

/// Dark inset panel with sunken 3D bevel (used for settings groups).
/// Stretches to fill available width with 6px horizontal padding.
pub fn inset_frame(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let full_width = ui.available_width();
    let inner_width = full_width - 12.0; // 6px padding each side
    let frame = egui::Frame::new()
        .fill(PANEL_BG)
        .inner_margin(egui::Margin::symmetric(6, 4));
    let resp = frame.show(ui, |ui| {
        ui.set_min_width(inner_width);
        ui.set_max_width(inner_width);
        add_contents(ui);
    });
    bevel_sunken(ui, resp.response.rect);
}

/// Checkbox in the Trilithium style with sunken bevel.
pub fn metal_checkbox(ui: &mut egui::Ui, checked: &mut bool, text: &str) {
    metal_checkbox_tip(ui, checked, text, "");
}

pub fn metal_checkbox_tip(ui: &mut egui::Ui, checked: &mut bool, text: &str, tip: &str) {
    metal_checkbox_tip_enabled(ui, checked, text, tip, true);
}

pub fn metal_checkbox_tip_enabled(ui: &mut egui::Ui, checked: &mut bool, text: &str, tip: &str, enabled: bool) {
    let desired = egui::vec2(ui.available_width(), 16.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(desired, sense);
    if enabled && response.clicked() { *checked = !*checked; }

    let box_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.center().y - 6.0), egui::vec2(12.0, 12.0));
    let dim_gray = egui::Color32::from_rgb(0x55, 0x55, 0x50);
    let box_bg = if !enabled {
        egui::Color32::from_rgb(0x30, 0x30, 0x2E)
    } else if *checked {
        egui::Color32::from_rgb(0x2E, 0x3A, 0x2A)
    } else {
        BTN_FACE
    };
    ui.painter().rect_filled(box_rect, 0.0, box_bg);
    bevel_sunken(ui, box_rect);

    if *checked {
        let check_color = if enabled { RETRO_TEAL } else { dim_gray };
        let pts = [
            egui::pos2(box_rect.min.x + 2.0, box_rect.center().y),
            egui::pos2(box_rect.center().x - 1.0, box_rect.max.y - 2.0),
            egui::pos2(box_rect.max.x - 1.0, box_rect.min.y + 2.0),
        ];
        ui.painter().line_segment([pts[0], pts[1]], egui::Stroke::new(2.0, check_color));
        ui.painter().line_segment([pts[1], pts[2]], egui::Stroke::new(2.0, check_color));
    }

    let text_pos = egui::pos2(box_rect.max.x + 6.0, rect.center().y - 5.0);
    let label_color = if !enabled { dim_gray } else if *checked { RETRO_TEAL } else { RETRO_CREAM };
    embossed_text(ui, text_pos, text, egui::FontId::monospace(10.0), label_color);

    if !tip.is_empty() {
        response.on_hover_text_at_pointer(tip);
    }
}

/// Raised header bar (for "SETTINGS", "PAINT TOOLS" titles).
pub fn panel_header(ui: &mut egui::Ui, text: &str) {
    let hdr_frame = egui::Frame::new().fill(BG_METAL)
        .inner_margin(egui::Margin::symmetric(0, 3));
    let hdr_resp = hdr_frame.show(ui, |ui| {
        let font = egui::FontId::monospace(11.0);
        let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), TEXT_DIM);
        let desired = egui::vec2(ui.available_width(), galley.size().y);
        let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
        let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.min.y);
        embossed_text(ui, pos, text, font, TEXT_DIM);
    });
    bevel_raised(ui, hdr_resp.response.rect);
}

/// A labeled slider row for 0-100 u8 values.
/// Shows label on the left in TEXT_LCD, slider on the right.
/// Returns true if the value changed.
pub fn metal_slider_u8(ui: &mut egui::Ui, label: &str, value: &mut u8) -> bool {
    let old = *value;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(TEXT_LCD).monospace().size(11.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().slider_width = 120.0;
            let mut v = *value as i32;
            ui.add(egui::Slider::new(&mut v, 0..=100).show_value(true).text(""));
            *value = v as u8;
        });
    });
    *value != old
}

// ─────────────────────────────────────────────────────────────────────────────
// EGUI STYLE
//
// Call this once at startup to apply the theme to egui's built-in widgets
// (DragValue, TextEdit, ScrollArea, etc.).
// ─────────────────────────────────────────────────────────────────────────────

/// Apply the Trilithium palette to egui's global style.
pub fn apply_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.override_font_id = Some(egui::FontId::monospace(11.0));
    style.spacing.item_spacing = egui::vec2(4.0, 3.0);
    style.spacing.button_padding = egui::vec2(6.0, 2.0);

    style.visuals.window_fill = BG_METAL;
    style.visuals.panel_fill = BG_METAL;
    style.visuals.extreme_bg_color = PANEL_BG;
    // Slider: dark grey rail, knob color is overridden per-slider via the accent bar.
    // All rounding = 0 for hard sharp edges.
    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(0x30, 0x30, 0x2E); // dark grey rail
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(0x30, 0x30, 0x2E);
    style.visuals.widgets.inactive.fg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::ZERO;
    style.visuals.widgets.inactive.expansion = 0.0;
    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(0x40, 0x40, 0x3E);
    style.visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(0x40, 0x40, 0x3E);
    style.visuals.widgets.hovered.fg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::ZERO;
    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(0x50, 0x50, 0x4E);
    style.visuals.widgets.active.weak_bg_fill = egui::Color32::from_rgb(0x50, 0x50, 0x4E);
    style.visuals.widgets.active.fg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.active.bg_stroke = egui::Stroke::NONE;
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::ZERO;
    style.visuals.widgets.noninteractive.bg_fill = PANEL_BG;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, RETRO_CREAM);
    style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::ZERO;
    style.visuals.selection.bg_fill = egui::Color32::from_rgb(0x4A, 0x5A, 0x70);
    style.visuals.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
    // Kill all rounding globally
    style.visuals.window_corner_radius = egui::CornerRadius::ZERO;
    style.visuals.menu_corner_radius = egui::CornerRadius::ZERO;

    ctx.set_style(style);
}
