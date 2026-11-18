//! The window of the place the car is stopped at: home (into the garage), the gas station
//! (fill up), the pizzeria and the car dealer (jobs, cars), the parts shop, the dyno.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::dyno::{DynoBooked, RUN_PRICE, cannot_run};
use super::hud::News;
use super::jobs::{JobKind, Jobs, TIERS, start_delivery, take_offer};
use super::lot::{self, Lot};
use super::places::{AtPlace, Place, PlaceKind, Places};
use super::{Career, Item, PLAYER, Ride, Scene, euros};
use crate::car::{ActiveCar, Car};
use crate::physics::fuel::Fuel;
use crate::terrain::Terrain;
use crate::wheels::Wheel;

const WARNING: egui::Color32 = egui::Color32::from_rgb(255, 170, 60);

/// What fuel costs (€ per litre).
pub const FUEL_PRICE: f64 = 1.85;

pub struct MenusPlugin;

impl Plugin for MenusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lot>()
            .add_systems(Update, forget_offers)
            .add_systems(
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
    pub cars: Query<'w, 's, (Entity, &'static mut Car)>,
    pub lot: ResMut<'w, Lot>,
    pub scene: ResMut<'w, NextState<Scene>>,
    pub jobs: ResMut<'w, Jobs>,
    pub terrain: Res<'w, Terrain>,
    pub commands: Commands<'w, 's>,
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub booked: ResMut<'w, DynoBooked>,
    /// The rims and tires looked at in the parts shop.
    pub shop_wheel: Local<'s, Option<Wheel>>,
}

impl Visit<'_, '_> {
    /// The car being driven.
    pub fn car(&mut self) -> Option<Mut<'_, Car>> {
        let active = self.active.0;
        (self.cars.iter_mut())
            .find(|(_, car)| car.index == active)
            .map(|(_, car)| car)
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
            PlaceKind::PartsShop => parts_shop(ui, &mut visit),
            PlaceKind::Dyno => dyno(ui, &mut visit),
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

/// The car dealer: cars to deliver, cars for sale, and buying the player's.
fn dealer(ui: &mut egui::Ui, visit: &mut Visit, places: &Places) {
    if visit.jobs.current.is_some() {
        ui.label("Finish the job you're on first.");
        return;
    }
    ui.horizontal(|ui| {
        for (page, name) in [(false, "Cars to deliver"), (true, "Cars for sale")] {
            let shown = ui.memory(|m| m.data.get_temp::<bool>(egui::Id::new("lot")));
            if ui
                .selectable_label(shown.unwrap_or(false) == page, name)
                .clicked()
            {
                ui.memory_mut(|m| m.data.insert_temp(egui::Id::new("lot"), page));
            }
        }
    });
    ui.separator();
    if ui.memory(|m| m.data.get_temp::<bool>(egui::Id::new("lot"))) == Some(true) {
        for_sale(ui, visit);
        return;
    }
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
        start_delivery(
            &mut visit.commands,
            &visit.terrain,
            places,
            &visit.jobs.offers[i],
            &mut visit.active,
        );
        visit.take_job(i);
        visit
            .news
            .say("Here are the keys. It's waiting in front; the address is on the map (K).");
    }
}

/// The lot: cars to buy, and what the dealer gives for the player's.
fn for_sale(ui: &mut egui::Ui, visit: &mut Visit) {
    let Visit {
        career, lot, jobs, ..
    } = visit;
    lot.restock(career, jobs.rng());
    if !career.has_room() {
        ui.colored_label(WARNING, "The garage at home is full: sell a car first.");
    }
    let mut bought = None;
    egui::Grid::new("lot").striped(true).show(ui, |ui| {
        for (i, sale) in lot.cars.iter().enumerate() {
            let setup = &sale.car.setup;
            ui.label(format!(
                "{} {}",
                setup.name, setup.engine.build.hardware.name
            ))
            .on_hover_text(setup.build.chassis().about);
            ui.label(if sale.used() {
                let cylinders = setup.engine.build.hardware.cylinders as usize;
                let worn = sale.car.wear.damage.worst(cylinders) * 100.0;
                format!("used, engine {worn:.0} % worn")
            } else {
                "new".to_string()
            });
            ui.label(euros(sale.price));
            let can = career.money >= sale.price && career.has_room();
            if ui.add_enabled(can, egui::Button::new("Buy")).clicked() {
                bought = Some(i);
            }
            ui.end_row();
        }
    });
    let ride = career.ride;
    let mut sold = false;
    if let super::Ride::Own(i) = ride
        && let Some(car) = career.cars.get(i)
    {
        ui.separator();
        let price = lot::trade_in(car);
        let text = format!("Sell your {} for {}", car.setup.name, euros(price));
        sold = ui
            .button(text)
            .on_hover_text("Mom will lend you her car to get home")
            .clicked();
    }
    let active = visit.active.0;
    let Some((entity, car)) = visit.cars.iter().find(|(_, car)| car.index == active) else {
        return;
    };
    if let Some(i) = bought {
        let came_in = visit.career.ride_car().0.name;
        if let Some(name) = lot::buy(
            &mut visit.lot,
            i,
            &mut visit.career,
            (entity, car),
            &mut visit.commands,
            &visit.terrain,
            &mut visit.active,
        ) {
            visit.news.say(format!(
                "The {name} is yours! The {came_in} you came in is back in the garage at home."
            ));
        }
    } else if sold
        && let Some(price) = lot::sell(
            &mut visit.career,
            (entity, car),
            &mut visit.commands,
            &visit.terrain,
            &mut visit.active,
        )
    {
        visit.news.say(format!(
            "Sold for {}. Mom dropped off her car for you.",
            euros(price)
        ));
    }
}

/// Parts to order: they wait on the shelf in the garage at home.
fn parts_shop(ui: &mut egui::Ui, visit: &mut Visit) {
    ui.label("Whatever you order waits on the shelf in your garage at home.");
    let mut order = None;
    egui::ScrollArea::vertical()
        .max_height(330.0)
        .show(ui, |ui| {
            ui.collapsing("Rims and tires", |ui| {
                order = order.or(rims_and_tires(ui, visit));
            });
            let mut shelves = super::catalog::stock();
            // Anti-roll bars are made for one car: those for the player's.
            let mut chassis: Vec<usize> = (visit.career.cars.iter())
                .map(|car| car.setup.build.chassis)
                .collect();
            chassis.sort_unstable();
            chassis.dedup();
            for chassis in chassis {
                let name = crate::chassis::CHASSIS[chassis].name;
                let shelf = (
                    format!("Anti-roll bars: {name}"),
                    super::catalog::roll_bars(chassis),
                );
                shelves.insert(2, shelf);
            }
            for (kind, items) in shelves {
                ui.collapsing(kind, |ui| {
                    for item in items {
                        let Some(price) = super::catalog::price(item) else {
                            continue;
                        };
                        ui.horizontal(|ui| {
                            let afford = visit.career.money >= price;
                            let button = egui::Button::new(euros(price));
                            if ui.add_enabled(afford, button).clicked() {
                                order = Some((item, price));
                            }
                            ui.label(super::fitting::name(item));
                        });
                    }
                });
            }
        });
    if let Some((item, price)) = order
        && visit.career.pay(price)
    {
        visit.career.items.push(item);
        visit.career.save();
        let name = super::fitting::name(item);
        visit.news.say(format!(
            "Ordered: {name} ({}). It's in your garage.",
            euros(price)
        ));
    }
}

/// Rims and tires of any size on offer, by the pair: which of the player's cars they fit
/// together, and the one ordered, if one is.
fn rims_and_tires(ui: &mut egui::Ui, visit: &mut Visit) -> Option<(Item, f64)> {
    let career = &visit.career;
    let ride = match career.ride {
        Ride::Own(i) => &career.cars[i].setup,
        Ride::Mom => &career.mom,
    };
    let wheel = visit.shop_wheel.get_or_insert(ride.build.wheels[0]);
    crate::garage::wheel_picker(ui, 0, wheel);
    let wheel = *wheel;
    // Where the rims with these tires on them would go.
    let fits: Vec<String> = (career.cars.iter())
        .flat_map(|car| {
            let chassis = car.setup.build.chassis();
            (0..2)
                .filter(move |&axle| chassis.room[axle].misfit(&wheel).is_none())
                .map(move |axle| format!("{} {}", chassis.name, ["front", "rear"][axle]))
        })
        .collect();
    ui.label(
        RichText::new(if career.cars.is_empty() {
            "You have no car of your own to fit them to yet.".to_string()
        } else if fits.is_empty() {
            "Fits none of your cars.".to_string()
        } else {
            format!("Fits: {}", fits.join(", "))
        })
        .weak(),
    );
    if !wheel.tire.fits(&wheel.rim) {
        ui.colored_label(WARNING, "These tires don't suit these rims.");
    }
    let mut order = None;
    for item in [
        Item::Rims(wheel.rim),
        Item::Tires(wheel.tire, wheel.compound),
    ] {
        let price = super::catalog::price(item)?;
        ui.horizontal(|ui| {
            let afford = career.money >= price;
            if ui
                .add_enabled(afford, egui::Button::new(euros(price)))
                .clicked()
            {
                order = Some((item, price));
            }
            ui.label(format!("{} (a pair)", super::fitting::name(item)));
        });
    }
    order
}

/// The dyno: a paid run, or a look at the runs on file.
fn dyno(ui: &mut egui::Ui, visit: &mut Visit) {
    ui.label(format!(
        "An all-wheel chassis dyno: {} a run. Through the gears, then a pull in fourth to \
         the rev limiter, measuring the power at the wheels.",
        euros(RUN_PRICE)
    ));
    if visit.active.0 != PLAYER {
        ui.label("Not with the dealer's car: deliver it first.");
        return;
    }
    if visit.jobs.current.is_some() {
        ui.label("Finish the job you're on first.");
        return;
    }
    let money = visit.career.money;
    let why_not = visit.car().and_then(|car| cannot_run(&car, money));
    if let Some(why) = why_not {
        ui.colored_label(WARNING, why);
    }
    let runs = visit.career.runs.len();
    let mut go = None;
    ui.horizontal(|ui| {
        let text = format!("Onto the rollers: {}", euros(RUN_PRICE));
        let button = egui::Button::new(RichText::new(text).strong());
        if ui.add_enabled(why_not.is_none(), button).clicked() {
            go = Some(true);
        }
        if runs > 0 && ui.button(format!("The runs on file ({runs})")).clicked() {
            go = Some(false);
        }
    });
    let Some(run) = go else {
        return;
    };
    if run && !visit.career.pay(RUN_PRICE) {
        return;
    }
    visit.booked.0 = run;
    visit.scene.set(Scene::Dyno);
}
