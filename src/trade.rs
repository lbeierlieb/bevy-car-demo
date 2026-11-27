//! Money and trading. Every player has a wallet (an account's is kept in the database, a
//! guest's for the session), in cents. It changes only on the server, in one go with what it
//! pays for (see `net::trading` and `db`); the game is told it with what the player owns
//! (`OwnedView::money`).
//!
//! **Used parts shops** buy anything a player brings (in the trunk, or held for them there)
//! for a little (`buying_price`; it's gone then) and sell worn parts the server makes up
//! (`used_parts`): an assortment from the shop's `sells` lines, changing every
//! `STOCK_PERIOD`, each with random wear, priced by what it's worth new and how worn it is
//! (`selling_price`). While every price in the catalogue is 0, what a part is worth new is a
//! guess by its kind (`new_value`).
//!
//! **Used car dealers** sell worn stock cars (`used_cars`): trims from their `sells` lines
//! with a mileage, their parts worn by it (tires and pads anyhow), in a paint of any kind,
//! priced by what they're worth as worn (`car_price`). Buying one works as at a car dealer
//! (the old car into the garage, or traded in: here it's worth `trade_in_price`).
//!
//! **The marketplace**: players offer parts (from the trunk, held for them there, or from
//! their garage) and cars from their garage, at a price of their own; what's offered is
//! locked there (gone from what they own) until it's bought or they take it back (into
//! their garage). Whoever buys it pays the seller (even one who's away), and it's shipped
//! to the buyer's garage. The offers are kept in the server's database, shared by every
//! instance of it (`db`).

use crate::catalogue::Catalogue;
use crate::items::{Item, Kind, Panel, Wear};
use crate::net::wire::{Reader, Wire};
use crate::owned::{ParkedView, Source};
use crate::shops::Shop;

/// What a new player has (cents): an account when it's made, a guest when they join.
pub const START_MONEY: i64 = 500_000;

/// How long a used parts shop's assortment lasts before new things come in (s).
pub const STOCK_PERIOD: f64 = 1800.0;
/// How many things a used parts shop has at a time (fewer if it sells fewer kinds).
pub const USED_PARTS: usize = 12;

/// How many cars a used car dealer has at a time (none if it sells no trims).
pub const USED_CARS: usize = 4;
/// The mileage of a used car (km).
pub const MILEAGE: std::ops::Range<f32> = 25_000.0..240_000.0;

/// What a used parts shop asks for a part, and pays for one, and what a used car dealer
/// asks for a car and pays for one traded in, as shares of what it's worth (new, less its
/// wear).
const SELLS_AT: f64 = 0.5;
const BUYS_AT: f64 = 0.15;
const CARS_AT: f64 = 0.4;
const TRADES_IN_AT: f64 = 0.15;
/// What a used car loses of its price by its mileage, per 250 000 km (most of its parts
/// don't wear, so its wear alone would hardly show).
const MILEAGE_LOSS: f64 = 0.45;

/// An amount for people: "5 000 €", "12.50 €", "−300 €".
pub fn euros(cents: i64) -> String {
    let sign = if cents < 0 { "−" } else { "" };
    let cents = cents.unsigned_abs();
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    match cents % 100 {
        0 => format!("{sign}{grouped} €"),
        rest => format!("{sign}{grouped}.{rest:02} €"),
    }
}

/// `cents` rounded to whole euros.
fn whole_euros(cents: f64) -> i64 {
    (cents / 100.0).round() as i64 * 100
}

/// What a part of its own (without what's fitted into it) is worth new (cents): its
/// definition's price, or while that's 0 (every price is, for now) a guess by its kind,
/// more for tuning parts.
fn own_value(catalogue: &Catalogue, item: &Item) -> i64 {
    let def = item.def(catalogue);
    if def.absent {
        return 0;
    }
    if def.price > 0 {
        return i64::from(def.price);
    }
    let euros = match def.kind {
        Kind::Chassis => 3000,
        Kind::Body(Panel::Mirror) => 60,
        Kind::Body(Panel::FrontLip | Panel::SideSkirt | Panel::Diffuser) => 150,
        Kind::Body(Panel::RearSpoiler) => 200,
        Kind::Body(_) => 250,
        Kind::Seat(_) => 150,
        // A rim and a tire, each worth its own.
        Kind::Wheel | Kind::Engine => 0,
        Kind::Rim => 80,
        Kind::Tire => 70,
        Kind::Brake => 110,
        Kind::Spring => 60,
        Kind::Damper => 90,
        Kind::AntiRollBar => 70,
        Kind::Gearbox => 700,
        Kind::FinalDrive => 150,
        Kind::Differential => 250,
        Kind::Clutch => 160,
        Kind::Ecu => 250,
        Kind::LongBlock => 400,
        Kind::EnginePart(_) => 150,
    };
    const TUNING: [&str; 14] = [
        "sport",
        "race",
        "track",
        "big",
        "forged",
        "programmable",
        "limited-slip",
        "turbo",
        "super",
        "bucket",
        "wing",
        "cut-out",
        "close-ratio",
        "roots",
    ];
    let tuning = TUNING.iter().any(|word| def.id.contains(word));
    let euros = if tuning { euros * 5 / 2 } else { euros };
    euros * 100
}

/// What a part is worth new, with what's fitted into it (cents).
pub fn new_value(catalogue: &Catalogue, item: &Item) -> i64 {
    own_value(catalogue, item)
        + item
            .slots
            .iter()
            .flatten()
            .map(|i| new_value(catalogue, i))
            .sum::<i64>()
}

/// How worn a part of its own is: its worst cylinder's or rib's (0 new, 1 failed).
pub fn worst_wear(item: &Item) -> f32 {
    item.wear
        .iter()
        .copied()
        .fold(0.0, f32::max)
        .clamp(0.0, 1.0)
}

/// What a part is worth as worn as it is, with what's fitted into it (cents): each part's
/// value new less its share worn.
pub fn worn_value(catalogue: &Catalogue, item: &Item) -> i64 {
    let own = own_value(catalogue, item) as f64 * f64::from(1.0 - worst_wear(item));
    own.round() as i64
        + (item.slots.iter().flatten())
            .map(|i| worn_value(catalogue, i))
            .sum::<i64>()
}

/// How worn a part is with what's fitted into it, by what each is worth (0 new, 1 all
/// failed): a car's or an engine's, for people.
pub fn wear_share(catalogue: &Catalogue, item: &Item) -> f32 {
    let new = new_value(catalogue, item);
    if new <= 0 {
        return worst_wear(item);
    }
    (1.0 - worn_value(catalogue, item) as f64 / new as f64).clamp(0.0, 1.0) as f32
}

/// How worn a part is, with what's fitted into it, for people: "like new", "worn 40 %",
/// "failed" (a part of its own past its limit).
pub fn condition(catalogue: &Catalogue, item: &Item) -> String {
    let share = wear_share(catalogue, item);
    if item.slots.iter().all(Option::is_none) && worst_wear(item) >= 1.0 {
        "failed".into()
    } else if share < 0.005 {
        "like new".into()
    } else {
        format!("worn {:.0} %", 100.0 * share)
    }
}

/// What a used parts shop asks for `item` (at least 5 €).
pub fn selling_price(catalogue: &Catalogue, item: &Item) -> i64 {
    whole_euros(worn_value(catalogue, item) as f64 * SELLS_AT).max(500)
}

/// What a used parts shop pays for `item`: a little (at least 1 €).
pub fn buying_price(catalogue: &Catalogue, item: &Item) -> i64 {
    whole_euros(worn_value(catalogue, item) as f64 * BUYS_AT).max(100)
}

/// What a used car dealer asks for `car` (a car's tree, as worn as it is) with `km` on
/// the clock, in fifties.
pub fn car_price(catalogue: &Catalogue, car: &Item, km: f32) -> i64 {
    let age = 1.0 - MILEAGE_LOSS * f64::from(km / 250_000.0).clamp(0.0, 1.0);
    let cents = worn_value(catalogue, car) as f64 * CARS_AT * age;
    ((cents / 5000.0).round() as i64 * 5000).max(5000)
}

/// What a used car dealer pays for `car` traded in (as worn as it is) with what's in its
/// trunk (`cargo`), in fifties.
pub fn trade_in_price(catalogue: &Catalogue, car: &Item, cargo: &[Item]) -> i64 {
    let cents = (std::iter::once(car).chain(cargo))
        .map(|item| worn_value(catalogue, item) as f64)
        .sum::<f64>()
        * TRADES_IN_AT;
    (cents / 5000.0).round() as i64 * 5000
}

/// Random numbers for making up wares (splitmix64): the same seed, the same wares.
pub struct Dice(u64);

impl Dice {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn roll(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Evenly in 0..1.
    pub fn unit(&mut self) -> f32 {
        (self.roll() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Evenly in `low..high`.
    pub fn within(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    /// Evenly one of 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.roll() % n.max(1) as u64) as usize
    }
}

/// A seed from words and numbers (a shop's name and the period, say).
pub fn seed(of: &str, number: u64) -> u64 {
    // FNV-1a: the same everywhere, unlike std's hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in of.bytes().chain(number.to_le_bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// How many values a kind of wear has on a loose part: a four-cylinder engine's per
/// cylinder, a tire's ribs, a gearbox's forward gears.
fn wear_values(wear: Wear) -> usize {
    match wear {
        Wear::Pistons | Wear::Rods | Wear::Valves => 4,
        Wear::Tread => crate::physics::tire::RIBS,
        Wear::Gears => 5,
        _ => 1,
    }
}

/// How much of a part's wear a kind of wear gets (the engine's insides less, so nothing
/// made up has failed), and the most it gets.
fn wear_scale(wear: Wear) -> (f32, f32) {
    match wear {
        Wear::Pistons | Wear::Rods | Wear::HeadGasket | Wear::Bearings => (0.4, 0.5),
        Wear::Valves => (0.1, 0.2),
        Wear::Turbo => (0.5, 0.6),
        Wear::Gears => (0.5, 0.6),
        Wear::Tread | Wear::Pads => (1.0, 0.9),
        Wear::Clutch => (0.9, 0.85),
    }
}

/// Wears every part of `item` (and what's fitted into it) that wears, about `level` (0..1)
/// each, varying.
pub fn wear_out(catalogue: &Catalogue, item: &mut Item, level: f32, dice: &mut Dice) {
    if let Some(wear) = item.def(catalogue).wear {
        let (scale, most) = wear_scale(wear);
        let values = (0..wear_values(wear))
            .map(|_| (level * scale * dice.within(0.6, 1.4)).clamp(0.0, most))
            .collect();
        item.wear = crate::saved::trimmed(values);
    }
    for child in item.slots.iter_mut().flatten() {
        wear_out(catalogue, child, level, dice);
    }
}

/// The sizes the catalogue's trims have of a kind of part (rims, tires): what a used part
/// of that kind comes in.
fn trim_sizes(catalogue: &Catalogue, kind: Kind) -> Vec<usize> {
    let mut sizes: Vec<usize> = (catalogue.trims.iter())
        .flat_map(|trim| trim.car.all())
        .filter(|item| item.def(catalogue).kind == kind)
        .map(|item| item.variant)
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

/// A used part of the definition at `def` (made up): a size of those it comes in that a
/// stock car has, any paint.
fn made_up(catalogue: &Catalogue, def: usize, dice: &mut Dice) -> Item {
    let made = &catalogue.defs[def];
    let paint = dice.below(catalogue.paints.len());
    let mut item = Item::new(catalogue, def, paint);
    item.variant = match made.kind {
        Kind::Rim | Kind::Tire => {
            let sizes = trim_sizes(catalogue, made.kind);
            sizes.get(dice.below(sizes.len())).copied().unwrap_or(0)
        }
        Kind::AntiRollBar => {
            let chassis = (catalogue.chassis.iter())
                .position(|c| made.fits.is_empty() || made.fits.contains(&c.name));
            dice.below(made.variants(catalogue, chassis).max(1))
        }
        _ => 0,
    };
    item
}

/// A used part on sale, numbered by the server.
#[derive(Clone, Debug, PartialEq)]
pub struct Ware {
    pub id: u32,
    pub item: Item,
    pub price: i64,
}

/// What a used parts shop has in period `period` (made up from its `sells` lines; the same
/// for the same `seed`, its numbers starting at `first_id`).
pub fn used_parts(catalogue: &Catalogue, shop: &Shop, seed: u64, first_id: u32) -> Vec<Ware> {
    let mut dice = Dice::new(seed);
    let mut kinds: Vec<Option<usize>> = (catalogue.defs.iter().enumerate())
        .filter(|(_, def)| !def.absent && def.kind != Kind::Chassis && shop.sells(&def.id))
        .map(|(def, _)| Some(def))
        .collect();
    let assemblies: Vec<usize> = (catalogue.assemblies.iter().enumerate())
        .filter(|(_, assembly)| shop.sells(&assembly.id))
        .map(|(a, _)| a)
        .collect();
    kinds.extend(assemblies.iter().map(|_| None));
    let mut assemblies = assemblies.into_iter();
    // In a random order: the first `USED_PARTS`.
    let mut kinds: Vec<(u64, Item)> = kinds
        .into_iter()
        .map(|kind| {
            let item = match kind {
                Some(def) => made_up(catalogue, def, &mut dice),
                None => {
                    let a = assemblies.next().expect("as many as counted");
                    catalogue.assemblies[a].item.clone()
                }
            };
            (dice.roll(), item)
        })
        .collect();
    kinds.sort_by_key(|(order, _)| *order);
    kinds.truncate(USED_PARTS);
    (kinds.into_iter().enumerate())
        .map(|(i, (_, mut item))| {
            let level = dice.within(0.05, 0.8);
            wear_out(catalogue, &mut item, level, &mut dice);
            Ware {
                id: first_id.wrapping_add(i as u32),
                price: selling_price(catalogue, &item),
                item,
            }
        })
        .collect()
}

/// Wears a car's parts as `km` would: tires and pads anyhow (they're changed now and
/// then), the clutch, gears and engine by the mileage, nothing failed.
pub fn wear_by_mileage(catalogue: &Catalogue, car: &mut Item, km: f32, dice: &mut Dice) {
    let miles = (km / 250_000.0).clamp(0.0, 1.0);
    if let Some(wear) = car.def(catalogue).wear {
        let (level, most) = match wear {
            Wear::Tread | Wear::Pads => (dice.within(0.1, 0.8), 0.85),
            Wear::Clutch => (miles * dice.within(0.6, 1.2), 0.85),
            Wear::Gears => (miles * 0.5, 0.6),
            Wear::Pistons | Wear::Rods | Wear::HeadGasket | Wear::Bearings => (miles * 0.4, 0.5),
            Wear::Valves => (miles * 0.05, 0.1),
            Wear::Turbo => (miles * 0.5, 0.6),
        };
        let values = (0..wear_values(wear))
            .map(|_| match wear {
                // A tire's ribs alike, more or less.
                Wear::Tread => (level * dice.within(0.9, 1.1)).clamp(0.0, most),
                _ => (level * dice.within(0.6, 1.4)).clamp(0.0, most),
            })
            .collect();
        car.wear = crate::saved::trimmed(values);
    }
    for child in car.slots.iter_mut().flatten() {
        wear_by_mileage(catalogue, child, km, dice);
    }
}

/// A used car on sale, numbered by the server: a stock car (the catalogue's trim `trim`),
/// worn, in its paint.
#[derive(Clone, Debug, PartialEq)]
pub struct CarWare {
    pub id: u32,
    pub trim: u16,
    pub car: Item,
    /// Its mileage (km).
    pub km: u32,
    pub price: i64,
}

/// What a used car dealer has (made up from its `sells` lines' trims; the same for the same
/// `seed`, its numbers starting at `first_id`).
pub fn used_cars(catalogue: &Catalogue, shop: &Shop, seed: u64, first_id: u32) -> Vec<CarWare> {
    let mut dice = Dice::new(seed);
    let trims: Vec<usize> = (catalogue.trims.iter().enumerate())
        .filter(|(_, trim)| shop.sells(&trim.id))
        .map(|(t, _)| t)
        .collect();
    if trims.is_empty() {
        return Vec::new();
    }
    (0..USED_CARS)
        .map(|i| {
            let trim = trims[dice.below(trims.len())];
            let mut car = catalogue.trims[trim].car.clone();
            let paint = dice.below(catalogue.paints.len());
            paint_all(catalogue, &mut car, paint);
            let km = (dice.within(MILEAGE.start, MILEAGE.end) / 1000.0).round() * 1000.0;
            wear_by_mileage(catalogue, &mut car, km, &mut dice);
            CarWare {
                id: first_id.wrapping_add(500 + i as u32),
                trim: trim as u16,
                price: car_price(catalogue, &car, km),
                car,
                km: km as u32,
            }
        })
        .collect()
}

/// Every painted part of `item` (and what's fitted into it) in `paint`.
fn paint_all(catalogue: &Catalogue, item: &mut Item, paint: usize) {
    if item.def(catalogue).painted {
        item.paint = Some(paint);
    }
    for child in item.slots.iter_mut().flatten() {
        paint_all(catalogue, child, paint);
    }
}

/// What a shop that trades has, as a player at it is told.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stock {
    /// The shop (its place in the server's list).
    pub shop: u16,
    /// A used parts shop's wares.
    pub parts: Vec<Ware>,
    /// A used car dealer's.
    pub cars: Vec<CarWare>,
}

/// What a player asks of a shop that trades, at it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trade {
    /// What it has (answered with `ToClient::Stock`).
    Look,
    /// A used part, by its number.
    BuyPart { ware: u32 },
    /// A part sold to the used parts shop: from the trunk, or what it holds for them.
    Sell { from: Source, item: u16 },
    /// A used car, by its number: the car driven goes into the garage, or is traded in.
    BuyCar { ware: u32, trade_in: bool },
    /// At the marketplace: something offered for `price` cents.
    Offer { what: Offering, price: i64 },
    /// At the marketplace: our offer taken back (into the garage).
    Withdraw { offer: u64 },
    /// At the marketplace: an offer bought for `price` (what we were shown).
    BuyOffer { offer: u64, price: i64 },
}

/// The least and most an offer at the marketplace may ask (cents).
pub const LEAST_PRICE: i64 = 100;
pub const MOST_PRICE: i64 = 100_000_000;

/// Where a part offered at the marketplace is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Trunk,
    /// Held for us at the marketplace.
    Here,
    Garage,
}

/// What's offered at the marketplace: a part (by its place in the list it's in) or a car
/// in the garage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offering {
    Part { from: Origin, item: u16 },
    Car { car: u16 },
}

/// What an offer is of.
#[derive(Clone, Debug, PartialEq)]
pub enum Offered {
    Part(Item),
    Car(ParkedView),
}

impl Offered {
    /// The part, or the car's tree.
    pub fn item(&self) -> &Item {
        match self {
            Offered::Part(item) => item,
            Offered::Car(parked) => &parked.car,
        }
    }
}

/// An offer at the marketplace as players see it.
#[derive(Clone, Debug, PartialEq)]
pub struct OfferView {
    pub id: u64,
    pub seller: String,
    /// Ours.
    pub mine: bool,
    pub price: i64,
    pub what: Offered,
}

/// The offers at the marketplace.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Market {
    pub offers: Vec<OfferView>,
}

/// The prices a price is picked from (cents): 1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 € and
/// every tenfold of them up to `MOST_PRICE`.
pub fn price_steps() -> Vec<i64> {
    const STEPS: [i64; 10] = [100, 120, 150, 200, 250, 300, 400, 500, 600, 800];
    let mut steps = Vec::new();
    let mut scale = 1;
    while STEPS[0] * scale < MOST_PRICE {
        steps.extend(STEPS.iter().map(|step| step * scale));
        scale *= 10;
    }
    steps.push(MOST_PRICE);
    steps
}

/// The step of `price_steps` nearest to `price`.
pub fn nearest_step(price: i64) -> usize {
    let steps = price_steps();
    (0..steps.len())
        .min_by_key(|&i| (steps[i] - price).abs())
        .unwrap_or(0)
}

impl Wire for Ware {
    fn put(&self, out: &mut Vec<u8>) {
        self.id.put(out);
        self.item.put(out);
        self.price.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            id: r.get()?,
            item: r.get()?,
            price: r.get()?,
        })
    }
}

impl Wire for CarWare {
    fn put(&self, out: &mut Vec<u8>) {
        self.id.put(out);
        self.trim.put(out);
        self.car.put(out);
        self.km.put(out);
        self.price.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let ware = Self {
            id: r.get()?,
            trim: r.get()?,
            car: r.get()?,
            km: r.get()?,
            price: r.get()?,
        };
        let catalogue = r.catalogue()?;
        (usize::from(ware.trim) < catalogue.trims.len()
            && ware.car.def(catalogue).kind == Kind::Chassis)
            .then_some(ware)
    }
}

impl Wire for Stock {
    fn put(&self, out: &mut Vec<u8>) {
        self.shop.put(out);
        self.parts.put(out);
        self.cars.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            shop: r.get()?,
            parts: r.get()?,
            cars: r.get()?,
        })
    }
}

impl Wire for Trade {
    fn put(&self, out: &mut Vec<u8>) {
        match *self {
            Trade::Look => 0u8.put(out),
            Trade::BuyPart { ware } => (1u8, ware).put(out),
            Trade::Sell { from, item } => {
                2u8.put(out);
                from.put(out);
                item.put(out);
            }
            Trade::BuyCar { ware, trade_in } => {
                3u8.put(out);
                ware.put(out);
                trade_in.put(out);
            }
            Trade::Offer { what, price } => {
                4u8.put(out);
                what.put(out);
                price.put(out);
            }
            Trade::Withdraw { offer } => (5u8, offer).put(out),
            Trade::BuyOffer { offer, price } => {
                6u8.put(out);
                offer.put(out);
                price.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Trade::Look,
            1 => Trade::BuyPart { ware: r.get()? },
            2 => Trade::Sell {
                from: r.get()?,
                item: r.get()?,
            },
            3 => Trade::BuyCar {
                ware: r.get()?,
                trade_in: r.get()?,
            },
            4 => Trade::Offer {
                what: r.get()?,
                price: r.get()?,
            },
            5 => Trade::Withdraw { offer: r.get()? },
            6 => Trade::BuyOffer {
                offer: r.get()?,
                price: r.get()?,
            },
            _ => return None,
        })
    }
}

impl Wire for Offering {
    fn put(&self, out: &mut Vec<u8>) {
        match *self {
            Offering::Part { from, item } => {
                let from = match from {
                    Origin::Trunk => 0u8,
                    Origin::Here => 1,
                    Origin::Garage => 2,
                };
                (0u8, from).put(out);
                item.put(out);
            }
            Offering::Car { car } => (1u8, car).put(out),
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Offering::Part {
                from: match r.get::<u8>()? {
                    0 => Origin::Trunk,
                    1 => Origin::Here,
                    2 => Origin::Garage,
                    _ => return None,
                },
                item: r.get()?,
            },
            1 => Offering::Car { car: r.get()? },
            _ => return None,
        })
    }
}

impl Wire for OfferView {
    fn put(&self, out: &mut Vec<u8>) {
        self.id.put(out);
        self.seller.put(out);
        self.mine.put(out);
        self.price.put(out);
        match &self.what {
            Offered::Part(item) => {
                0u8.put(out);
                item.put(out);
            }
            Offered::Car(parked) => {
                1u8.put(out);
                parked.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let (id, seller, mine, price) = (r.get()?, r.get()?, r.get()?, r.get()?);
        let what = match r.get::<u8>()? {
            0 => {
                let item: Item = r.get()?;
                (item.def(r.catalogue()?).kind != Kind::Chassis).then_some(())?;
                Offered::Part(item)
            }
            1 => Offered::Car(r.get()?),
            _ => return None,
        };
        Some(Self {
            id,
            seller,
            mine,
            price,
            what,
        })
    }
}

impl Wire for Market {
    fn put(&self, out: &mut Vec<u8>) {
        self.offers.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self { offers: r.get()? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;
    use crate::net::wire::{decode_with, encode};

    #[test]
    fn amounts_read_well() {
        assert_eq!(euros(START_MONEY), "5 000 €");
        assert_eq!(euros(0), "0 €");
        assert_eq!(euros(1250), "12.50 €");
        assert_eq!(euros(-30_000), "−300 €");
        assert_eq!(euros(123_456_789), "1 234 567.89 €");
    }

    /// A used parts shop's wares: what it sells, worn, cheaper the more worn, the same for
    /// the same seed and others for another; what it pays is less than what it asks.
    #[test]
    fn used_parts_are_made_up_worn_and_priced() {
        let catalogue = builtin();
        let shop = Shop {
            kind: crate::shops::ShopKind::Used,
            name: "Teile".into(),
            at: Default::default(),
            sells: vec!["*".into()],
        };
        let wares = used_parts(&catalogue, &shop, seed("Teile", 3), 100);
        assert_eq!(wares.len(), USED_PARTS);
        assert_eq!(wares, used_parts(&catalogue, &shop, seed("Teile", 3), 100));
        assert_ne!(wares, used_parts(&catalogue, &shop, seed("Teile", 4), 100));
        assert_eq!(wares[3].id, 103);
        for ware in &wares {
            let def = ware.item.def(&catalogue);
            assert!(!def.absent && def.kind != Kind::Chassis, "{}", def.id);
            assert!(
                ware.item.fine(&catalogue) || !def.slots.is_empty(),
                "{}",
                def.id
            );
            assert!(ware.price >= 500);
            assert!(
                buying_price(&catalogue, &ware.item) < ware.price,
                "{}",
                def.id
            );
        }
        assert!(
            wares.iter().any(|ware| worst_wear(&ware.item) > 0.0
                || ware.item.all().iter().any(|i| worst_wear(i) > 0.0)),
            "something worn"
        );
        // Worn, cheaper.
        let tire = (catalogue.defs.iter())
            .position(|def| def.kind == Kind::Tire)
            .unwrap();
        let mut item = Item::new(&catalogue, tire, 0);
        let new = selling_price(&catalogue, &item);
        item.wear = vec![0.6; 5];
        assert!(selling_price(&catalogue, &item) < new);
        assert!(wear_share(&catalogue, &item) > 0.5);
        // Only what it sells.
        let tires = Shop {
            sells: vec!["tire.*".into()],
            ..shop
        };
        let wares = used_parts(&catalogue, &tires, 1, 0);
        assert!(!wares.is_empty() && wares.len() <= USED_PARTS);
        for ware in &wares {
            assert!(ware.item.def(&catalogue).id.starts_with("tire."));
        }
        // Over the wire.
        let stock = Stock {
            shop: 2,
            parts: wares,
            cars: Vec::new(),
        };
        let read: Stock = decode_with(&encode(&stock), Some(&catalogue)).unwrap();
        assert_eq!(read, stock);
        for trade in [
            Trade::Look,
            Trade::BuyPart { ware: 7 },
            Trade::Sell {
                from: Source::Trunk,
                item: 3,
            },
            Trade::BuyCar {
                ware: 9,
                trade_in: true,
            },
            Trade::Offer {
                what: Offering::Part {
                    from: Origin::Garage,
                    item: 4,
                },
                price: 12_000,
            },
            Trade::Offer {
                what: Offering::Car { car: 1 },
                price: 300_000,
            },
            Trade::Withdraw { offer: 77 },
            Trade::BuyOffer {
                offer: 78,
                price: 900,
            },
        ] {
            assert_eq!(decode_with(&encode(&trade), None), Some(trade));
        }
    }

    /// A used car dealer's cars: stock cars it sells, worn by their mileage (the more, the
    /// cheaper), each a car the catalogue allows; worth less traded in than sold.
    #[test]
    fn used_cars_are_worn_by_their_mileage() {
        let catalogue = builtin();
        let shop = Shop {
            kind: crate::shops::ShopKind::UsedCars,
            name: "Wagen".into(),
            at: Default::default(),
            sells: vec!["vireo.*".into()],
        };
        let cars = used_cars(&catalogue, &shop, 7, 0);
        assert_eq!(cars.len(), USED_CARS);
        for ware in &cars {
            assert!(ware.car.fine(&catalogue));
            assert!(ware.car.resolve(&catalogue).is_ok());
            assert!(MILEAGE.contains(&(ware.km as f32)));
            assert!(wear_share(&catalogue, &ware.car) > 0.01);
            let new = new_value(&catalogue, &ware.car);
            assert!(ware.price < new / 2, "{} of {}", ware.price, new);
            assert!(trade_in_price(&catalogue, &ware.car, &[]) < ware.price);
        }
        // Afforded with what a new player has.
        assert!(cars.iter().all(|ware| ware.price <= START_MONEY));
        let mut dice = Dice::new(1);
        let trim = &catalogue.trims[0].car;
        let (mut low, mut high) = (trim.clone(), trim.clone());
        wear_by_mileage(&catalogue, &mut low, 30_000.0, &mut dice);
        let mut dice = Dice::new(1);
        wear_by_mileage(&catalogue, &mut high, 230_000.0, &mut dice);
        assert!(car_price(&catalogue, &high, 230_000.0) < car_price(&catalogue, &low, 30_000.0));
        let none = Shop {
            sells: Vec::new(),
            ..shop
        };
        assert!(used_cars(&catalogue, &none, 7, 0).is_empty());
        let stock = Stock {
            shop: 0,
            parts: Vec::new(),
            cars,
        };
        let read: Stock = decode_with(&encode(&stock), Some(&catalogue)).unwrap();
        assert_eq!(read, stock);
    }

    #[test]
    fn prices_are_picked_in_steps_and_offers_go_over_the_wire() {
        let steps = price_steps();
        assert_eq!(steps[0], LEAST_PRICE);
        assert_eq!(*steps.last().unwrap(), MOST_PRICE);
        assert!(steps.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(steps[nearest_step(149_000)], 150_000);
        assert_eq!(nearest_step(1), 0);
        let catalogue = builtin();
        let car = catalogue.trims[0].car.clone();
        let part = car.slots.iter().flatten().next().unwrap().clone();
        let market = Market {
            offers: vec![
                OfferView {
                    id: 3,
                    seller: "Anna".into(),
                    mine: false,
                    price: 1500,
                    what: Offered::Part(part),
                },
                OfferView {
                    id: 4,
                    seller: "Ben".into(),
                    mine: true,
                    price: 300_000,
                    what: Offered::Car(ParkedView {
                        car: car.clone(),
                        cargo: Vec::new(),
                    }),
                },
            ],
        };
        let read: Market = decode_with(&encode(&market), Some(&catalogue)).unwrap();
        assert_eq!(read, market);
        // A car isn't a part.
        let wrong = Market {
            offers: vec![OfferView {
                what: Offered::Part(car),
                ..market.offers[0].clone()
            }],
        };
        assert!(decode_with::<Market>(&encode(&wrong), Some(&catalogue)).is_none());
    }
}
