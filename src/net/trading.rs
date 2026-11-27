//! The server's side of trading (see `trade`): what the used parts shops and used car
//! dealers have (made up anew every `trade::STOCK_PERIOD`), buying from them and selling to
//! them. Money changes with what it pays for, in one go (`Peer::pay`).
//!
//! The marketplace is the database's (`db`): what players ask of it goes there
//! (`Accounts::offers`, `offer`, `withdraw`, `buy_offer`), and its answers come back to be
//! told (`answered`). A server without a database has no marketplace.

use std::collections::HashMap;

use bevy::prelude::*;

use super::accounts::{Accounts, CAR, MarketReply, OfferRow, PART};
use super::protocol::ToClient;
use super::server::Peer;
use super::shadow::BakedMaps;
use super::stash::{bought, drive, spec_of_trim, tell};
use crate::cargo;
use crate::catalogue::{Catalogue, Shared};
use crate::items::Item;
use crate::owned::{GARAGE_CARS, GARAGE_PARTS, Parked, ParkedView, SHOP_HOLD, Source, name};
use crate::saved::{self, Condition};
use crate::shops::{Shop, ShopKind};
use crate::trade::{
    self, CarWare, LEAST_PRICE, MOST_PRICE, Market, OfferView, Offered, Offering, Origin, Stock,
    Trade, Ware,
};

/// What a server without a database is told at the marketplace.
const NO_MARKET: &str = "This server keeps no marketplace (it has no database)";

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
    // At the marketplace, the database.
    let market = || {
        (shop.kind == ShopKind::Market)
            .then_some(())
            .ok_or_else(|| format!("{} isn't a marketplace", shop.name))?;
        desk.accounts.ok_or_else(|| NO_MARKET.to_string())
    };
    match asked {
        Trade::Look if shop.kind == ShopKind::Market => {
            market()?.offers(peer.id);
            peer.market_waiting += 1;
            Ok(())
        }
        Trade::Look => Ok(()),
        Trade::Offer { what, price } => {
            let accounts = market()?;
            let account = (peer.account)
                .ok_or("Offering at the marketplace needs an account: a guest can only buy")?;
            if !(LEAST_PRICE..=MOST_PRICE).contains(&price) {
                return Err(format!(
                    "A price is {} to {}",
                    trade::euros(LEAST_PRICE),
                    trade::euros(MOST_PRICE)
                ));
            }
            if peer.shadow.lent() {
                return Err("Not while you drive a lent car".into());
            }
            let gone = || "That's gone already".to_string();
            let (kind, text, pending) = match what {
                Offering::Part { from, item } => {
                    let i = usize::from(item);
                    let part = match from {
                        Origin::Trunk => {
                            let mut spec = peer.shadow.spec().cloned().ok_or_else(gone)?;
                            spec.cargo.get(i).ok_or_else(gone)?;
                            let part = spec.cargo.remove(i);
                            drive(peer, spec, desk.baked);
                            part
                        }
                        Origin::Here => {
                            let held = peer.owned.held_at_mut(&shop.name);
                            held.get(i).ok_or_else(gone)?;
                            held.remove(i)
                        }
                        Origin::Garage => {
                            peer.owned.parts.get(i).ok_or_else(gone)?;
                            peer.owned.parts.remove(i)
                        }
                    };
                    let text = saved::write_items(catalogue, std::slice::from_ref(&part));
                    (PART, text, Pending::Part(part))
                }
                Offering::Car { car } => {
                    let i = usize::from(car);
                    peer.owned.cars.get(i).ok_or_else(gone)?;
                    let parked = peer.owned.cars.remove(i);
                    let text = saved::write(catalogue, &parked.spec, &parked.condition);
                    (CAR, text, Pending::Car(Box::new(parked)))
                }
            };
            peer.owned.touch();
            peer.offering.push_back(pending);
            peer.market_waiting += 1;
            let seller = (account, peer.name.clone().unwrap_or_default());
            let belongings = super::stash::belongings(peer, catalogue);
            accounts.offer(peer.id, seller, (kind, text, price), belongings);
            peer.owned_saved = peer.owned.revision;
            Ok(())
        }
        Trade::Withdraw { offer } => {
            let accounts = market()?;
            let account = peer.account.ok_or("A guest has no offers")?;
            accounts.withdraw(peer.id, account, offer as i64);
            peer.market_waiting += 1;
            Ok(())
        }
        Trade::BuyOffer { offer, price } => {
            let accounts = market()?;
            if peer.money < price {
                return Err(format!(
                    "Not enough money: it's {}, you have {}",
                    trade::euros(price),
                    trade::euros(peer.money)
                ));
            }
            if peer.owned.cars.len() >= GARAGE_CARS || peer.owned.parts.len() >= GARAGE_PARTS {
                return Err("Your garage is full".into());
            }
            accounts.buy_offer(peer.id, peer.account, offer as i64, price);
            peer.market_waiting += 1;
            Ok(())
        }
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

/// Something offered at the marketplace, until the database has said it's up.
pub enum Pending {
    Part(Item),
    Car(Box<Parked>),
}

/// What sold at the marketplace: whose it was, for how much, and what it was called.
pub struct Sold {
    pub seller: super::accounts::AccountId,
    pub price: i64,
    pub what: String,
}

/// What an offer as the database keeps it is of (none if it doesn't read: a definition
/// gone), and as it goes into a garage.
fn read_offer(catalogue: &Shared, row: &OfferRow) -> Option<(Offered, Pending)> {
    if row.what == CAR {
        let (spec, condition) = saved::read(catalogue, &row.data).ok()?;
        let mut car = spec.car.clone();
        condition.put_on(catalogue, &mut car);
        let view = ParkedView {
            car,
            cargo: spec.cargo.clone(),
        };
        Some((
            Offered::Car(view),
            Pending::Car(Box::new(Parked { spec, condition })),
        ))
    } else {
        let item = saved::read_items(catalogue, &row.data).into_iter().next()?;
        Some((Offered::Part(item.clone()), Pending::Part(item)))
    }
}

/// What an offer is called, for people.
fn offer_name(catalogue: &Catalogue, what: &Offered) -> String {
    match what {
        Offered::Part(item) => name(catalogue, item),
        Offered::Car(parked) => crate::trunk::car_name(catalogue, &parked.car),
    }
}

/// Into `peer`'s garage.
fn into_garage(peer: &mut Peer, pending: Pending) {
    match pending {
        Pending::Part(item) => peer.owned.parts.push(item),
        Pending::Car(parked) => peer.owned.cars.push(*parked),
    }
    peer.owned.touch();
}

/// Tells `peer` what the database answered to what they asked of the marketplace (and asks
/// it for the offers again after a change, with `accounts`). What they bought, if they did,
/// for the seller to be paid.
pub(super) fn answered(
    peer: &mut Peer,
    reply: MarketReply,
    accounts: Option<&Accounts>,
    shops: &[Shop],
    catalogue: &Shared,
) -> Option<Sold> {
    peer.market_waiting = peer.market_waiting.saturating_sub(1);
    let mut sold = None;
    let why = match reply {
        MarketReply::Offers(Ok(rows)) => {
            let offers = (rows.iter())
                .filter_map(|row| {
                    let (what, _) = read_offer(catalogue, row)?;
                    Some(OfferView {
                        id: row.id as u64,
                        seller: row.seller_name.clone(),
                        mine: peer.account == Some(row.seller),
                        price: row.price,
                        what,
                    })
                })
                .collect();
            peer.tell(&ToClient::Market(Box::new(Market { offers })));
            return None;
        }
        MarketReply::Offered(Ok(_)) => {
            peer.offering.pop_front();
            None
        }
        MarketReply::Offered(Err(why)) => {
            // Back, into the garage.
            if let Some(pending) = peer.offering.pop_front() {
                into_garage(peer, pending);
            }
            Some(format!("{why}: it's in your garage"))
        }
        MarketReply::Withdrawn(Ok(row)) => {
            if let Some((_, pending)) = read_offer(catalogue, &row) {
                into_garage(peer, pending);
            }
            None
        }
        MarketReply::Bought(Ok(row)) => {
            if let Some((what, pending)) = read_offer(catalogue, &row) {
                let called = offer_name(catalogue, &what);
                peer.tell(&ToClient::Chat {
                    from: super::protocol::SERVER,
                    text: format!(
                        "You bought the {called} for {}: it's in your garage",
                        trade::euros(row.price)
                    ),
                });
                into_garage(peer, pending);
                sold = Some(Sold {
                    seller: row.seller,
                    price: row.price,
                    what: called,
                });
            }
            peer.paid(-row.price);
            None
        }
        MarketReply::Offers(Err(why))
        | MarketReply::Withdrawn(Err(why))
        | MarketReply::Bought(Err(why)) => Some(why),
    };
    if let Some(why) = &why {
        info!("Player {}: the marketplace: {why}", peer.id);
    }
    if let Some(accounts) = accounts {
        accounts.offers(peer.id);
        peer.market_waiting += 1;
    }
    tell(peer, shops, why);
    sold
}
