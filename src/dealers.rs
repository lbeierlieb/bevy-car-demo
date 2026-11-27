//! The windows of the shops that trade (see `trade`): a used parts shop's wares and what
//! it pays for what we bring, a used car dealer's cars, and the view of what a car is built
//! of and how worn each part is (also for the garage's cars). Their screens go on the shop's stack (`AtShop::stash`, as
//! `trunk::Screen::Deal`), so "back" goes back a screen as in the trunk's. Everything is
//! asked of the server (`ToServer::Trade`), which answers with what the shop has and what
//! we own.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car};
use crate::catalogue::{Catalogue, Current};
use crate::items::Item;
use crate::menu::kit::{self, Panel, Place};
use crate::net::Connection;
use crate::net::protocol::ToServer;
use crate::owned::{Source, name};
use crate::shops::{AtShop, ShopKind, Shops, shop_window};
use crate::trade::{self, Trade, euros};

/// How long the server's word on why something wasn't done stays up (s).
const REFUSAL: f64 = 8.0;

/// The screens over a shop's window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Screen {
    /// A used part on sale (by its number).
    Ware(u32),
    /// What we could sell: the trunk's and what the shop holds for us.
    Sell,
    /// One of them.
    SellOne(Source, usize),
    /// A used car on sale (by its number).
    Car(u32),
    /// What a car or part is built of, and how worn.
    Inspect(Look),
}

/// A car or part looked into.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// A used car dealer's car (by its number).
    CarWare(u32),
    /// A car in our garage (by its place).
    Garage(usize),
}

/// What's picked on the screens.
#[derive(Resource, Default)]
pub struct Deals {
    /// Whether the car driven is traded in for a used car.
    pub trade_in: bool,
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Screen::Ware(_) => "Used part",
            Screen::Sell => "Sell a part",
            Screen::SellOne(..) => "Sell it",
            Screen::Car(_) => "Used car",
            Screen::Inspect(_) => "What it's built of",
        }
    }
}

pub struct DealersPlugin;

impl Plugin for DealersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Deals>()
            .add_systems(EguiPrimaryContextPass, (used_parts, used_cars, screens));
    }
}

/// A part's name and how worn it is.
fn worn_name(catalogue: &Catalogue, item: &Item) -> String {
    format!(
        "{} ({})",
        name(catalogue, item),
        trade::condition(catalogue, item)
    )
}

/// What the server last said wasn't done, while it's fresh.
fn refusal(connection: &Connection, time: &Time<Real>) -> Option<String> {
    (connection.refused.as_ref())
        .filter(|(_, at)| time.elapsed_secs_f64() - at < REFUSAL)
        .map(|(why, _)| why.clone())
}

/// A part's rows: what it is, how worn, its mass and size.
fn part_rows(ui: &mut bevy_egui::egui::Ui, catalogue: &Catalogue, item: &Item) {
    let def = item.def(catalogue);
    kit::info(ui, "Part", &name(catalogue, item));
    kit::info(ui, "Condition", &trade::condition(catalogue, item));
    kit::info(
        ui,
        "Mass",
        &format!("{:.0} kg", crate::cargo::mass(catalogue, item)),
    );
    let [l, w, h] = def.size.map(|m| (m * 100.0).round());
    kit::info(ui, "Size", &format!("{l} × {w} × {h} cm"));
}

/// What `item` is built of, a row for each part in a slot (indented by how deep), with how
/// worn it is: each a row the arrows go through, for a long list to scroll.
pub fn inspect(ui: &mut bevy_egui::egui::Ui, catalogue: &Catalogue, item: &Item) {
    kit::info(ui, "Condition", &trade::condition(catalogue, item));
    rows(ui, catalogue, item, 0);
}

fn rows(ui: &mut bevy_egui::egui::Ui, catalogue: &Catalogue, item: &Item, depth: usize) {
    let slots = &item.def(catalogue).slots;
    for (slot, part) in slots.iter().zip(&item.slots) {
        let Some(part) = part.as_ref().filter(|part| !part.def(catalogue).absent) else {
            continue;
        };
        let label = format!("{}{}", "    ".repeat(depth), slot.name);
        let detail = match crate::garage::fit::wear_note(part) {
            Some(wear) => format!("{} ({wear})", name(catalogue, part)),
            None => name(catalogue, part),
        };
        kit::button_with(ui, &label, &detail);
        rows(ui, catalogue, part, depth + 1);
    }
}

/// The used car dealer's window: its cars.
fn used_cars(
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    connection: Option<ResMut<Connection>>,
    catalogue: Option<Res<Current>>,
    time: Res<Time<Real>>,
) -> Result {
    let (Some(mut connection), Some(catalogue)) = (connection, catalogue) else {
        return Ok(());
    };
    let catalogue = &catalogue.0;
    let here = at.shop;
    let stock = (connection.stock.as_ref())
        .filter(|stock| Some(usize::from(stock.shop)) == here)
        .cloned();
    let refused = refusal(&connection, &time);
    let (mut look, mut go) = (false, None);
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::UsedCars, |ui, came_in| {
        look = came_in;
        if let Some(why) = &refused {
            kit::warning(ui, why);
        }
        kit::section(ui, "Used cars");
        match &stock {
            None => kit::note(ui, "Looking what's there…"),
            Some(stock) if stock.cars.is_empty() => kit::note(ui, "No cars just now."),
            Some(stock) => {
                for ware in &stock.cars {
                    let label = format!(
                        "{}, {} km",
                        crate::trunk::car_name(catalogue, &ware.car),
                        thousands(ware.km)
                    );
                    if kit::submenu_with(ui, &label, &euros(ware.price)).clicked() {
                        go = Some(Screen::Car(ware.id));
                    }
                }
            }
        }
        kit::note(
            ui,
            "Other cars come in every half hour. The car you drive goes into your garage, or \
             the dealer takes it in part exchange.",
        );
    });
    if look {
        connection.refused = None;
        connection.say(&ToServer::Trade(Trade::Look));
    }
    if let Some(screen) = go {
        at.stash.push(crate::trunk::Screen::Deal(screen));
    }
    Ok(())
}

/// A whole number with its thousands apart: "123 000".
fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

/// The used parts shop's window: its wares, and selling to it.
fn used_parts(
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    connection: Option<ResMut<Connection>>,
    catalogue: Option<Res<Current>>,
    time: Res<Time<Real>>,
) -> Result {
    let (Some(mut connection), Some(catalogue)) = (connection, catalogue) else {
        return Ok(());
    };
    let catalogue = &catalogue.0;
    let here = at.shop;
    let stock = (connection.stock.as_ref())
        .filter(|stock| Some(usize::from(stock.shop)) == here)
        .cloned();
    let refused = refusal(&connection, &time);
    let (mut look, mut go) = (false, None);
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Used, |ui, came_in| {
        look = came_in;
        if let Some(why) = &refused {
            kit::warning(ui, why);
        }
        kit::section(ui, "For sale, used");
        match &stock {
            None => kit::note(ui, "Looking what's there…"),
            Some(stock) if stock.parts.is_empty() => kit::note(ui, "Nothing just now."),
            Some(stock) => {
                for ware in &stock.parts {
                    let label = worn_name(catalogue, &ware.item);
                    if kit::submenu_with(ui, &label, &euros(ware.price)).clicked() {
                        go = Some(Screen::Ware(ware.id));
                    }
                }
            }
        }
        if kit::submenu(ui, Screen::Sell.title()).clicked() {
            go = Some(Screen::Sell);
        }
        kit::note(
            ui,
            "New things come in every half hour. The shop buys anything you bring, in the \
             trunk or left here, for a little.",
        );
    });
    if look {
        connection.refused = None;
        connection.say(&ToServer::Trade(Trade::Look));
    }
    if let Some(screen) = go {
        at.stash.push(crate::trunk::Screen::Deal(screen));
    }
    Ok(())
}

/// The screen on top of a shop's stack, if it's a dealer's.
fn screens(
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    connection: Option<ResMut<Connection>>,
    catalogue: Option<Res<Current>>,
    (active, cars): (Res<ActiveCar>, Query<&Car>),
    time: Res<Time<Real>>,
    mut deals: ResMut<Deals>,
) -> Result {
    let Some(&crate::trunk::Screen::Deal(screen)) = at.stash.last() else {
        return Ok(());
    };
    let (Some(mut connection), Some(catalogue)) = (connection, catalogue) else {
        at.stash.clear();
        return Ok(());
    };
    let Some((here, shop)) = at.shop.and_then(|i| Some((i, &shops.list.get(i)?.shop))) else {
        at.stash.clear();
        return Ok(());
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let catalogue = &catalogue.0;
    let trunk = &car.setup.cargo;
    let held = connection.owned.held_at(here).to_vec();
    let garage = connection.owned.cars.clone();
    let money = connection.owned.money;
    let stock = (connection.stock.as_ref())
        .filter(|stock| usize::from(stock.shop) == here)
        .cloned()
        .unwrap_or_default();
    let refused = refusal(&connection, &time);
    let (mut ask, mut go, mut up) = (None, None, false);
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
            kit::info(ui, "In your wallet", &euros(money));
            match screen {
                Screen::Ware(id) => {
                    let Some(ware) = stock.parts.iter().find(|ware| ware.id == id) else {
                        kit::note(ui, "It's gone.");
                        up = kit::button(ui, "Back").clicked();
                        return;
                    };
                    part_rows(ui, catalogue, &ware.item);
                    kit::info(ui, "Price", &euros(ware.price));
                    let afford = money >= ware.price;
                    let label = format!("Buy it for {}", euros(ware.price));
                    let buy = ui
                        .add_enabled_ui(afford, |ui| kit::button(ui, &label))
                        .inner;
                    if buy.clicked() {
                        ask = Some(Trade::BuyPart { ware: id });
                    }
                    if !afford {
                        kit::note(ui, "You haven't got enough money for it.");
                    }
                    kit::note(
                        ui,
                        "It goes into your trunk if there's room, else the shop holds it for \
                         you.",
                    );
                }
                Screen::Sell => {
                    let lists = [
                        (Source::Trunk, "In the trunk", trunk.as_slice()),
                        (Source::Here, "Held for you here", held.as_slice()),
                    ];
                    for (from, title, items) in lists {
                        kit::section(ui, title);
                        if items.is_empty() {
                            kit::note(ui, "Nothing.");
                        }
                        for (i, item) in items.iter().enumerate() {
                            let offer = euros(trade::buying_price(catalogue, item));
                            if kit::submenu_with(ui, &worn_name(catalogue, item), &offer).clicked()
                            {
                                go = Some(Screen::SellOne(from, i));
                            }
                        }
                    }
                }
                Screen::SellOne(from, i) => {
                    let list = match from {
                        Source::Trunk => trunk.as_slice(),
                        Source::Here => held.as_slice(),
                    };
                    let Some(item) = list.get(i) else {
                        up = true;
                        return;
                    };
                    part_rows(ui, catalogue, item);
                    let price = trade::buying_price(catalogue, item);
                    if kit::button(ui, &format!("Sell it for {}", euros(price))).clicked() {
                        ask = Some(Trade::Sell {
                            from,
                            item: i as u16,
                        });
                    }
                    kit::note(ui, "It's gone once it's sold.");
                }
                Screen::Car(id) => {
                    let Some(ware) = stock.cars.iter().find(|ware| ware.id == id) else {
                        kit::note(ui, "It's gone.");
                        up = kit::button(ui, "Back").clicked();
                        return;
                    };
                    kit::info(ui, "Car", &crate::trunk::car_name(catalogue, &ware.car));
                    let paint = ware.car.paint.and_then(|p| catalogue.paints.get(p));
                    if let Some(paint) = paint {
                        kit::info(ui, "Paint", &paint.name);
                    }
                    kit::info(ui, "Mileage", &format!("{} km", thousands(ware.km)));
                    kit::info(ui, "Condition", &trade::condition(catalogue, &ware.car));
                    kit::info(ui, "Price", &euros(ware.price));
                    if kit::submenu(ui, Screen::Inspect(Look::CarWare(id)).title()).clicked() {
                        go = Some(Screen::Inspect(Look::CarWare(id)));
                    }
                    // What the dealer gives for the car driven.
                    let mut tree = car.setup.car.clone();
                    crate::saved::Condition::of(car).put_on(catalogue, &mut tree);
                    let credit = trade::trade_in_price(catalogue, &tree, trunk);
                    let label = format!("Trade in the car I drive ({})", euros(credit));
                    kit::toggle(ui, &label, &mut deals.trade_in);
                    let price = ware.price - if deals.trade_in { credit } else { 0 };
                    let afford = money >= price;
                    let label = if price >= 0 {
                        format!("Buy it for {}", euros(price))
                    } else {
                        format!("Buy it and get {}", euros(-price))
                    };
                    let buy = ui
                        .add_enabled_ui(afford, |ui| kit::button(ui, &label))
                        .inner;
                    if buy.clicked() {
                        ask = Some(Trade::BuyCar {
                            ware: id,
                            trade_in: deals.trade_in,
                        });
                    }
                    if !afford {
                        kit::note(ui, "You haven't got enough money for it.");
                    }
                    kit::note(
                        ui,
                        if deals.trade_in {
                            "The car you drive is the dealer's then, with what's in its trunk."
                        } else {
                            "The car you drive goes into your garage, with what's in its trunk."
                        },
                    );
                }
                Screen::Inspect(look) => {
                    let item = match look {
                        Look::CarWare(id) => stock.cars.iter().find(|w| w.id == id).map(|w| &w.car),
                        Look::Garage(i) => garage.get(i).map(|parked| &parked.car),
                    };
                    match item {
                        Some(item) => inspect(ui, catalogue, item),
                        None => kit::note(ui, "It's gone."),
                    }
                }
            }
        });
    if let Some(trade) = ask {
        connection.refused = None;
        connection.say(&ToServer::Trade(trade));
        // Back to the shop's window: the places in the lists change.
        at.stash.clear();
    } else if let Some(screen) = go {
        at.stash.push(crate::trunk::Screen::Deal(screen));
    } else if up {
        at.stash.pop();
    }
    Ok(())
}
