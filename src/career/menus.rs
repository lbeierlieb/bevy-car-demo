//! The window of the place the car is stopped at: home (into the garage), the gas station
//! (fill up), the pizzeria and the car dealer (jobs, cars), the parts shop.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::hud::News;
use super::jobs::{JobKind, Jobs, TIERS, start_delivery, take_offer};
use super::places::{AtPlace, Place, PlaceKind, Places};
use super::{Career, PLAYER, Scene, euros};
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
    pub active: ResMut<'w, ActiveCar>,
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

    /// Starts the job on offer at `index`.
    fn take_job(&mut self, index: usize) {
        take_offer(
            &mut self.jobs,
            index,
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
        jobs.offers.clear();
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
            PlaceKind::Dealer => dealer(ui, &mut visit, &places),
            _ => {
                ui.label("Closed for now.");
            }
        });
    Ok(())
}

fn home(ui: &mut egui::Ui, visit: &mut Visit) {
    ui.label("Home: the garage, with mom's car and yours.");
    if visit.active.0 != PLAYER {
        ui.label("Deliver the dealer's car first, or call the tow truck (T).");
        return;
    }
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
    if visit.jobs.offers.is_empty() {
        let offer = visit.jobs.pizza(&visit.terrain, place.at);
        visit.jobs.offers.extend(offer);
    }
    let Some(offer) = visit.jobs.offers.first() else {
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
            visit.take_job(0);
            visit
                .news
                .say("Off you go! The house is marked on the map (K).");
        }
        if ui.button("Another order").clicked() {
            visit.jobs.offers.clear();
        }
    });
}

/// The car dealer: cars to deliver.
fn dealer(ui: &mut egui::Ui, visit: &mut Visit, places: &Places) {
    if visit.jobs.current.is_some() {
        ui.label("Finish the job you're on first.");
        return;
    }
    ui.strong("Cars to deliver");
    let trust = visit.career.trust;
    let next = TIERS.iter().find(|tier| tier.0 > trust);
    ui.label(match next {
        Some((needed, ..)) => format!(
            "Deliver them without a scratch: the dealer trusts you with nicer cars from \
             {needed:.0} trust."
        ),
        None => "The dealer trusts you with anything on the lot.".to_string(),
    });
    if visit.jobs.offers.is_empty() {
        let at = places.of(PlaceKind::Dealer).at;
        let offers = visit.jobs.deliveries(&visit.terrain, at, trust);
        visit.jobs.offers = offers;
    }
    let mut take = None;
    egui::Grid::new("deliveries").striped(true).show(ui, |ui| {
        for (i, offer) in visit.jobs.offers.iter().enumerate() {
            let JobKind::Delivery {
                chassis, option, ..
            } = offer.kind
            else {
                continue;
            };
            let model = &crate::chassis::CHASSIS[chassis];
            ui.label(format!("{} {}", model.name, model.engines[option]));
            ui.label(format!("{:.1} km", offer.distance / 1000.0));
            ui.label(euros(offer.pay));
            if ui.button("Deliver it").clicked() {
                take = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = take {
        let automatic = visit.car().is_none_or(|car| car.drivetrain.automatic);
        start_delivery(
            &mut visit.commands,
            &visit.terrain,
            places,
            &visit.jobs.offers[i],
            automatic,
            &mut visit.active,
        );
        visit.take_job(i);
        visit
            .news
            .say("Here are the keys. It's waiting in front; the address is on the map (K).");
    }
}
