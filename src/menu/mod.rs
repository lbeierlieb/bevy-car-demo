//! The menus' common ground: how big everything is (from the window's size, not the
//! display's scaling, times the player's "UI size"), the fonts and the style.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, egui};

use crate::car::SETTINGS;

pub mod central;
pub mod kit;
pub mod nav;

/// The window size the UI is laid out for, in points: in a window this many pixels (or a
/// multiple of it) one point is one pixel (or that multiple), whatever the display's
/// scaling says. A 4K screen shows the same as a 720p one, only sharper.
pub const LAYOUT: Vec2 = Vec2::new(1280.0, 720.0);
/// The range of the player's "UI size".
pub const SIZES: std::ops::RangeInclusive<f32> = 0.7..=1.5;
/// Fewest pixels per point, so a tiny window stays readable.
const LEAST_PPP: f32 = 0.6;

/// The highlight colour: the focus, keys in hints, what's picked.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(255, 184, 64);
/// The panels' background.
pub const PANEL: egui::Color32 = egui::Color32::from_rgb(15, 17, 21);
/// The font family of titles and menu rows (Ubuntu Medium).
pub const MEDIUM: &str = "medium";

/// The scale, fonts and style of every egui window in a driving app.
pub struct LookPlugin;

impl Plugin for LookPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(nav::NavPlugin)
            .insert_resource(Display::load())
            .add_systems(Update, (scale, save_display));
    }
}

/// The display settings kept between runs (`~/.config/car-sim/settings/display.cfg`).
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct Display {
    /// The player's scale on top of the window's (1: as laid out).
    pub ui_size: f32,
}

impl Default for Display {
    fn default() -> Self {
        Self { ui_size: 1.0 }
    }
}

impl Display {
    fn load() -> Self {
        let mut display = Self::default();
        for line in SETTINGS.load("display").unwrap_or_default().lines() {
            if let Some(("ui_size", size)) = line.split_once(' ')
                && let Ok(size) = size.trim().parse::<f32>()
                && size.is_finite()
            {
                display.ui_size = size.clamp(*SIZES.start(), *SIZES.end());
            }
        }
        display
    }

    fn save(&self) {
        if let Err(e) = SETTINGS.save("display", &format!("ui_size {}\n", self.ui_size)) {
            warn!("Couldn't save the display settings: {e}");
        }
    }
}

fn save_display(display: Res<Display>) {
    if display.is_changed() && !display.is_added() {
        display.save();
    }
}

/// Pixels per point for a window of this many physical pixels.
pub fn pixels_per_point(physical: Vec2, ui_size: f32) -> f32 {
    ((physical / LAYOUT).min_element() * ui_size).max(LEAST_PPP)
}

/// Sets egui's zoom so a point is `pixels_per_point` pixels, over the display's own
/// scaling (which egui takes as its native pixels per point); the fonts and style once.
fn scale(
    mut contexts: EguiContexts,
    windows: Query<&Window, With<PrimaryWindow>>,
    display: Res<Display>,
    mut styled: Local<bool>,
) -> Result {
    let Ok(window) = windows.single() else {
        return Ok(());
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    if !*styled {
        *styled = true;
        style(ctx);
    }
    let physical = Vec2::new(
        window.resolution.physical_width() as f32,
        window.resolution.physical_height() as f32,
    );
    if physical.min_element() < 1.0 {
        return Ok(());
    }
    let zoom = pixels_per_point(physical, display.ui_size) / window.scale_factor();
    if (ctx.zoom_factor() - zoom).abs() > 1e-4 {
        ctx.set_zoom_factor(zoom);
    }
    Ok(())
}

/// A point in the window's logical pixels (as `Camera::world_to_viewport` gives it) in
/// egui's points.
pub fn to_points(ctx: &egui::Context, logical: Vec2) -> egui::Pos2 {
    let p = logical / ctx.zoom_factor();
    egui::pos2(p.x, p.y)
}

/// Ubuntu Regular for text (egui's own Ubuntu Light is too thin over the scene), Ubuntu
/// Medium for titles and menu rows, and a style with room to aim at things.
fn style(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "regular".into(),
        FontData::from_static(include_bytes!("../../assets/fonts/Ubuntu-R.ttf")).into(),
    );
    fonts.font_data.insert(
        MEDIUM.into(),
        FontData::from_static(include_bytes!("../../assets/fonts/Ubuntu-M.ttf")).into(),
    );
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "regular".into());
    fonts.families.insert(
        FontFamily::Name(MEDIUM.into()),
        std::iter::once(MEDIUM.to_string())
            .chain(fallbacks)
            .collect(),
    );
    ctx.set_fonts(fonts);
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    ctx.all_styles_mut(|style| {
        let medium = FontFamily::Name(MEDIUM.into());
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.5)),
            (TextStyle::Button, FontId::proportional(14.5)),
            (TextStyle::Heading, FontId::new(21.0, medium)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(8.0, 6.0);
        spacing.button_padding = egui::vec2(10.0, 4.0);
        spacing.interact_size = egui::vec2(40.0, 26.0);
        spacing.slider_width = 170.0;
        spacing.icon_width = 18.0;
        spacing.icon_width_inner = 10.0;
        spacing.window_margin = egui::Margin::same(12);
        spacing.combo_width = 160.0;
        let visuals = &mut style.visuals;
        visuals.window_fill = PANEL;
        visuals.panel_fill = PANEL;
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(60));
        visuals.selection.bg_fill = egui::Color32::from_rgb(150, 98, 20);
        visuals.selection.stroke = egui::Stroke::new(1.5, ACCENT);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = egui::CornerRadius::same(6);
        }
        visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(48);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_gray(48);
        visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_gray(66);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_gray(66);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ui_scales_with_the_window_not_the_display() {
        // The layout size: a point a pixel; 4K: three pixels, whatever the scaling.
        assert_eq!(pixels_per_point(LAYOUT, 1.0), 1.0);
        assert_eq!(pixels_per_point(Vec2::new(3840.0, 2160.0), 1.0), 3.0);
        // Ultra-wide: the height decides; narrow: the width.
        assert_eq!(pixels_per_point(Vec2::new(3440.0, 1440.0), 1.0), 2.0);
        assert_eq!(pixels_per_point(Vec2::new(1024.0, 768.0), 1.0), 0.8);
        assert_eq!(pixels_per_point(Vec2::new(1920.0, 1080.0), 1.2), 1.5 * 1.2);
        assert_eq!(pixels_per_point(Vec2::new(320.0, 200.0), 1.0), LEAST_PPP);
    }
}
