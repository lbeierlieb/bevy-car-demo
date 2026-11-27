//! Changing a car at shops, as the server does it: a player's game asks for `Work`s (fit
//! a part into a slot, take one off, set a part's knob, paint a part, load a tune, make or
//! take apart a wheel), the server checks each against the rules (which shop does what,
//! what the shop sells, what fits) and does them all or none (`Bench`), and tells the
//! player the car it has now. The games use the same rules to show what can be done.
//!
//! - Car workshops and tuning shops fit any part but a long block's insides (the engine
//!   shop's) and a wheel's rim and tire (the tire shop's); engines come out whole, and
//!   only for another. They set the final drive's ratio.
//! - Tire shops fit wheels, and rims and tires on the car's wheels; they make loose wheels
//!   of a rim and a tire, and take them apart.
//! - Alignment shops set the chassis', dampers' and tires' knobs (brakes, camber and toe,
//!   pressure), dynos the engine's (cam timing, boost) and the ECU's tune, paint shops
//!   paint any painted part.
//! - A new part is one the shop sells (its list in the shops file), made for the car's
//!   chassis; parts brought come from the trunk or what the shop holds for the player.
//! - What comes off goes into the trunk while there's room, else the shop holds it (up to
//!   `SHOP_HOLD`), else it's thrown away. A part standing for nothing (no turbo) is never
//!   kept.

use crate::catalogue::{Catalogue, Shared};
use crate::items::{Does, Item, Kind, PartDef};
use crate::net::wire::{Reader, Wire};
use crate::owned::{SHOP_HOLD, name};
use crate::physics::engine::ecu::Ecu;
use crate::shops::{Shop, ShopKind};
use crate::spec::CarSpec;
use crate::wheels::{Rim, TireSize, Wheel};

/// Where a slot is: the places of the slots from the car down (the car itself: none).
pub type Path = Vec<u8>;

/// The most works in one request, and slots in one fit.
const MOST_WORKS: usize = 64;
const MOST_SLOTS: usize = 8;

/// Where a part comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Supply {
    /// New from the shop: a part (`Catalogue::defs`) in a size (`variant`), painted
    /// `paint` if it's painted.
    Part {
        def: u16,
        variant: u16,
        paint: u16,
    },
    /// New from the shop, put together (`Catalogue::assemblies`).
    Assembly(u16),
    /// Brought: in the trunk, or held for the player at this shop.
    Trunk(u16),
    Held(u16),
    /// A wheel made of a rim and a tire (at a tire shop).
    Wheel(Box<Supply>, Box<Supply>),
}

/// What a player asks done to their car.
#[derive(Clone, Debug, PartialEq)]
pub enum Work {
    /// A part into each of `slots` (several only of new parts, one each).
    Fit { slots: Vec<Path>, supply: Supply },
    /// The part in `slot` off (an optional one, or one with a part standing for nothing).
    Remove { slot: Path },
    /// Knob `knob` of the part at `at` set to `value`.
    Knob { at: Path, knob: u8, value: f32 },
    /// The part at `at` painted.
    Paint { at: Path, paint: u16 },
    /// The ECU's tune.
    Tune(Box<Ecu>),
    /// A loose wheel made of a rim and a tire (`Supply::Wheel`), into the trunk.
    MakeWheel(Supply),
    /// A loose wheel in the trunk taken apart into its rim and tire.
    Split { item: u16 },
}

/// The kind of slot at `path` in `car`, and the kind of what holds it (none: it's the
/// car's own).
pub fn slot_kinds(catalogue: &Catalogue, car: &Item, path: &[u8]) -> Option<(Kind, Option<Kind>)> {
    let (&last, rest) = path.split_last()?;
    let mut holder = car;
    for &place in rest {
        holder = holder.slots.get(usize::from(place))?.as_ref()?;
    }
    let slot = holder.def(catalogue).slots.get(usize::from(last))?;
    let holder_kind = (!rest.is_empty()).then(|| holder.def(catalogue).kind);
    Some((slot.kind, holder_kind))
}

/// The part at `path` (the car itself at none).
pub fn item_at<'a>(car: &'a Item, path: &[u8]) -> Option<&'a Item> {
    let mut item = car;
    for &place in path {
        item = item.slots.get(usize::from(place))?.as_ref()?;
    }
    Some(item)
}

fn slot_mut<'a>(car: &'a mut Item, path: &[u8]) -> Option<&'a mut Option<Item>> {
    let (&last, rest) = path.split_last()?;
    let mut holder = car;
    for &place in rest {
        holder = holder.slots.get_mut(usize::from(place))?.as_mut()?;
    }
    holder.slots.get_mut(usize::from(last))
}

pub fn item_mut<'a>(car: &'a mut Item, path: &[u8]) -> Option<&'a mut Item> {
    let mut item = car;
    for &place in path {
        item = item.slots.get_mut(usize::from(place))?.as_mut()?;
    }
    Some(item)
}

/// Whether a shop of `kind` fits parts into a slot of `slot` kind held by a part of
/// `holder` kind (none: the car).
pub fn fits_into(kind: ShopKind, slot: Kind, holder: Option<Kind>) -> bool {
    match kind {
        ShopKind::Car | ShopKind::Tuning => {
            !matches!(
                slot,
                Kind::Rim | Kind::Tire | Kind::LongBlock | Kind::Chassis
            ) && holder != Some(Kind::LongBlock)
        }
        ShopKind::Tire => {
            slot == Kind::Wheel && holder.is_none()
                || matches!(slot, Kind::Rim | Kind::Tire) && holder == Some(Kind::Wheel)
        }
        _ => false,
    }
}

/// Which kind of shop sets the knobs of a part of `kind`.
pub fn knobs_at(kind: Kind, shop: ShopKind) -> bool {
    match kind {
        Kind::Chassis | Kind::Damper | Kind::Tire => shop == ShopKind::Alignment,
        Kind::Engine => shop == ShopKind::Dyno,
        Kind::FinalDrive => matches!(shop, ShopKind::Car | ShopKind::Tuning),
        _ => false,
    }
}

/// The chassis' name the car is (for what parts are made for).
fn chassis_name<'a>(catalogue: &'a Catalogue, car: &Item) -> &'a str {
    match car.def(catalogue).does {
        Does::Chassis(c) => &catalogue.chassis[c].name,
        _ => "",
    }
}

/// Whether `def` is made for the chassis called `chassis` (or any).
pub fn made_for(def: &PartDef, chassis: &str) -> bool {
    def.fits.is_empty() || def.fits.iter().any(|c| c == chassis)
}

/// What a shop sells that goes into a slot of `kind` in a car of `chassis`: part
/// definitions and assemblies, by their places.
pub fn wares(catalogue: &Catalogue, shop: &Shop, kind: Kind, chassis: &str) -> Vec<Supply> {
    let parts = (catalogue.defs.iter().enumerate())
        .filter(|(_, def)| {
            def.kind == kind && !def.absent && made_for(def, chassis) && shop.sells(&def.id)
        })
        .map(|(def, _)| Supply::Part {
            def: def as u16,
            variant: 0,
            paint: 0,
        });
    let assemblies = (catalogue.assemblies.iter().enumerate())
        .filter(|(_, a)| a.item.def(catalogue).kind == kind && shop.sells(&a.id))
        .map(|(a, _)| Supply::Assembly(a as u16));
    parts.chain(assemblies).collect()
}

/// The paints a new part may have on `car`: the paint shops' and the car's own.
pub fn new_paints(catalogue: &Catalogue, car: &Item) -> Vec<usize> {
    let mut paints = catalogue.shop_paints();
    if let Some(own) = car.paint.filter(|p| !paints.contains(p)) {
        paints.insert(0, own);
    }
    paints
}

/// A wheel's rim and tire as the simulation knows them, if it has both.
pub fn wheel_of(catalogue: &Catalogue, wheel: &Item) -> Option<Wheel> {
    let rim = wheel.fitted(catalogue, Kind::Rim).next()?;
    let tire = wheel.fitted(catalogue, Kind::Tire).next()?;
    let Does::Tire(compound) = tire.def(catalogue).does else {
        return None;
    };
    Some(Wheel {
        rim: Rim::all().nth(rim.variant)?,
        tire: TireSize::all().nth(tire.variant)?,
        compound,
    })
}

/// A shop's bench: the car being changed, with the trunk and what the shop holds for the
/// player. Works are done on it one after another; `finish` checks what came of them.
pub struct Bench<'a> {
    catalogue: &'a Shared,
    shop: &'a Shop,
    pub car: Item,
    pub ecu: Ecu,
    pub cargo: Vec<Item>,
    /// What the shop holds for the player.
    pub held: Vec<Item>,
    /// What came off, to go into the trunk or be held.
    off: Vec<Item>,
}

/// What came of a bench's works.
#[derive(Debug)]
pub struct Done {
    pub spec: CarSpec,
    pub held: Vec<Item>,
    /// What was thrown away (no room for it), for the player.
    pub thrown: Vec<String>,
}

impl<'a> Bench<'a> {
    pub fn new(catalogue: &'a Shared, shop: &'a Shop, spec: &CarSpec, held: &[Item]) -> Self {
        Self {
            catalogue,
            shop,
            car: spec.car.clone(),
            ecu: spec.ecu.clone(),
            cargo: spec.cargo.clone(),
            held: held.to_vec(),
            off: Vec::new(),
        }
    }

    /// Does `work`, if the rules allow it.
    pub fn work(&mut self, work: &Work) -> Result<(), String> {
        let kind = self.shop.kind;
        let catalogue: &'a Catalogue = self.catalogue;
        match work {
            Work::Fit { slots, supply } => {
                if slots.is_empty() || slots.len() > MOST_SLOTS {
                    return Err("Fit it where?".into());
                }
                let new = matches!(supply, Supply::Part { .. } | Supply::Assembly(_));
                if slots.len() > 1 && !new {
                    return Err("A part brought goes into one place".into());
                }
                let part = self.take(supply)?;
                for path in slots {
                    self.fit(path, part.clone())?;
                }
            }
            Work::Remove { slot } => {
                let (slot_kind, holder) =
                    slot_kinds(catalogue, &self.car, slot).ok_or("No such place on the car")?;
                if !fits_into(kind, slot_kind, holder) {
                    return Err(format!("A {} doesn't do that", kind.title().to_lowercase()));
                }
                let required = self.slot_def(slot).is_some_and(|slot| slot.required);
                let nothing =
                    (catalogue.defs.iter()).position(|def| def.absent && def.kind == slot_kind);
                let place = slot_mut(&mut self.car, slot).ok_or("No such place on the car")?;
                let old = place.take().ok_or("Nothing's fitted there")?;
                if required {
                    let Some(nothing) = nothing else {
                        *place = Some(old);
                        return Err(match slot_kind {
                            Kind::Engine => "An engine comes out only for another".into(),
                            _ => "The car can't do without it: fit another instead".into(),
                        });
                    };
                    *place = Some(Item::new(catalogue, nothing, 0));
                }
                self.off.push(old);
            }
            Work::Knob { at, knob, value } => {
                let item = item_mut(&mut self.car, at).ok_or("Nothing's fitted there")?;
                let def = &catalogue.defs[item.def];
                if !knobs_at(def.kind, kind) {
                    return Err(format!(
                        "The {} isn't set at a {}",
                        def.name.to_lowercase(),
                        kind.title().to_lowercase()
                    ));
                }
                let range = def.knobs.get(usize::from(*knob)).ok_or("No such setting")?;
                if !(range.low - 1e-3..=range.high + 1e-3).contains(value) {
                    return Err(format!("{} out of range", range.name));
                }
                item.knobs[usize::from(*knob)] = value.clamp(range.low, range.high);
            }
            Work::Paint { at, paint } => {
                if kind != ShopKind::Paint {
                    return Err("Paint is sprayed at paint shops".into());
                }
                let paint = usize::from(*paint);
                if !catalogue.paints.get(paint).is_some_and(|p| p.shop) {
                    return Err("The paint shop hasn't got that paint".into());
                }
                let item = item_mut(&mut self.car, at).ok_or("Nothing's fitted there")?;
                if item.paint.is_none() {
                    return Err("That isn't painted".into());
                }
                item.paint = Some(paint);
            }
            Work::Tune(ecu) => {
                if kind != ShopKind::Dyno {
                    return Err("Tunes are loaded at a dyno".into());
                }
                self.ecu = (**ecu).clone();
            }
            Work::MakeWheel(supply) => {
                if kind != ShopKind::Tire || !matches!(supply, Supply::Wheel(..)) {
                    return Err("Wheels are made at tire shops".into());
                }
                let wheel = self.take(supply)?;
                if !crate::cargo::fits(catalogue, &self.car, &self.cargo, &wheel) {
                    return Err("No room in the trunk for the wheel".into());
                }
                self.cargo.push(wheel);
            }
            Work::Split { item } => {
                if kind != ShopKind::Tire {
                    return Err("Wheels are taken apart at tire shops".into());
                }
                let i = usize::from(*item);
                let wheel = self.cargo.get(i).ok_or("That's gone already")?;
                if wheel.def(catalogue).kind != Kind::Wheel {
                    return Err("That isn't a wheel".into());
                }
                let wheel = self.cargo.remove(i);
                self.off.extend(wheel.slots.into_iter().flatten());
            }
        }
        Ok(())
    }

    fn slot_def(&self, path: &[u8]) -> Option<&crate::items::SlotDef> {
        let (&last, rest) = path.split_last()?;
        let holder = item_at(&self.car, rest)?;
        holder.def(self.catalogue).slots.get(usize::from(last))
    }

    /// The part `supply` says, taken from the trunk or what's held, or made new.
    fn take(&mut self, supply: &Supply) -> Result<Item, String> {
        let catalogue: &'a Catalogue = self.catalogue;
        let chassis = chassis_name(catalogue, &self.car);
        match supply {
            Supply::Part {
                def,
                variant,
                paint,
            } => {
                let d = usize::from(*def);
                let made = catalogue.defs.get(d).ok_or("No such part")?;
                if made.absent || !self.shop.sells(&made.id) {
                    return Err(format!("{} doesn't sell that", self.shop.name));
                }
                if !made_for(made, chassis) || made.kind == Kind::Chassis {
                    return Err(format!("The {} isn't made for the {chassis}", made.name));
                }
                let chassis_place = match self.car.def(catalogue).does {
                    Does::Chassis(c) => Some(c),
                    _ => None,
                };
                let variant = usize::from(*variant);
                if variant >= made.variants(catalogue, chassis_place).max(1) {
                    return Err("No such size".into());
                }
                let paint = usize::from(*paint);
                if made.painted && !new_paints(catalogue, &self.car).contains(&paint) {
                    return Err("Not in that paint".into());
                }
                let mut item = Item::new(catalogue, d, paint);
                item.variant = variant;
                Ok(item)
            }
            Supply::Assembly(a) => {
                let assembly = (catalogue.assemblies.get(usize::from(*a))).ok_or("No such part")?;
                if !self.shop.sells(&assembly.id) {
                    return Err(format!("{} doesn't sell that", self.shop.name));
                }
                Ok(assembly.item.clone())
            }
            Supply::Trunk(i) => {
                let i = usize::from(*i);
                self.cargo.get(i).ok_or("That's gone already")?;
                Ok(self.cargo.remove(i))
            }
            Supply::Held(i) => {
                let i = usize::from(*i);
                self.held.get(i).ok_or("That's gone already")?;
                Ok(self.held.remove(i))
            }
            Supply::Wheel(rim, tire) => {
                if self.shop.kind != ShopKind::Tire {
                    return Err("Wheels are made at tire shops".into());
                }
                let def = (catalogue.defs.iter())
                    .position(|def| def.does == Does::Wheel)
                    .ok_or("No wheels in the catalogue")?;
                let mut wheel = Item::new(catalogue, def, 0);
                let rim = self.take(rim)?;
                let tire = self.take(tire)?;
                let slots = catalogue.defs[def].slots.clone();
                for part in [rim, tire] {
                    let kind = part.def(catalogue).kind;
                    let place = (slots.iter().zip(&mut wheel.slots))
                        .find(|(slot, item)| slot.kind == kind && item.is_none())
                        .map(|(_, item)| item)
                        .ok_or("A wheel is a rim and a tire")?;
                    *place = Some(part);
                }
                let fine = wheel_of(catalogue, &wheel)
                    .ok_or_else(|| "A wheel is a rim and a tire".to_string())?;
                if !fine.tire.fits(&fine.rim) {
                    return Err(format!(
                        "{} tires don't go on {} rims",
                        fine.tire.label(),
                        fine.rim.label()
                    ));
                }
                Ok(wheel)
            }
        }
    }

    /// `part` into the slot at `path`; what was there comes off, but for what goes into
    /// the new part's empty slots (a gearbox's final drive and differential).
    fn fit(&mut self, path: &[u8], mut part: Item) -> Result<(), String> {
        let catalogue: &'a Catalogue = self.catalogue;
        let (slot_kind, holder) =
            slot_kinds(catalogue, &self.car, path).ok_or("No such place on the car")?;
        let kind = self.shop.kind;
        if !fits_into(kind, slot_kind, holder) {
            return Err(match slot_kind {
                Kind::Rim | Kind::Tire => "Rims and tires are fitted at tire shops".into(),
                Kind::LongBlock => "Long blocks are the engine shop's work".into(),
                _ if holder == Some(Kind::LongBlock) => {
                    "What's inside a long block is the engine shop's work".into()
                }
                _ => format!(
                    "A {} doesn't fit that",
                    self.shop.kind.title().to_lowercase()
                ),
            });
        }
        let def = part.def(catalogue);
        if def.kind != slot_kind {
            return Err(format!("The {} doesn't go there", name(catalogue, &part)));
        }
        let chassis = chassis_name(catalogue, &self.car);
        if !made_for(def, chassis) {
            return Err(format!("The {} isn't made for the {chassis}", def.name));
        }
        let place = slot_mut(&mut self.car, path).ok_or("No such place on the car")?;
        if let Some(mut old) = place.take() {
            let slots = &catalogue.defs[part.def].slots;
            let old_slots = &catalogue.defs[old.def].slots;
            for (slot, item) in slots.iter().zip(&mut part.slots) {
                if item.is_some() {
                    continue;
                }
                let moved = (old_slots.iter().zip(&mut old.slots))
                    .find(|(s, i)| s.kind == slot.kind && i.is_some())
                    .and_then(|(_, i)| i.take());
                *item = moved;
            }
            self.off.push(old);
        }
        *place = Some(part);
        Ok(())
    }

    /// Whether the car that came of the works can be driven, and if so the car with what
    /// came off put away.
    pub fn finish(mut self) -> Result<Done, String> {
        let catalogue = self.catalogue;
        if let Some(why) = needs_missing(catalogue, &self.car) {
            return Err(why);
        }
        if !self.car.fine(catalogue) {
            return Err("That doesn't make a car".into());
        }
        let (build, parts) = self.car.resolve(catalogue).map_err(|e| e.0)?;
        // Every wheel, not just the left ones the simulation goes by.
        let chassis = build.chassis();
        for (n, wheel) in self.car.fitted(catalogue, Kind::Wheel).enumerate() {
            let wheel = wheel_of(catalogue, wheel).ok_or("A wheel is a rim and a tire")?;
            if let Some(why) = chassis.room[(n / 2).min(1)].misfit(&wheel) {
                return Err(format!("{}: {why}", crate::items::CORNERS[n.min(3)]));
            }
        }
        // Another ECU runs the same tune as far as it goes.
        self.ecu.model = parts.ecu;
        if self.ecu.cylinders != parts.hardware().cylinders {
            return Err("The tune is for another number of cylinders".into());
        }
        let mut thrown = Vec::new();
        let mut cargo = std::mem::take(&mut self.cargo);
        for part in std::mem::take(&mut self.off) {
            if part.def(catalogue).absent {
                continue;
            }
            if crate::cargo::fits(catalogue, &self.car, &cargo, &part) {
                cargo.push(part);
            } else if self.held.len() < SHOP_HOLD {
                self.held.push(part);
            } else {
                thrown.push(name(catalogue, &part));
            }
        }
        // The trunk may have shrunk (a seat fitted where things lay).
        let fitting = crate::cargo::fitting(catalogue, &self.car, cargo.clone());
        if fitting.len() < cargo.len() {
            return Err("The trunk's load wouldn't fit any more".into());
        }
        let spec = CarSpec {
            car: self.car,
            build,
            parts,
            ecu: self.ecu,
            cargo,
        };
        // Within everything a car read from the wire must be (settings, the tune).
        let bytes = crate::net::wire::encode(&spec);
        crate::net::wire::decode_with::<CarSpec>(&bytes, Some(catalogue))
            .ok_or("Settings out of range")?;
        Ok(Done {
            spec,
            held: self.held,
            thrown,
        })
    }
}

/// Why `car` lacks a part one of its parts needs, if it does.
pub fn needs_missing(catalogue: &Catalogue, car: &Item) -> Option<String> {
    let slots = &car.def(catalogue).slots;
    for (slot, item) in slots.iter().zip(&car.slots) {
        let Some(item) = item else {
            continue;
        };
        if let Some(needed) = slot.needs
            && car.slots.get(needed).is_some_and(Option::is_none)
        {
            return Some(format!(
                "The {} needs a {}",
                slot.name.to_lowercase(),
                slots[needed].name.to_lowercase()
            ));
        }
        if let Some(why) = needs_missing(catalogue, item) {
            return Some(why);
        }
    }
    None
}

/// The works that make `old` (a car's tree and tune) into `new`, as far as works can:
/// knobs and paint of the same parts, and the tune. Parts changed can't be (they're asked
/// for as fits).
pub fn changes(old: &CarSpec, new: &CarSpec) -> Vec<Work> {
    let mut works = Vec::new();
    fn walk(old: &Item, new: &Item, path: &mut Path, works: &mut Vec<Work>) {
        if old.def != new.def || old.variant != new.variant {
            return;
        }
        for (knob, (a, b)) in old.knobs.iter().zip(&new.knobs).enumerate() {
            if a != b {
                works.push(Work::Knob {
                    at: path.clone(),
                    knob: knob as u8,
                    value: *b,
                });
            }
        }
        if old.paint != new.paint
            && let Some(paint) = new.paint
        {
            works.push(Work::Paint {
                at: path.clone(),
                paint: paint as u16,
            });
        }
        for (place, (a, b)) in old.slots.iter().zip(&new.slots).enumerate() {
            if let (Some(a), Some(b)) = (a, b) {
                path.push(place as u8);
                walk(a, b, path, works);
                path.pop();
            }
        }
    }
    walk(&old.car, &new.car, &mut Vec::new(), &mut works);
    if old.ecu != new.ecu {
        works.push(Work::Tune(Box::new(new.ecu.clone())));
    }
    works
}

/// Whether `changes` covers everything `old` and `new` differ in.
pub fn only_settings(old: &CarSpec, new: &CarSpec) -> bool {
    fn same_parts(a: &Item, b: &Item) -> bool {
        a.def == b.def
            && a.variant == b.variant
            && a.slots.len() == b.slots.len()
            && (a.slots.iter().zip(&b.slots)).all(|(a, b)| match (a, b) {
                (Some(a), Some(b)) => same_parts(a, b),
                (None, None) => true,
                _ => false,
            })
    }
    same_parts(&old.car, &new.car) && old.cargo == new.cargo
}

impl Wire for Supply {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Supply::Part {
                def,
                variant,
                paint,
            } => {
                0u8.put(out);
                def.put(out);
                variant.put(out);
                paint.put(out);
            }
            Supply::Assembly(a) => (1u8, *a).put(out),
            Supply::Trunk(i) => (2u8, *i).put(out),
            Supply::Held(i) => (3u8, *i).put(out),
            Supply::Wheel(rim, tire) => {
                4u8.put(out);
                rim.put(out);
                tire.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        read_supply(r, 0)
    }
}

fn read_supply(r: &mut Reader, depth: usize) -> Option<Supply> {
    Some(match r.get::<u8>()? {
        0 => Supply::Part {
            def: r.get()?,
            variant: r.get()?,
            paint: r.get()?,
        },
        1 => Supply::Assembly(r.get()?),
        2 => Supply::Trunk(r.get()?),
        3 => Supply::Held(r.get()?),
        // A wheel of a rim and a tire, no deeper.
        4 if depth == 0 => {
            Supply::Wheel(Box::new(read_supply(r, 1)?), Box::new(read_supply(r, 1)?))
        }
        _ => return None,
    })
}

impl Wire for Work {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Work::Fit { slots, supply } => {
                0u8.put(out);
                slots.put(out);
                supply.put(out);
            }
            Work::Remove { slot } => {
                1u8.put(out);
                slot.put(out);
            }
            Work::Knob { at, knob, value } => {
                2u8.put(out);
                at.put(out);
                knob.put(out);
                value.put(out);
            }
            Work::Paint { at, paint } => {
                3u8.put(out);
                at.put(out);
                paint.put(out);
            }
            Work::Tune(ecu) => {
                4u8.put(out);
                ecu.put(out);
            }
            Work::MakeWheel(supply) => {
                5u8.put(out);
                supply.put(out);
            }
            Work::Split { item } => (6u8, *item).put(out),
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let path = |r: &mut Reader| -> Option<Path> {
            let path: Path = r.get()?;
            (path.len() <= 8).then_some(path)
        };
        Some(match r.get::<u8>()? {
            0 => {
                let n: u16 = r.get()?;
                (usize::from(n) <= MOST_SLOTS).then_some(())?;
                let slots = (0..n).map(|_| path(r)).collect::<Option<_>>()?;
                Work::Fit {
                    slots,
                    supply: r.get()?,
                }
            }
            1 => Work::Remove { slot: path(r)? },
            2 => Work::Knob {
                at: path(r)?,
                knob: r.get()?,
                value: r.get()?,
            },
            3 => Work::Paint {
                at: path(r)?,
                paint: r.get()?,
            },
            4 => Work::Tune(Box::new(r.get()?)),
            5 => Work::MakeWheel(r.get()?),
            6 => Work::Split { item: r.get()? },
            _ => return None,
        })
    }
}

/// A request's works, at most `MOST_WORKS`.
pub fn read_works(r: &mut Reader) -> Option<Vec<Work>> {
    let n: u16 = r.get()?;
    (usize::from(n) <= MOST_WORKS).then_some(())?;
    (0..n).map(|_| r.get()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;
    use crate::catalogue::builtin;
    use crate::items::{Panel, Seat};
    use crate::net::wire::{decode, encode};

    fn shop(kind: ShopKind, sells: &[&str]) -> Shop {
        Shop {
            kind,
            name: "Shop".into(),
            at: Default::default(),
            sells: sells.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn spec(option: usize) -> CarSpec {
        CarSpec::of(&CarSetup::factory(0, option))
    }

    /// The place of the car's first slot of `kind`, and how many there are.
    fn slots_of(catalogue: &Catalogue, car: &Item, kind: Kind) -> Vec<u8> {
        (car.def(catalogue).slots.iter().enumerate())
            .filter(|(_, slot)| slot.kind == kind)
            .map(|(i, _)| i as u8)
            .collect()
    }

    fn def(catalogue: &Catalogue, id: &str) -> u16 {
        catalogue.defs.iter().position(|d| d.id == id).unwrap() as u16
    }

    fn new(catalogue: &Catalogue, id: &str) -> Supply {
        Supply::Part {
            def: def(catalogue, id),
            variant: 0,
            paint: 0,
        }
    }

    fn run(
        catalogue: &Shared,
        shop: &Shop,
        spec: &CarSpec,
        works: &[Work],
    ) -> Result<Done, String> {
        let mut bench = Bench::new(catalogue, shop, spec, &[]);
        for work in works {
            bench.work(work)?;
        }
        bench.finish()
    }

    #[test]
    fn a_workshop_fits_what_it_sells_and_the_old_part_goes_into_the_trunk() {
        let catalogue = builtin();
        let before = spec(0);
        let workshop = shop(ShopKind::Tuning, &["clutch.*", "seat.*"]);
        let clutch = slots_of(&catalogue, &before.car, Kind::Clutch);
        let fit = Work::Fit {
            slots: vec![clutch.clone()],
            supply: new(&catalogue, "clutch.race"),
        };
        let done = run(&catalogue, &workshop, &before, std::slice::from_ref(&fit)).unwrap();
        assert_eq!(done.spec.build.clutch, 2);
        let carried: Vec<&str> = (done.spec.cargo.iter())
            .map(|item| item.def(&catalogue).id.as_str())
            .collect();
        assert_eq!(carried, ["clutch.stock"]);
        // A shop that doesn't sell it, or doesn't fit parts, refuses.
        let other = shop(ShopKind::Car, &["seat.*"]);
        assert!(run(&catalogue, &other, &before, std::slice::from_ref(&fit)).is_err());
        let paint = shop(ShopKind::Paint, &["clutch.*"]);
        assert!(run(&catalogue, &paint, &before, &[fit]).is_err());
    }

    #[test]
    fn a_gearbox_keeps_its_final_drive_and_differential() {
        let catalogue = builtin();
        let before = spec(1);
        let workshop = shop(ShopKind::Tuning, &["gearbox.*"]);
        let gearbox = slots_of(&catalogue, &before.car, Kind::Gearbox);
        let done = run(
            &catalogue,
            &workshop,
            &before,
            &[Work::Fit {
                slots: vec![gearbox],
                supply: new(&catalogue, "gearbox.6-speed-close-ratio-transverse"),
            }],
        )
        .unwrap();
        assert_eq!(done.spec.build.gearbox, 1);
        assert!(done.spec.build.limited_slip);
        assert_eq!(done.spec.build.final_drive, before.build.final_drive);
    }

    #[test]
    fn long_blocks_rims_and_tires_are_other_shops_work() {
        let catalogue = builtin();
        let before = spec(0);
        let workshop = shop(ShopKind::Car, &["*"]);
        let engine = slots_of(&catalogue, &before.car, Kind::Engine)[0];
        let wheel = slots_of(&catalogue, &before.car, Kind::Wheel)[0];
        let pistons = Work::Fit {
            slots: vec![vec![engine, 0, 0]],
            supply: new(&catalogue, "pistons.race"),
        };
        let err = run(&catalogue, &workshop, &before, &[pistons]).unwrap_err();
        assert!(err.contains("engine shop"), "{err}");
        let tire = Work::Fit {
            slots: vec![vec![wheel, 1]],
            supply: new(&catalogue, "tire.sport"),
        };
        assert!(run(&catalogue, &workshop, &before, &[tire]).is_err());
        // The engine itself, whole, is fine; it can't come out for nothing.
        let swap = Work::Fit {
            slots: vec![vec![engine]],
            supply: Supply::Assembly(1),
        };
        let done = run(&catalogue, &workshop, &before, &[swap]).unwrap();
        assert_eq!(done.spec.parts.hardware().name, "B18 1.8 four");
        let out = Work::Remove { slot: vec![engine] };
        assert!(run(&catalogue, &workshop, &before, &[out]).is_err());
    }

    #[test]
    fn a_tire_shop_puts_new_tires_on_all_four() {
        let catalogue = builtin();
        let before = spec(0);
        let tires = shop(ShopKind::Tire, &["tire.*", "rim.*"]);
        let wheels = slots_of(&catalogue, &before.car, Kind::Wheel);
        let size = TireSize::all()
            .position(|t| t == before.build.wheels[0].tire)
            .unwrap() as u16;
        let fit = Work::Fit {
            slots: wheels.iter().map(|&w| vec![w, 1]).collect(),
            supply: Supply::Part {
                def: def(&catalogue, "tire.sport"),
                variant: size,
                paint: 0,
            },
        };
        // The old tires are worn: they keep it, the new ones are new.
        let mut worn = before.clone();
        let mut condition = crate::saved::Condition::of_parts(&catalogue, &worn.car, 40.0, 95.0);
        condition.treads = [[0.3; crate::physics::tire::RIBS]; 4];
        condition.put_on(&catalogue, &mut worn.car);
        let done = run(&catalogue, &tires, &worn, &[fit]).unwrap();
        let now = crate::saved::Condition::of_parts(&catalogue, &done.spec.car, 40.0, 95.0);
        assert_eq!(now.treads, [[0.0; crate::physics::tire::RIBS]; 4]);
        let off: Vec<&Item> = done.spec.cargo.iter().chain(&done.held).collect();
        assert!(
            off.iter()
                .all(|tire| tire.wear == [0.3; crate::physics::tire::RIBS])
        );
        assert_eq!(done.spec.build.wheels[1].compound, 2);
        // The old tires: as many as the trunk takes, the rest held.
        assert_eq!(done.spec.cargo.len() + done.held.len(), 4);
        // A tire for another rim diameter doesn't go on the rim.
        let other = TireSize::all()
            .position(|t| t.rim != before.build.wheels[0].rim.diameter)
            .unwrap() as u16;
        let wrong = Work::Fit {
            slots: vec![vec![wheels[0], 1]],
            supply: Supply::Part {
                def: def(&catalogue, "tire.sport"),
                variant: other,
                paint: 0,
            },
        };
        assert!(run(&catalogue, &tires, &before, &[wrong]).is_err());
    }

    #[test]
    fn body_parts_need_what_they_go_on() {
        let catalogue = builtin();
        let before = spec(1);
        let workshop = shop(ShopKind::Tuning, &["vireo.*"]);
        let bumper = slots_of(&catalogue, &before.car, Kind::Body(Panel::FrontBumper));
        let lip = slots_of(&catalogue, &before.car, Kind::Body(Panel::FrontLip));
        let hatch = slots_of(&catalogue, &before.car, Kind::Body(Panel::Hatch));
        let fit_lip = Work::Fit {
            slots: vec![lip.clone()],
            supply: new(&catalogue, "vireo.front-lip"),
        };
        assert!(
            run(
                &catalogue,
                &workshop,
                &before,
                std::slice::from_ref(&fit_lip)
            )
            .is_ok()
        );
        let off = Work::Remove { slot: bumper };
        let err = run(&catalogue, &workshop, &before, &[fit_lip, off]).unwrap_err();
        assert!(err.contains("front bumper"), "{err}");
        // The hatch can't come off under the 1.8's spoiler.
        assert!(
            run(
                &catalogue,
                &workshop,
                &before,
                &[Work::Remove { slot: hatch }]
            )
            .is_err()
        );
        // The rear bench can, and a seat can't go where a door goes.
        let bench = slots_of(&catalogue, &before.car, Kind::Seat(Seat::RearBench));
        let done = run(
            &catalogue,
            &workshop,
            &before,
            &[Work::Remove {
                slot: bench.clone(),
            }],
        )
        .unwrap();
        assert!(done.spec.car.slots[usize::from(bench[0])].is_none());
    }

    #[test]
    fn knobs_are_set_where_they_belong() {
        let catalogue = builtin();
        let before = spec(0);
        let damper = slots_of(&catalogue, &before.car, Kind::Damper)[0];
        let camber = Work::Knob {
            at: vec![damper],
            knob: 0,
            value: -1.5,
        };
        let aligner = shop(ShopKind::Alignment, &[]);
        let done = run(&catalogue, &aligner, &before, std::slice::from_ref(&camber)).unwrap();
        assert_eq!(done.spec.build.camber[0], -1.5);
        let dyno = shop(ShopKind::Dyno, &[]);
        assert!(run(&catalogue, &dyno, &before, &[camber]).is_err());
        let far = Work::Knob {
            at: vec![damper],
            knob: 0,
            value: -40.0,
        };
        assert!(run(&catalogue, &aligner, &before, &[far]).is_err());
    }

    #[test]
    fn settings_changed_in_the_game_become_works() {
        let catalogue = builtin();
        let before = spec(0);
        let setup = CarSetup::factory(0, 0);
        let mut build = setup.build.clone();
        build.camber = [-1.3, -0.8];
        build.paint = catalogue.shop_paints()[2];
        let after = CarSpec::of(&setup.changed(
            build,
            setup.engine.parts.clone(),
            setup.engine.build.clone(),
        ));
        assert!(only_settings(&before, &after));
        let works = changes(&before, &after);
        let aligner = shop(ShopKind::Alignment, &[]);
        let knobs: Vec<Work> = (works.iter())
            .filter(|w| matches!(w, Work::Knob { .. }))
            .cloned()
            .collect();
        let paints: Vec<Work> = (works.iter())
            .filter(|w| matches!(w, Work::Paint { .. }))
            .cloned()
            .collect();
        assert!(knobs.len() >= 2 && paints.len() > 10);
        let done = run(&catalogue, &aligner, &before, &knobs).unwrap();
        let painter = shop(ShopKind::Paint, &[]);
        let done = run(&catalogue, &painter, &done.spec, &paints).unwrap();
        assert_eq!(done.spec.car, after.car);
        // Works go over the wire.
        for work in works {
            assert_eq!(decode::<Work>(&encode(&work)), Some(work));
        }
    }

    #[test]
    fn a_wheel_is_made_and_taken_apart() {
        let catalogue = builtin();
        let before = spec(0);
        let tires = shop(ShopKind::Tire, &["tire.*", "rim.*"]);
        let rim = Rim::all()
            .position(|r| r == before.build.wheels[0].rim)
            .unwrap() as u16;
        let size = TireSize::all()
            .position(|t| t == before.build.wheels[0].tire)
            .unwrap() as u16;
        let make = Work::MakeWheel(Supply::Wheel(
            Box::new(Supply::Part {
                def: def(&catalogue, "rim.alloy"),
                variant: rim,
                paint: 0,
            }),
            Box::new(Supply::Part {
                def: def(&catalogue, "tire.race"),
                variant: size,
                paint: 0,
            }),
        ));
        let done = run(&catalogue, &tires, &before, std::slice::from_ref(&make)).unwrap();
        assert_eq!(done.spec.cargo.len(), 1);
        assert_eq!(decode::<Work>(&encode(&make)), Some(make));
        let split = run(&catalogue, &tires, &done.spec, &[Work::Split { item: 0 }]).unwrap();
        let ids: Vec<&str> = (split.spec.cargo.iter().chain(&split.held))
            .map(|item| item.def(&catalogue).id.as_str())
            .collect();
        assert_eq!(ids.len(), 2, "{ids:?}");
        assert!(ids.contains(&"rim.alloy") && ids.contains(&"tire.race"));
    }
}
