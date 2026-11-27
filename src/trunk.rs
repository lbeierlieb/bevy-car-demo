//! What a player owns, at shops: every shop's window has "Trunk and parts", screens of
//! what's in the car's trunk and what's here (the garage at the garage park, else what the
//! shop holds for them): load, unload, ship to the garage, and from the garage to a shop.
//! The garage park's window also lists the cars in the garage, to drive another (see
//! `garage::park` for the garage itself). All of it
//! is asked of the server (`owned::Stash`), which answers with what they own now (and the
//! car, when it changed).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car};
use crate::catalogue::{Catalogue, Current};
use crate::items::{Does, Item};
use crate::menu::kit::{self, Panel, Place};
use crate::net::Connection;
use crate::net::protocol::ToServer;
use crate::owned::{Source, Stash, name};
use crate::shops::{AtShop, ShopKind, Shops};

/// How long the server's word on why something wasn't done stays up (s).
const REFUSAL: f64 = 8.0;

/// The screens, in a stack (`AtShop::stash`; none open: the shop's own window).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Screen {
    /// The trunk and what's here.
    List,
    /// A part: in the trunk, or here.
    Item(Source, usize),
    /// Where to ship a part from the garage.
    ShipTo(usize),
    /// The cars in the garage, and one of them.
    Cars,
    Car(usize),
    /// A screen of a shop that trades (drawn by `dealers`).
    Deal(crate::dealers::Screen),
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Screen::List => "Trunk and parts",
            Screen::Item(..) => "Part",
            Screen::ShipTo(_) => "Ship it to",
            Screen::Cars => "Your cars",
            Screen::Car(_) => "Car",
            Screen::Deal(screen) => screen.title(),
        }
    }
}

pub struct TrunkPlugin;

impl Plugin for TrunkPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(EguiPrimaryContextPass, window);
    }
}

/// What a car is called: its chassis and engine.
pub fn car_name(catalogue: &Catalogue, car: &Item) -> String {
    let chassis = match car.def(catalogue).does {
        Does::Chassis(c) => catalogue.chassis[c].name.clone(),
        _ => car.def(catalogue).name.clone(),
    };
    let block = car
        .all()
        .into_iter()
        .find_map(|item| match item.def(catalogue).does {
            Does::Block { family, version } => {
                Some(&catalogue.families[family].versions[version].code)
            }
            _ => None,
        });
    match block {
        Some(code) => format!("{chassis} {code}"),
        None => chassis,
    }
}

/// A part's mass with what's fitted to it, and its box, for a row's right side.
fn detail(catalogue: &Catalogue, item: &Item) -> String {
    format!("{:.0} kg", crate::cargo::mass(catalogue, item))
}

/// The screens, while one is open at a shop.
#[allow(clippy::too_many_arguments)]
fn window(
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    connection: Option<ResMut<Connection>>,
    catalogue: Option<Res<Current>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    time: Res<Time<Real>>,
) -> Result {
    let Some(&screen) = at.stash.last().filter(|s| !matches!(s, Screen::Deal(_))) else {
        return Ok(());
    };
    let (Some(mut connection), Some(catalogue)) = (connection, catalogue) else {
        at.stash.clear();
        return Ok(());
    };
    let Some(shop) = at.shop.and_then(|i| Some((i, &shops.list.get(i)?.shop))) else {
        at.stash.clear();
        return Ok(());
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let catalogue = &catalogue.0;
    let (here, shop) = shop;
    let garage = shop.kind == ShopKind::Garage;
    let trunk = &car.setup.cargo;
    let owned = connection.owned.clone();
    let held: &[Item] = if garage {
        &owned.parts
    } else {
        owned.held_at(here)
    };
    let refused = (connection.refused.as_ref())
        .filter(|(_, at)| time.elapsed_secs_f64() - at < REFUSAL)
        .map(|(why, _)| why.clone());
    let mut ask = None;
    let mut go = None;
    let mut up = false;
    let title = format!("{} ({})", screen.title(), shop.name);
    let ctx = contexts.ctx_mut()?;
    Panel::new("shop", &title)
        .place(Place::Right)
        .width(440.0)
        .back(Some("Back"))
        .show(ctx, |ui| {
            if let Some(why) = &refused {
                kit::warning(ui, why);
            }
            match screen {
                Screen::List => {
                    let mass: f32 = trunk
                        .iter()
                        .map(|item| crate::cargo::mass(catalogue, item))
                        .sum();
                    kit::section(ui, &format!("In the trunk ({:.0} kg)", mass.abs()));
                    if trunk.is_empty() {
                        kit::note(ui, "Nothing.");
                    }
                    for (i, item) in trunk.iter().enumerate() {
                        if kit::submenu_with(ui, &name(catalogue, item), &detail(catalogue, item))
                            .clicked()
                        {
                            go = Some(Screen::Item(Source::Trunk, i));
                        }
                    }
                    let here_title = if garage {
                        "In the garage".to_string()
                    } else {
                        format!("Held for you here (at most {})", crate::owned::SHOP_HOLD)
                    };
                    kit::section(ui, &here_title);
                    if held.is_empty() {
                        kit::note(ui, "Nothing.");
                    }
                    for (i, item) in held.iter().enumerate() {
                        if kit::submenu_with(ui, &name(catalogue, item), &detail(catalogue, item))
                            .clicked()
                        {
                            go = Some(Screen::Item(Source::Here, i));
                        }
                    }
                    kit::note(
                        ui,
                        "Parts a shop takes off your car go into the trunk while there's room. \
                         Shipping is free, for now.",
                    );
                    if kit::button(ui, "Back").clicked() {
                        up = true;
                    }
                }
                Screen::Item(from, i) => {
                    let list = match from {
                        Source::Trunk => trunk.as_slice(),
                        Source::Here => held,
                    };
                    let Some(item) = list.get(i) else {
                        up = true;
                        return;
                    };
                    let def = item.def(catalogue);
                    kit::info(ui, "Part", &name(catalogue, item));
                    kit::info(ui, "Mass", &detail(catalogue, item));
                    let [l, w, h] = def.size.map(|m| (m * 100.0).round());
                    kit::info(ui, "Size", &format!("{l} × {w} × {h} cm"));
                    let item = i as u16;
                    match (from, garage) {
                        (Source::Trunk, true) => {
                            if kit::button(ui, "Put it in the garage").clicked() {
                                ask = Some(Stash::Unload { item });
                            }
                        }
                        (Source::Trunk, false) => {
                            if kit::button(ui, "Leave it here").clicked() {
                                ask = Some(Stash::Unload { item });
                            }
                            if kit::button(ui, "Ship it to the garage").clicked() {
                                ask = Some(Stash::ShipHome { from, item });
                            }
                        }
                        (Source::Here, _) => {
                            let fits =
                                crate::cargo::fits(catalogue, &car.setup.car, trunk, &list[i]);
                            let load = ui
                                .add_enabled_ui(fits, |ui| {
                                    kit::button(ui, "Load it into the trunk")
                                })
                                .inner;
                            if load.clicked() {
                                ask = Some(Stash::Load { item });
                            }
                            if !fits {
                                kit::note(ui, "There's no room for it in the trunk.");
                            }
                            if garage {
                                if kit::submenu(ui, "Ship it to a shop").clicked() {
                                    go = Some(Screen::ShipTo(i));
                                }
                            } else if kit::button(ui, "Ship it to the garage").clicked() {
                                ask = Some(Stash::ShipHome { from, item });
                            }
                        }
                    }
                }
                Screen::ShipTo(i) => {
                    let Some(item) = held.get(i).filter(|_| garage) else {
                        up = true;
                        return;
                    };
                    kit::note(
                        ui,
                        &format!("The {} waits for you there.", name(catalogue, item)),
                    );
                    for (s, placed) in shops.list.iter().enumerate() {
                        if placed.shop.kind == ShopKind::Garage {
                            continue;
                        }
                        let count = owned.held_at(s).len();
                        let label = format!("{} ({})", placed.shop.name, placed.shop.kind.title());
                        if kit::button_with(ui, &label, &format!("{count} held")).clicked() {
                            ask = Some(Stash::Ship {
                                item: i as u16,
                                shop: s as u16,
                            });
                        }
                    }
                }
                Screen::Cars => {
                    kit::info(ui, "You drive", &car_name(catalogue, &car.setup.car));
                    if owned.cars.is_empty() {
                        kit::note(
                            ui,
                            "No other cars: a car bought puts the one you drove in here.",
                        );
                    }
                    for (i, parked) in owned.cars.iter().enumerate() {
                        let carried = format!("{} in the trunk", parked.cargo.len());
                        if kit::submenu_with(ui, &car_name(catalogue, &parked.car), &carried)
                            .clicked()
                        {
                            go = Some(Screen::Car(i));
                        }
                    }
                }
                Screen::Car(i) => {
                    let Some(parked) = owned.cars.get(i) else {
                        up = true;
                        return;
                    };
                    kit::info(ui, "Car", &car_name(catalogue, &parked.car));
                    let look = crate::dealers::Screen::Inspect(crate::dealers::Look::Garage(i));
                    if kit::submenu(ui, look.title()).clicked() {
                        go = Some(Screen::Deal(look));
                    }
                    for item in &parked.cargo {
                        kit::info(ui, "In the trunk", &name(catalogue, item));
                    }
                    if kit::button(ui, "Drive it").clicked() {
                        ask = Some(Stash::Drive { car: i as u16 });
                    }
                    kit::note(
                        ui,
                        "The car you drive goes into the garage, with what's in its trunk.",
                    );
                }
                // (Drawn by `dealers`.)
                Screen::Deal(_) => {}
            }
        });
    if let Some(stash) = ask {
        connection.refused = None;
        connection.say(&ToServer::Stash(stash));
        // Back to the list: the places in it change.
        match stash {
            Stash::Drive { .. } => at.stash.clear(),
            _ => at.stash.truncate(1),
        }
    } else if let Some(screen) = go {
        at.stash.push(screen);
    } else if up {
        at.stash.pop();
    }
    Ok(())
}
