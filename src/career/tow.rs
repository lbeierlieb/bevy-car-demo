//! Towing home (T): stuck, out of fuel or crashed, the tow truck brings the player and
//! their car into the garage, for a call-out fee and a price per kilometre. Without the
//! money, mom comes to fetch them.

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::hud::News;
use super::places::PlaceKind;
use super::{Career, Places, Scene, driven, euros};
use crate::car::{ActiveCar, Car};

/// What the tow truck costs: to come out, and per kilometre to home (€).
const CALL_OUT: f64 = 50.0;
const PER_KM: f64 = 2.0;

pub struct TowPlugin;

impl Plugin for TowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TowAsked>()
            .add_systems(OnEnter(Scene::Garage), |mut asked: ResMut<TowAsked>| {
                asked.0 = false;
            })
            .add_systems(Update, ask.run_if(in_state(Scene::Driving)))
            .add_systems(
                EguiPrimaryContextPass,
                window.run_if(in_state(Scene::Driving)),
            );
    }
}

/// Whether the player asked for the tow truck (the window is open).
#[derive(Resource, Default)]
struct TowAsked(bool);

fn ask(keys: Res<ButtonInput<KeyCode>>, mut asked: ResMut<TowAsked>) {
    if keys.just_pressed(KeyCode::KeyT) {
        asked.0 = !asked.0;
    }
}

/// What towing the car being driven home costs.
fn price(car: &Car, places: &Places) -> f64 {
    let home = places.of(PlaceKind::Home).at;
    let km = f64::from(car.body.position.xz().distance(home)) / 1000.0;
    CALL_OUT + PER_KM * km
}

#[allow(clippy::too_many_arguments)]
fn window(
    mut contexts: EguiContexts,
    mut asked: ResMut<TowAsked>,
    mut career: ResMut<Career>,
    mut news: ResMut<News>,
    places: Res<Places>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut scene: ResMut<NextState<Scene>>,
) -> Result {
    if !asked.0 {
        return Ok(());
    }
    let Some(car) = driven(&cars, &active) else {
        return Ok(());
    };
    let price = price(car, &places);
    let mut home = None;
    egui::Window::new("Tow truck")
        .anchor(Align2::CENTER_CENTER, egui::vec2(0.0, -60.0))
        .collapsible(false)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label("The tow truck brings you and your car home, into the garage.");
            if active.0 != super::PLAYER {
                ui.label("The car you're delivering goes back to the dealer.");
            }
            if career.money >= price {
                let text = format!("Call the tow truck: {}", euros(price));
                if ui.button(RichText::new(text).strong()).clicked() {
                    home = Some(price);
                }
            } else {
                ui.label(format!(
                    "The tow truck costs {}, more than you have.",
                    euros(price)
                ));
                if ui
                    .button(RichText::new("Mommy, please fetch me").strong())
                    .clicked()
                {
                    home = Some(0.0);
                }
            }
            if ui.button("Never mind  (T)").clicked() {
                asked.0 = false;
            }
        });
    if let Some(paid) = home {
        asked.0 = false;
        if paid > 0.0 {
            career.money -= paid;
            news.say(format!("The tow truck brought you home ({}).", euros(paid)));
        } else {
            news.say("Mom came to fetch you. She's not thrilled.");
        }
        scene.set(Scene::Garage);
    }
    Ok(())
}
