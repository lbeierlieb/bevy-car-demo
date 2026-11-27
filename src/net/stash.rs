//! The server's side of what players own (see `owned`): what they own read from what was
//! saved and back, and what they ask done with it.

use bevy::prelude::*;

use super::accounts::{Belongings, GARAGE, held_at};
use super::protocol::ToClient;
use super::server::Peer;
use super::shadow::BakedMaps;
use crate::car::CarSetup;
use crate::cargo;
use crate::catalogue::{Catalogue, Shared};
use crate::fitting::{Bench, Work, engine_shop_takes};
use crate::items::Item;
use crate::items::{Does, trim_of};
use crate::owned::{GARAGE_CARS, GARAGE_PARTS, Owned, Parked, SHOP_HOLD, Source, Stash, name};
use crate::saved::{self, Condition};
use crate::shops::{Shop, ShopKind};
use crate::spec::CarSpec;

/// What `belongings` (as saved) come to: the car driven, if it reads, and the rest.
pub fn read(
    catalogue: &Shared,
    belongings: &Belongings,
    who: &str,
) -> (Option<(CarSpec, Condition)>, Owned) {
    let car = (belongings.car.as_ref()).and_then(|text| match saved::read(catalogue, text) {
        Ok(car) => Some(car),
        Err(e) => {
            warn!("{who}'s saved car: {e}; they get the start car");
            None
        }
    });
    let mut owned = Owned::default();
    for text in &belongings.garage {
        match saved::read(catalogue, text) {
            Ok((spec, condition)) => owned.cars.push(Parked { spec, condition }),
            Err(e) => warn!("A car in {who}'s garage: {e}; left out"),
        }
    }
    for (place, text) in &belongings.items {
        let items = saved::read_items(catalogue, text);
        if place == GARAGE {
            owned.parts.extend(items);
        } else if let Some(shop) = place.strip_prefix(&held_at("")) {
            owned.held_at_mut(shop).extend(items);
        } else {
            warn!("{who}'s parts at \"{place}\"? Left out");
        }
    }
    (car, owned)
}

/// What `peer` owns as it's saved: their car driven (if the server has it) and the rest.
pub fn belongings(peer: &Peer, catalogue: &Catalogue) -> Belongings {
    let one = |item: &Item| saved::write_items(catalogue, std::slice::from_ref(item));
    let owned = &peer.owned;
    let held = (owned.held.iter())
        .flat_map(|(shop, items)| items.iter().map(move |item| (held_at(shop), item)));
    Belongings {
        car: (peer.shadow.kept())
            .map(|(spec, car)| saved::write(catalogue, spec, &Condition::of(car))),
        garage: (owned.cars.iter())
            .map(|parked| saved::write(catalogue, &parked.spec, &parked.condition))
            .collect(),
        items: (owned
            .parts
            .iter()
            .map(|item| (GARAGE.to_string(), one(item))))
        .chain(held.map(|(place, item)| (place, one(item))))
        .collect(),
    }
}

/// Tells `peer` what they own (and why what they asked wasn't done, if it wasn't).
pub fn tell(peer: &mut Peer, shops: &[Shop], why: Option<String>) {
    let owned = Box::new(peer.owned.view(shops));
    peer.tell(&ToClient::Owned { owned, why });
}

/// `peer`'s car is built as `spec` from now on: the server's copy is built anew, and the
/// player told.
fn drive(peer: &mut Peer, spec: CarSpec, baked: &mut BakedMaps) {
    peer.shadow.set_spec(spec, baked);
    if let Some(spec) = peer.shadow.driven().cloned() {
        peer.tell(&ToClient::YourCar(Box::new(spec)));
    }
}

/// Does what `peer` asked (as far as the rules allow), and tells them what they own now.
pub fn act(
    peer: &mut Peer,
    stash: Stash,
    shops: &[Shop],
    catalogue: &Catalogue,
    baked: &mut BakedMaps,
) {
    let why = try_act(peer, stash, shops, catalogue, baked).err();
    if let Some(why) = &why {
        info!("Player {}: {stash:?} not done: {why}", peer.id);
    }
    tell(peer, shops, why);
}

fn try_act(
    peer: &mut Peer,
    stash: Stash,
    shops: &[Shop],
    catalogue: &Catalogue,
    baked: &mut BakedMaps,
) -> Result<(), String> {
    let here = peer.at_shop().and_then(|i| Some((i, shops.get(i)?)));
    let Some(mut spec) = peer.shadow.spec().cloned() else {
        return Err("You have no car yet".into());
    };
    if peer.shadow.lent() {
        return Err("Not while you drive a lent car".into());
    }
    let none = || "That's gone already".to_string();
    let garage =
        |here: Option<(usize, &Shop)>| here.is_some_and(|(_, shop)| shop.kind == ShopKind::Garage);
    let owned = &mut peer.owned;
    match stash {
        Stash::Unload { item } => {
            let (_, shop) = here.ok_or("Stop at a shop or the garage park to unload")?;
            let part = spec.cargo.get(usize::from(item)).ok_or_else(none)?;
            if shop.kind == ShopKind::Garage {
                if owned.parts.len() >= GARAGE_PARTS {
                    return Err("Your garage is full".into());
                }
                owned.parts.push(part.clone());
            } else {
                let held = owned.held_at_mut(&shop.name);
                if held.len() >= SHOP_HOLD {
                    return Err(format!(
                        "{} holds {SHOP_HOLD} parts for you at most",
                        shop.name
                    ));
                }
                if shop.kind == ShopKind::Engine && !engine_shop_takes(catalogue, held, part) {
                    return Err(format!("{} keeps one of those for you at most", shop.name));
                }
                held.push(part.clone());
            }
            spec.cargo.remove(usize::from(item));
        }
        Stash::Load { item } => {
            let (_, shop) = here.ok_or("Stop at a shop or the garage park to load")?;
            let from = if shop.kind == ShopKind::Garage {
                &mut owned.parts
            } else {
                owned.held_at_mut(&shop.name)
            };
            let part = from.get(usize::from(item)).ok_or_else(none)?;
            if !cargo::fits(catalogue, &spec.car, &spec.cargo, part) {
                return Err(format!(
                    "No room in the trunk for the {}",
                    name(catalogue, part)
                ));
            }
            spec.cargo.push(from.remove(usize::from(item)));
        }
        Stash::ShipHome { from, item } => {
            let (_, shop) = here.ok_or("Stop at a shop to ship parts to your garage")?;
            if owned.parts.len() >= GARAGE_PARTS {
                return Err("Your garage is full".into());
            }
            let part = match from {
                Source::Trunk => {
                    (spec.cargo.get(usize::from(item))).ok_or_else(none)?;
                    spec.cargo.remove(usize::from(item))
                }
                Source::Here if shop.kind == ShopKind::Garage => {
                    return Err("It's in your garage".into());
                }
                Source::Here => {
                    let held = owned.held_at_mut(&shop.name);
                    held.get(usize::from(item)).ok_or_else(none)?;
                    held.remove(usize::from(item))
                }
            };
            owned.parts.push(part);
        }
        Stash::Ship { item, shop: to } => {
            if !garage(here) {
                return Err("Parts are shipped to shops from the garage park".into());
            }
            let to = shops
                .get(usize::from(to))
                .filter(|shop| shop.kind != ShopKind::Garage);
            let to = to.ok_or("Not a shop")?;
            owned.parts.get(usize::from(item)).ok_or_else(none)?;
            if owned.held_at(&to.name).len() >= SHOP_HOLD {
                return Err(format!(
                    "{} holds {SHOP_HOLD} parts for you at most",
                    to.name
                ));
            }
            let part = &owned.parts[usize::from(item)];
            if to.kind == ShopKind::Engine
                && !engine_shop_takes(catalogue, owned.held_at(&to.name), part)
            {
                return Err(format!("{} keeps one of those for you at most", to.name));
            }
            let part = owned.parts.remove(usize::from(item));
            owned.held_at_mut(&to.name).push(part);
        }
        Stash::Drive { car } => {
            if !garage(here) {
                return Err("Cars are switched at the garage park".into());
            }
            owned.cars.get(usize::from(car)).ok_or_else(none)?;
            let Some((_, now)) = peer.shadow.kept() else {
                return Err("Your car isn't there yet".into());
            };
            let condition = Condition::of(now);
            // The wear on its parts, for what the player is told.
            condition.put_on(catalogue, &mut spec.car);
            let picked = owned.cars.remove(usize::from(car));
            owned
                .cars
                .insert(usize::from(car), Parked { spec, condition });
            owned.touch();
            peer.shadow.set_condition(picked.condition);
            drive(peer, picked.spec, baked);
            return Ok(());
        }
    }
    owned.touch();
    drive(peer, spec, baked);
    Ok(())
}

/// Does the works `peer` asked for (request `id`) at the shop their car is at, all or
/// none, and tells them the car as it is now, what they own (and why it wasn't done, if it
/// wasn't), and that the request is answered.
pub fn work(
    peer: &mut Peer,
    id: u32,
    works: &[Work],
    shops: &[Shop],
    catalogue: &Shared,
    baked: &mut BakedMaps,
) {
    let why = match try_work(peer, works, shops, catalogue, baked) {
        Ok(said) => said,
        Err(why) => {
            info!("Player {}: works not done: {why}", peer.id);
            Some(why)
        }
    };
    if let Some(spec) = peer.shadow.driven().cloned() {
        peer.tell(&ToClient::YourCar(Box::new(spec)));
    }
    tell(peer, shops, why);
    peer.tell(&ToClient::Done { id });
}

/// The works done (what the player should hear about it, if anything: parts thrown away),
/// or why not.
fn try_work(
    peer: &mut Peer,
    works: &[Work],
    shops: &[Shop],
    catalogue: &Shared,
    baked: &mut BakedMaps,
) -> Result<Option<String>, String> {
    let shop = (peer.at_shop())
        .and_then(|i| shops.get(i))
        .ok_or("Stop at a shop for that")?;
    let mut spec = peer.shadow.spec().cloned().ok_or("You have no car yet")?;
    if peer.shadow.lent() {
        return Err("Not while you drive a lent car".into());
    }
    // The parts as worn as the car is now (unless the car driven is still the one before
    // a change whose maps are baking: then the parts are as they went in).
    let condition = (peer.shadow.kept())
        .filter(|_| !peer.shadow.pending())
        .map(|(_, car)| Condition::of(car));
    if let Some(condition) = &condition {
        condition.put_on(catalogue, &mut spec.car);
    }
    let mut bench = Bench::new(catalogue, shop, &spec, peer.owned.held_at(&shop.name));
    for work in works {
        bench.work(work)?;
    }
    let done = bench.finish()?;
    if done.spec == spec && done.held == peer.owned.held_at(&shop.name) {
        return Ok(None);
    }
    // The car is as worn as the parts in it now: a new part is new, one brought as worn
    // as it was (the server says so, not the game).
    let condition = condition.map(|condition| {
        Condition::of_parts(catalogue, &done.spec.car, condition.fuel, condition.octane)
    });
    if done.held != peer.owned.held_at(&shop.name) {
        *peer.owned.held_at_mut(&shop.name) = done.held;
        peer.owned.touch();
    }
    peer.shadow.set_spec(done.spec, baked);
    if let Some(condition) = condition {
        peer.shadow.set_condition(condition);
    }
    let baking = peer.shadow.pending();
    let mut said = Vec::new();
    if !done.thrown.is_empty() {
        said.push(format!(
            "No room in the trunk or the shop for the old {}: thrown away",
            done.thrown.join(", ")
        ));
    }
    if baking {
        said.push(
            "The engine's maps are being worked out: it runs as it was until they're done"
                .to_string(),
        );
    }
    Ok((!said.is_empty()).then(|| said.join(". ")))
}

/// A player at a car dealer buys its trim `trim` (in the catalogue's `trims`): the car they
/// drove goes into their garage with what's in its trunk, or is traded in (gone, trunk and
/// all). The trim's name, or why not.
pub fn buy(
    peer: &mut Peer,
    trim: u16,
    trade_in: bool,
    shops: &[Shop],
    catalogue: &Shared,
    baked: &mut BakedMaps,
) -> Result<String, String> {
    let shop = (peer.at_shop())
        .and_then(|i| shops.get(i))
        .filter(|shop| shop.kind == ShopKind::Showroom);
    let made = catalogue
        .trims
        .get(usize::from(trim))
        .ok_or("No such car")?;
    match shop {
        Some(shop) if !shop.sells(&made.id) => {
            return Err(format!("{} doesn't sell the {}", shop.name, made.name));
        }
        None if !super::shadow::trusts_placements() => {
            return Err("Cars are bought at car dealers".into());
        }
        _ => {}
    }
    let Does::Chassis(chassis) = made.car.def(catalogue).does else {
        return Err("No such car".into());
    };
    let option = (0..catalogue.chassis[chassis].engines.len())
        .find(|&option| trim_of(catalogue, chassis, option).is_some_and(|t| t.id == made.id))
        .ok_or("No such car")?;
    let spec = CarSpec::of(&CarSetup::factory_in(catalogue, chassis, option));
    bought(peer, spec, catalogue, baked, trade_in)?;
    Ok(made.name.clone())
}

/// A player bought a car (`spec`): the one they drove goes into their garage, with what's
/// in its trunk (refused if the garage is full), unless it's traded in.
fn bought(
    peer: &mut Peer,
    spec: CarSpec,
    catalogue: &Catalogue,
    baked: &mut BakedMaps,
    trade_in: bool,
) -> Result<(), String> {
    if peer.shadow.lent() {
        return Err("Not while you drive a lent car".into());
    }
    if let Some((old, car)) = peer.shadow.kept().filter(|_| !trade_in) {
        if peer.owned.cars.len() >= GARAGE_CARS {
            return Err(format!("Your garage holds {GARAGE_CARS} cars at most"));
        }
        let mut parked = Parked {
            spec: old.clone(),
            condition: Condition::of(car),
        };
        parked.condition.put_on(catalogue, &mut parked.spec.car);
        peer.owned.cars.push(parked);
        peer.owned.touch();
    }
    drive(peer, spec, baked);
    Ok(())
}
