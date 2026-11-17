//! The window of the place the car is stopped at: home (into the garage), the gas station
//! (fill up), the pizzeria and the car dealer (jobs, cars), the parts shop.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::hud::News;
use super::jobs::{Jobs, take_offer};
use super::places::{AtPlace, Place, PlaceKind, Places};
use super::{Career, Scene, euros};
use crate::car::{ActiveCar, Car};
use crate::physics::fuel::Fuel;
use crate::terrain::Terrain;

const WARNING: egui::Color32 = egui::Color32::from_rgb(255, 170, 60);

/// What fuel costs (€ per litre).
pub const FUEL_PRICE: f64 = 1.85;

pub struct MenusPlugin;

impl Plugin for MenusPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, forget_offers).add_systems(
            EguiPrimaryContextPass,
            place_window.run_if(in_state(Scene::Driving)),
        );
    }
}

/// What the places' windows work with.
#[derive(SystemParam)]
pub struct Visit<'w, 's> {
    pub career: ResMut<'w, Career>,
    pub news: ResMut<'w, News>,
    pub active: Res<'w, ActiveCar>,
    pub cars: Query<'w, 's, &'static mut Car>,
    pub scene: ResMut<'w, NextState<Scene>>,
    pub jobs: ResMut<'w, Jobs>,
    pub terrain: Res<'w, Terrain>,
    pub commands: Commands<'w, 's>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
}

impl Visit<'_, '_> {
    /// The car being driven.
    pub fn car(&mut self) -> Option<Mut<'_, Car>> {
        let active = self.active.0;
        self.cars.iter_mut().find(|car| car.index == active)
    }

    /// Starts the job on offer.
    fn take_job(&mut self) {
        take_offer(
            &mut self.jobs,
            &mut self.commands,
            &mut self.meshes,
            &mut self.materials,
            &self.terrain,
        );
    }
}

/// A job on offer is only on offer while the player is at the place.
fn forget_offers(at: Res<AtPlace>, mut jobs: ResMut<Jobs>) {
    if at.is_changed() {
        jobs.offer = None;
    }
}

fn place_window(
    mut contexts: EguiContexts,
    at: Res<AtPlace>,
    places: Res<Places>,
    mut visit: Visit,
) -> Result {
    let Some(place) = at.0.map(|i| &places.list[i]) else {
        return Ok(());
    };
    egui::Window::new(place.name)
        .id(egui::Id::new("place"))
        .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
        .collapsible(false)
        .resizable(false)
        .min_width(320.0)
        .show(contexts.ctx_mut()?, |ui| match place.kind {
            PlaceKind::Home => home(ui, &mut visit),
            PlaceKind::GasStation => gas_station(ui, &mut visit),
            PlaceKind::Pizzeria => pizzeria(ui, &mut visit, place),
            _ => {
                ui.label("Closed for now.");
            }
        });
    Ok(())
}

fn home(ui: &mut egui::Ui, visit: &mut Visit) {
    ui.label("Home: the garage, with mom's car and yours.");
    if visit.jobs.current.is_some() {
        ui.colored_label(WARNING, "Going in now gives up the job you're on.");
    }
    if ui.button("Into the garage").clicked() {
        visit.scene.set(Scene::Garage);
    }
}

/// Filling up: as much as fits, or as much as the money buys.
fn gas_station(ui: &mut egui::Ui, visit: &mut Visit) {
    let money = visit.career.money;
    let mut paid = None;
    if let Some(mut car) = visit.car() {
        let (litres, tank) = (car.fuel.litres(), car.setup.tank.litres);
        ui.label(format!(
            "{litres:.1} of {tank:.0} L in the tank. Fuel: €{FUEL_PRICE:.2} a litre."
        ));
        let missing = (tank - litres).max(0.0);
        if missing < 0.5 {
            ui.label("The tank is full.");
            return;
        }
        let affordable = (money / FUEL_PRICE) as f32;
        let buy = missing.min(affordable);
        let price = f64::from(buy) * FUEL_PRICE;
        let text = if buy < missing {
            format!("Put in {buy:.1} L for {} (all the money)", euros(price))
        } else {
            format!("Fill up: {buy:.1} L for {}", euros(price))
        };
        let button = egui::Button::new(RichText::new(text).strong());
        if ui.add_enabled(buy > 0.1, button).clicked() {
            car.fuel = Fuel::new(litres + buy);
            car.load_fuel();
            paid = Some((buy, price));
        }
    }
    if let Some((litres, price)) = paid {
        visit.career.money -= price;
        visit
            .news
            .say(format!("Filled up {litres:.1} L for {}.", euros(price)));
    }
}

/// A pizza to deliver: where to, what it pays, how long there is.
fn pizzeria(ui: &mut egui::Ui, visit: &mut Visit, place: &Place) {
    if visit.jobs.current.is_some() {
        ui.label("Deliver the one you have first!");
        return;
    }
    if visit.jobs.offer.is_none() {
        visit.jobs.offer = visit.jobs.pizza(&visit.terrain, place.at);
    }
    let Some(offer) = &visit.jobs.offer else {
        ui.label("No orders right now.");
        return;
    };
    let time = offer.time.unwrap_or(0.0);
    ui.label(format!(
        "A pizza for a house {:.1} km away (as the crow flies).",
        offer.distance / 1000.0
    ));
    ui.label(format!(
        "{} for it, more with a tip if it's there within {}:{:02} and in one piece.",
        euros(offer.pay),
        (time / 60.0) as u32,
        time as u32 % 60
    ));
    ui.horizontal(|ui| {
        if ui.button(RichText::new("Take it").strong()).clicked() {
            visit.take_job();
            visit
                .news
                .say("Off you go! The house is marked on the map (K).");
        }
        if ui.button("Another order").clicked() {
            visit.jobs.offer = None;
        }
    });
}
