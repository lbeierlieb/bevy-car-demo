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

use crate::catalogue::Catalogue;
use crate::items::{Item, Kind, Panel, Wear};
use crate::net::wire::{Reader, Wire};
use crate::owned::Source;
use crate::shops::Shop;

/// What a new player has (cents): an account when it's made, a guest when they join.
pub const START_MONEY: i64 = 500_000;

/// How long a used parts shop's assortment lasts before new things come in (s).
pub const STOCK_PERIOD: f64 = 1800.0;
/// How many things a used parts shop has at a time (fewer if it sells fewer kinds).
pub const USED_PARTS: usize = 12;

/// What a used parts shop asks for a part, and pays for one, as shares of what it's worth
/// (new, less its wear).
const SELLS_AT: f64 = 0.5;
const BUYS_AT: f64 = 0.15;

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

/// What a shop that trades has, as a player at it is told.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stock {
    /// The shop (its place in the server's list).
    pub shop: u16,
    /// A used parts shop's wares.
    pub parts: Vec<Ware>,
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

impl Wire for Stock {
    fn put(&self, out: &mut Vec<u8>) {
        self.shop.put(out);
        self.parts.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            shop: r.get()?,
            parts: r.get()?,
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
            _ => return None,
        })
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
        ] {
            assert_eq!(decode_with(&encode(&trade), None), Some(trade));
        }
    }
}
