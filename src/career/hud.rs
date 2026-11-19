//! The career's HUD: money and trust in the top left with the job under way and the keys,
//! messages in the top middle, and the settings window (2: driving aids, wheel and
//! pedals, controls).

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, RichText};
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::{Career, Scene, euros};
use crate::car::DriverAids;
use crate::ui::{WheelPanel, credits, toggle_key};

/// How long a message stays up (s).
const NEWS_TIME: f32 = 6.0;
const PANEL: Color32 = Color32::from_rgba_premultiplied(10, 12, 16, 200);
const GOLD: Color32 = Color32::from_rgb(255, 200, 60);
const DIM: Color32 = Color32::from_rgb(170, 174, 182);

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<News>()
            .init_resource::<Status>()
            .init_resource::<Settings>()
            .add_systems(Update, (age_news, toggle_settings))
            .add_systems(EguiPrimaryContextPass, (hud, news, settings));
    }
}

/// Messages for the player, newest last, with the time each has left.
#[derive(Resource, Default)]
pub struct News(Vec<(String, f32)>);

impl News {
    pub fn say(&mut self, text: impl Into<String>) {
        self.0.push((text.into(), NEWS_TIME));
    }
}

/// Lines about what's under way (a delivery, the tow truck), shown under the money.
#[derive(Resource, Default)]
pub struct Status(pub Vec<String>);

/// Whether the settings window is open.
#[derive(Resource, Default)]
struct Settings(bool);

fn age_news(time: Res<Time>, mut news: ResMut<News>) {
    for (_, left) in &mut news.0 {
        *left -= time.delta_secs();
    }
    news.0.retain(|(_, left)| *left > 0.0);
}

fn toggle_settings(
    keys: Res<ButtonInput<KeyCode>>,
    egui: Option<Res<EguiWantsInput>>,
    mut settings: ResMut<Settings>,
) {
    if toggle_key(&keys, egui.as_deref(), KeyCode::Digit2) {
        settings.0 = !settings.0;
    }
}

fn hud(
    mut contexts: EguiContexts,
    career: Res<Career>,
    status: Res<Status>,
    scene: Res<State<Scene>>,
) -> Result {
    egui::Area::new(egui::Id::new("career hud"))
        .anchor(Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            egui::Frame::new()
                .fill(PANEL)
                .corner_radius(6.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(euros(career.money))
                            .font(FontId::proportional(26.0))
                            .color(GOLD),
                    );
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Dealer's trust").color(DIM));
                        ui.add(
                            egui::ProgressBar::new(career.trust / 100.0)
                                .desired_width(110.0)
                                .text(format!("{:.0}", career.trust)),
                        );
                    });
                    for line in &status.0 {
                        let text = RichText::new(line).color(Color32::WHITE);
                        ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend));
                    }
                    let keys = match scene.get() {
                        Scene::Driving => "K map   T tow home   C view   2 settings",
                        Scene::Garage => "2 settings",
                        Scene::Dyno => "Drag: look around   M mute   2 settings",
                    };
                    ui.label(RichText::new(keys).small().color(DIM));
                });
        });
    Ok(())
}

fn news(mut contexts: EguiContexts, news: Res<News>) -> Result {
    if news.0.is_empty() {
        return Ok(());
    }
    egui::Area::new(egui::Id::new("career news"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
        .interactable(false)
        .show(contexts.ctx_mut()?, |ui| {
            for (text, left) in &news.0 {
                let alpha = (left / 1.0).clamp(0.0, 1.0);
                egui::Frame::new()
                    .fill(PANEL.gamma_multiply(alpha))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(text)
                                .font(FontId::proportional(17.0))
                                .color(Color32::WHITE.gamma_multiply(alpha)),
                        );
                    });
                ui.add_space(4.0);
            }
        });
    Ok(())
}

/// Driving aids, the wheel and pedals, the controls.
fn settings(
    mut contexts: EguiContexts,
    mut open: ResMut<Settings>,
    mut wheel_panel: WheelPanel,
    mut aids: ResMut<DriverAids>,
) -> Result {
    if !open.0 {
        return Ok(());
    }
    egui::Window::new("Settings  (2)")
        .open(&mut open.0)
        .default_pos([12.0, 160.0])
        .default_width(360.0)
        .vscroll(true)
        .show(contexts.ctx_mut()?, |ui| {
            ui.horizontal(|ui| aids.show(ui));
            ui.collapsing("Wheel and pedals", |ui| wheel_panel.show(ui));
            ui.collapsing("Controls", |ui| {
                ui.label("W/S or arrows: throttle, brake    A/D: steer");
                ui.label("Q/E: shift down/up    G: automatic gearbox    Space: parking brake");
                ui.label("I: start the engine (after stalling with a clutch pedal)");
                ui.label("Stop on a glowing marker to go into a place");
                ui.label("T: call the tow truck home    K: map    C: chase/driver view");
                ui.label("M: mute    Left-drag: look around    Scroll: zoom");
            });
            credits(ui);
        });
    Ok(())
}
