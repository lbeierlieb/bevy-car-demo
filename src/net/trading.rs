//! The server's side of trading (see `trade`): what the used parts shops and used car
//! dealers have (made up anew every `trade::STOCK_PERIOD`), buying from them and selling to
//! them. Money changes with what it pays for, in one go (`Peer::pay`).

use std::collections::HashMap;

use bevy::prelude::*;

use super::accounts::Accounts;
use super::protocol::ToClient;
use super::server::Peer;
use super::shadow::BakedMaps;
use super::stash::{bought, drive, spec_of_trim, tell};
use crate::cargo;
use crate::catalogue::{Catalogue, Shared};
use crate::owned::{SHOP_HOLD, Source, name};
use crate::saved::Condition;
use crate::shops::{Shop, ShopKind};
use crate::trade::{self, CarWare, Stock, Trade, Ware};

/// How full a used car's tank is (a share).
const USED_TANK: f32 = 0.3;

/// What the shops that trade have now, by their places in the server's list.
#[derive(Default)]
pub struct Stocks {
    /// A number of this server's own, so its wares differ from another's.
    seed: u64,
    shops: HashMap<usize, Wares>,
}

/// What a shop has in a period.
struct Wares {
    period: u64,
    parts: Vec<Ware>,
    cars: Vec<CarWare>,
}

impl Stocks {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            shops: HashMap::new(),
        }
    }

    /// What shop `at` (`shop`) has at `now` (s on the server's clock): made up when a new
    /// period has begun.
    fn of(&mut self, at: usize, shop: &Shop, catalogue: &Catalogue, now: f64) -> &mut Wares {
        let period = (now / trade::STOCK_PERIOD).max(0.0) as u64;
        let seed = trade::seed(&shop.name, period ^ self.seed);
        let wares = self.shops.entry(at).or_insert_with(|| Wares {
            period: u64::MAX,
            parts: Vec::new(),
            cars: Vec::new(),
        });
        if wares.period != period {
            // Numbered by the period, so a number never means another ware.
            let first = (period as u32).wrapping_mul(1000);
            *wares = Wares {
                period,
                parts: match shop.kind {
                    ShopKind::Used => trade::used_parts(catalogue, shop, seed, first),
                    _ => Vec::new(),
                },
                cars: match shop.kind {
                    ShopKind::UsedCars => trade::used_cars(catalogue, shop, seed, first),
                    _ => Vec::new(),
                },
            };
        }
        wares
    }
}

/// What the server needs to trade.
pub struct Desk<'a> {
    pub shops: &'a [Shop],
    pub stocks: &'a mut Stocks,
    pub accounts: Option<&'a Accounts>,
    pub catalogue: &'a Shared,
    pub baked: &'a mut BakedMaps,
    /// The server's clock (s).
    pub now: f64,
}

/// Does what `peer` asked of the shop they're at (as far as the rules and their wallet
/// allow), and tells them what it has and what they own now (and why it wasn't done, if it
/// wasn't).
pub fn trade(peer: &mut Peer, asked: Trade, desk: &mut Desk) {
    let why = try_trade(peer, asked, desk).err();
    if let Some(why) = &why {
        info!("Player {}: {asked:?} not done: {why}", peer.id);
    }
    tell_stock(peer, desk);
    tell(peer, desk.shops, why);
}

/// Tells `peer` what the shop they're at has, if it trades.
fn tell_stock(peer: &mut Peer, desk: &mut Desk) {
    let Some((at, shop)) = (peer.at_shop()).and_then(|i| Some((i, desk.shops.get(i)?))) else {
        return;
    };
    if !matches!(shop.kind, ShopKind::Used | ShopKind::UsedCars) {
        return;
    }
    let wares = desk.stocks.of(at, shop, desk.catalogue, desk.now);
    let stock = Stock {
        shop: at as u16,
        parts: wares.parts.clone(),
        cars: wares.cars.clone(),
    };
    peer.tell(&ToClient::Stock(Box::new(stock)));
}

fn try_trade(peer: &mut Peer, asked: Trade, desk: &mut Desk) -> Result<(), String> {
    let (at, shop) = (peer.at_shop())
        .and_then(|i| Some((i, desk.shops.get(i)?)))
        .ok_or("Stop at a shop for that")?;
    let catalogue: &Shared = desk.catalogue;
    let used = || {
        (shop.kind == ShopKind::Used)
            .then_some(())
            .ok_or_else(|| format!("{} doesn't trade in parts", shop.name))
    };
    match asked {
        Trade::Look => Ok(()),
        Trade::BuyPart { ware } => {
            used()?;
            let mut spec = peer.shadow.spec().cloned().ok_or("You have no car yet")?;
            if peer.shadow.lent() {
                return Err("Not while you drive a lent car".into());
            }
            let wares = desk.stocks.of(at, shop, catalogue, desk.now);
            let place = (wares.parts.iter())
                .position(|w| w.id == ware)
                .ok_or("Someone bought that just before you")?;
            let price = wares.parts[place].price;
            if peer.money < price {
                return Err(format!(
                    "Not enough money: it's {}, you have {}",
                    trade::euros(price),
                    trade::euros(peer.money)
                ));
            }
            let item = wares.parts[place].item.clone();
            let into_trunk = cargo::fits(catalogue, &spec.car, &spec.cargo, &item);
            if !into_trunk && peer.owned.held_at(&shop.name).len() >= SHOP_HOLD {
                return Err(format!(
                    "No room in the trunk for the {}, and {} holds {SHOP_HOLD} parts for you \
                     at most",
                    name(catalogue, &item),
                    shop.name
                ));
            }
            wares.parts.remove(place);
            if into_trunk {
                spec.cargo.push(item);
                drive(peer, spec, desk.baked);
            } else {
                peer.owned.held_at_mut(&shop.name).push(item);
                peer.owned.touch();
            }
            peer.pay(-price, desk.accounts, catalogue);
            Ok(())
        }
        Trade::Sell { from, item } => {
            used()?;
            let mut spec = peer.shadow.spec().cloned().ok_or("You have no car yet")?;
            if peer.shadow.lent() {
                return Err("Not while you drive a lent car".into());
            }
            let gone = || "That's gone already".to_string();
            let item = usize::from(item);
            let price = match from {
                Source::Trunk => {
                    let part = spec.cargo.get(item).ok_or_else(gone)?;
                    let price = trade::buying_price(catalogue, part);
                    spec.cargo.remove(item);
                    drive(peer, spec, desk.baked);
                    price
                }
                Source::Here => {
                    let held = peer.owned.held_at_mut(&shop.name);
                    let part = held.get(item).ok_or_else(gone)?;
                    let price = trade::buying_price(catalogue, part);
                    held.remove(item);
                    peer.owned.touch();
                    price
                }
            };
            peer.pay(price, desk.accounts, catalogue);
            Ok(())
        }
        Trade::BuyCar { ware, trade_in } => {
            if shop.kind != ShopKind::UsedCars {
                return Err(format!("{} doesn't sell used cars", shop.name));
            }
            let wares = desk.stocks.of(at, shop, catalogue, desk.now);
            let place = (wares.cars.iter())
                .position(|w| w.id == ware)
                .ok_or("Someone bought that just before you")?;
            let ware = wares.cars[place].clone();
            // What the dealer pays for the car traded in.
            let credit = match peer.shadow.kept().filter(|_| trade_in) {
                Some((spec, car)) => {
                    let mut tree = spec.car.clone();
                    Condition::of(car).put_on(catalogue, &mut tree);
                    trade::trade_in_price(catalogue, &tree, &spec.cargo)
                }
                None if trade_in => return Err("Your car isn't there yet".into()),
                None => 0,
            };
            let price = ware.price - credit;
            if peer.money < price {
                return Err(format!(
                    "Not enough money: it's {}, you have {}",
                    trade::euros(price),
                    trade::euros(peer.money)
                ));
            }
            let spec = spec_of_trim(catalogue, usize::from(ware.trim), &ware.car)
                .ok_or("That car can't be built")?;
            let tank = spec.build.chassis().tank.litres * USED_TANK;
            let octane = crate::physics::fuel::PUMP_OCTANE;
            let condition = Condition::of_parts(catalogue, &spec.car, tank, octane);
            bought(
                peer,
                (spec, Some(condition)),
                catalogue,
                desk.baked,
                trade_in,
            )?;
            desk.stocks
                .of(at, shop, catalogue, desk.now)
                .cars
                .remove(place);
            peer.pay(-price, desk.accounts, catalogue);
            info!("Player {} bought a used car", peer.id);
            Ok(())
        }
    }
}
