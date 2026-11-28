//! A car as the server keeps it between sessions: text, readable whatever the catalogue
//! has become. What it's built of goes by the definitions' string ids, slot and knob names,
//! paint names and sizes' labels (a definition gone takes its item with it, a knob gone
//! is dropped, a new one starts where it starts), each part's wear (the engine's on the
//! parts it's in, the tires' on the tires: see `Condition::put_on`), then the ECU's tune
//! (all of it). The fuel is the car's own.
//!
//! ```text
//! # car-sim car
//! version 4
//! fuel 31.5 96.2
//! part 0 -  vireo  putty  0  Brake force=1 …  (tab-separated: depth, slot, id, paint,
//! part 1 Front bumper  vireo.bumper.front …    variant, knobs as name=value, and its
//! part 3 Pistons  pistons.stock  -  0  wear=0.1,0,0.02   wear if it has any)
//! cargo                                      (then the loose parts it carries, each a
//! part 0 -  silencer.stock  -  0              tree from depth 0)
//! tune
//! # car-sim ECU tune …
//! ```
//!
//! Before version 3 the wear was the car's: `tread CORNER RIB…`, `engine GASKET BEARINGS
//! TURBO` and `cylinder I PISTON VALVES ROD` lines, read onto the parts.
//!
//! Before version 4 an engine had one exhaust manifold part for every family
//! (`exhaust-manifold.stock`, `short-headers`, `long-tube-headers`: lengths relative to
//! the block's headers) and a "Turbo or supercharger" slot. They're read by their ids (in
//! loose parts too, which carry no version): an old manifold becomes its family's without
//! turbo mounts whose primaries are nearest what the old part made of them; a turbo in the
//! old slot goes on every mount of its family's turbo manifold (nearest the manifold it
//! had, which it replaces if that has no mounts); superchargers go into the supercharger
//! slot.

use crate::car::Car;
use crate::catalogue::{Catalogue, Shared};
use crate::items::{Does, Item, Kind, Wear};
use crate::net::wire::{decode_with, encode};
use crate::physics::drivetrain::{FRONT_SHAFTS, MAX_GEARS, REAR_SHAFTS, SHAFTS};
use crate::physics::engine::damage::{Damage, MAX_CYLINDERS};
use crate::physics::engine::ecu::Ecu;
use crate::physics::fuel::Fuel;
use crate::physics::tire::RIBS;
use crate::spec::CarSpec;
use crate::wheels::{Rim, TireSize};

/// The format's version.
const VERSION: u32 = 5;

/// Before version 4: the exhaust manifolds' ids' start, each one's primaries relative to
/// the block's headers, and the slot turbos and superchargers shared.
const OLD_MANIFOLD: &str = "exhaust-manifold.";
const OLD_HEADERS: [(&str, f32); 3] = [
    ("stock", 1.0),
    ("short-headers", 0.56),
    ("long-tube-headers", 1.375),
];
const OLD_TURBO_SLOT: &str = "Turbo or supercharger";
/// Porting was the long block's "Cylinder head" slot before the heads were parts of their
/// own (version 5): its parts' ids then.
const OLD_PORTING: &str = "cylinder-head.";
/// Parts renamed since (version 5: the B18 became the B20).
const RENAMED: [(&str, &str); 3] = [
    ("b18", "b20"),
    ("b18.head", "b20.head"),
    ("ecu.b18", "ecu.b20"),
];

/// Each part's wear is a field `wear=…` after its knobs.
const WEAR: &str = "wear";

/// Where the loose parts carried begin (version 2 on).
const CARGO: &str = "cargo";

/// Where the tune begins: everything after this line.
const TUNE: &str = "tune";

/// How worn a car is and what's in its tank: what a car built anew from what it's built
/// of doesn't know.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub fuel: f32,
    pub octane: f32,
    /// Each corner's ribs' wear (0 new, 1 worn through).
    pub treads: [[f32; RIBS]; 4],
    pub damage: Damage,
    /// Each corner's brake pads' wear (0 new, 1 worn through).
    pub pads: [f32; 4],
    /// The clutch's wear, and each forward gear's (0 new, 1 worn out).
    pub clutch: f32,
    pub gears: [f32; MAX_GEARS],
    /// The front drive shafts', the rear ones' and the prop shaft's wear (0 new, 1 broken).
    pub shafts: [f32; SHAFTS],
}

impl Condition {
    pub fn of(car: &Car) -> Self {
        Self {
            fuel: car.fuel.litres(),
            octane: car.fuel.octane(),
            treads: car.corners.each_ref().map(|corner| corner.tread.wear),
            damage: car.drivetrain.engine.damage,
            pads: car.corners.each_ref().map(|corner| corner.brake.wear),
            clutch: car.drivetrain.clutch_wear,
            gears: car.drivetrain.gear_wear,
            shafts: car.drivetrain.shaft_wear,
        }
    }

    /// Gives `car` this condition (as much fuel as fits its tank).
    pub fn apply(&self, car: &mut Car) {
        car.fuel = Fuel::with(self.fuel, self.octane);
        car.fuel.fit(&car.setup.tank);
        car.load_fuel();
        for (corner, wear) in car.corners.iter_mut().zip(self.treads) {
            corner.tread.wear = wear;
        }
        for (corner, wear) in car.corners.iter_mut().zip(self.pads) {
            corner.brake.wear = wear;
        }
        car.drivetrain.engine.damage = self.damage;
        car.drivetrain.clutch_wear = self.clutch;
        car.drivetrain.gear_wear = self.gears;
        car.drivetrain.shaft_wear = self.shafts;
    }

    /// Puts the wear on the parts of `car` (a car's tree) that carry it (`Item::wear`): the
    /// engine's on the parts of it each kind of wear is in (pistons, rods, the head's
    /// valves, the head gasket, the crank's bearings, the turbo), each tire's tread on the
    /// tire at its corner, each shaft's on that shaft (`items::shafts`).
    pub fn put_on(&self, catalogue: &Catalogue, car: &mut Item) {
        let damage = &self.damage;
        each_mut(car, &mut |item| {
            let Some(wear) = item.def(catalogue).wear else {
                return;
            };
            let per = |of: fn(&crate::physics::engine::damage::CylinderWear) -> f32| {
                damage.cylinders.iter().map(of).collect()
            };
            let values: Vec<f32> = match wear {
                Wear::Pistons => per(|c| c.piston),
                Wear::Rods => per(|c| c.rod),
                Wear::Valves => per(|c| c.valves),
                Wear::HeadGasket => vec![damage.head_gasket],
                Wear::Bearings => vec![damage.bearings],
                Wear::Turbo => vec![damage.turbo],
                Wear::Clutch => vec![self.clutch],
                Wear::Gears => self.gears.to_vec(),
                // By the corner or the shaft's place (below).
                Wear::Tread | Wear::Pads | Wear::Shaft => return,
            };
            item.wear = trimmed(values);
        });
        for (corner, tire) in tires_mut(catalogue, car).into_iter().enumerate() {
            if let Some(tread) = self.treads.get(corner) {
                tire.wear = trimmed(tread.to_vec());
            }
        }
        for (corner, brake) in brakes_mut(catalogue, car).into_iter().enumerate() {
            if let Some(&wear) = self.pads.get(corner) {
                brake.wear = trimmed(vec![wear]);
            }
        }
        for (shaft, wear) in self.shafts.iter().enumerate() {
            if let Some(item) = shaft_mut(catalogue, car, shaft) {
                item.wear = trimmed(vec![*wear]);
            }
        }
    }

    /// The condition of a car whose parts carry `car`'s wear (see `put_on`), with `fuel`
    /// litres of `octane` in the tank.
    pub fn of_parts(catalogue: &Catalogue, car: &Item, fuel: f32, octane: f32) -> Self {
        let mut damage = Damage::default();
        let (mut clutch, mut gears) = (0.0, [0.0; MAX_GEARS]);
        for item in car.all() {
            let Some(wear) = item.def(catalogue).wear else {
                continue;
            };
            let at = |i: usize| item.wear.get(i).copied().unwrap_or(0.0);
            for (i, cylinder) in damage.cylinders.iter_mut().enumerate() {
                match wear {
                    Wear::Pistons => cylinder.piston = at(i),
                    Wear::Rods => cylinder.rod = at(i),
                    Wear::Valves => cylinder.valves = at(i),
                    _ => {}
                }
            }
            match wear {
                Wear::HeadGasket => damage.head_gasket = at(0),
                Wear::Bearings => damage.bearings = at(0),
                // Any turbo failed fails the boost.
                Wear::Turbo => damage.turbo = damage.turbo.max(at(0)),
                Wear::Clutch => clutch = at(0),
                Wear::Gears => gears = std::array::from_fn(at),
                _ => {}
            }
        }
        let mut treads = [[0.0; RIBS]; 4];
        let mut car = car.clone();
        for (corner, tire) in tires_mut(catalogue, &mut car).into_iter().enumerate() {
            if let Some(tread) = treads.get_mut(corner) {
                for (rib, wear) in tread.iter_mut().enumerate() {
                    *wear = tire.wear.get(rib).copied().unwrap_or(0.0);
                }
            }
        }
        let mut pads = [0.0; 4];
        for (corner, brake) in brakes_mut(catalogue, &mut car).into_iter().enumerate() {
            if let Some(wear) = pads.get_mut(corner) {
                *wear = brake.wear.first().copied().unwrap_or(0.0);
            }
        }
        let shafts = crate::items::shafts(catalogue, &car).map(|shaft| {
            shaft
                .and_then(|item| item.wear.first().copied())
                .unwrap_or(0.0)
        });
        Self {
            fuel,
            octane,
            treads,
            damage,
            pads,
            clutch,
            gears,
            shafts,
        }
    }
}

/// The shaft at `shaft`'s place (`drivetrain::SHAFTS`) in a car's tree: see
/// `items::shafts`.
fn shaft_mut<'a>(catalogue: &Catalogue, car: &'a mut Item, shaft: usize) -> Option<&'a mut Item> {
    let slot_of = |item: &Item, kind: Kind| {
        (catalogue.defs[item.def].slots.iter())
            .zip(&item.slots)
            .position(|(slot, fitted)| slot.kind == kind && fitted.is_some())
    };
    match shaft {
        FRONT_SHAFTS => {
            let at = slot_of(car, Kind::DriveShafts)?;
            car.slots[at].as_mut()
        }
        REAR_SHAFTS => {
            let at = slot_of(car, Kind::RearSubframe)?;
            let subframe = car.slots[at].as_mut()?;
            let at = slot_of(subframe, Kind::DriveShafts)?;
            subframe.slots[at].as_mut()
        }
        _ => {
            let at = slot_of(car, Kind::PropShaft)?;
            car.slots[at].as_mut()
        }
    }
}

/// The brakes at a car's corners, in the order of its brake slots.
fn brakes_mut<'a>(catalogue: &Catalogue, car: &'a mut Item) -> Vec<&'a mut Item> {
    let slots = &catalogue.defs[car.def].slots;
    (slots.iter().zip(&mut car.slots))
        .filter(|(slot, _)| slot.kind == Kind::Brake)
        .filter_map(|(_, brake)| brake.as_mut())
        .collect()
}

/// `f` on every item of the tree.
fn each_mut(item: &mut Item, f: &mut impl FnMut(&mut Item)) {
    f(item);
    for child in item.slots.iter_mut().flatten() {
        each_mut(child, f);
    }
}

/// The tires on a car's wheels, in the order of its wheel slots (its corners).
fn tires_mut<'a>(catalogue: &Catalogue, car: &'a mut Item) -> Vec<&'a mut Item> {
    let slots = &catalogue.defs[car.def].slots;
    let mut tires = Vec::new();
    for (slot, wheel) in slots.iter().zip(&mut car.slots) {
        if slot.kind != Kind::Wheel {
            continue;
        }
        let Some(wheel) = wheel else {
            continue;
        };
        let inner = &catalogue.defs[wheel.def].slots;
        let tire = (inner.iter().zip(&mut wheel.slots))
            .find(|(slot, _)| slot.kind == Kind::Tire)
            .and_then(|(_, tire)| tire.as_mut());
        if let Some(tire) = tire {
            tires.push(tire);
        }
    }
    tires
}

/// Wear values without the zeros at the end (none: new), each within 0..1.
pub fn trimmed(mut values: Vec<f32>) -> Vec<f32> {
    for value in &mut values {
        *value = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    values.truncate(crate::items::MOST_WEAR);
    while values.last() == Some(&0.0) {
        values.pop();
    }
    values
}

/// `spec` as built and in `condition`, as text.
pub fn write(catalogue: &Catalogue, spec: &CarSpec, condition: &Condition) -> String {
    let mut text = format!("# car-sim car\nversion {VERSION}\n");
    text += &format!("fuel {} {}\n", condition.fuel, condition.octane);
    let mut car = spec.car.clone();
    condition.put_on(catalogue, &mut car);
    write_item(catalogue, &car, 0, "-", &mut text);
    text += CARGO;
    text += "\n";
    for item in &spec.cargo {
        write_item(catalogue, item, 0, "-", &mut text);
    }
    text += TUNE;
    text += "\n";
    text += &spec.ecu.to_text_whole(&catalogue.defs[spec.car.def].name);
    text
}

/// Loose parts (each a tree, with what's fitted to it) as text, by the same rules as a
/// car's parts.
pub fn write_items(catalogue: &Catalogue, items: &[Item]) -> String {
    let mut text = String::new();
    for item in items {
        write_item(catalogue, item, 0, "-", &mut text);
    }
    text
}

/// Loose parts written by `write_items`, as `catalogue` has them now: parts gone from it
/// are left out, and so is anything that's no longer a whole part (`Item::fine`).
pub fn read_items(catalogue: &Catalogue, text: &str) -> Vec<Item> {
    let mut parts = Parts::default();
    for line in text.lines() {
        if let Some(rest) = line.trim_end().strip_prefix("part\t") {
            // A broken line is skipped.
            let _ = parts.line(catalogue, rest);
        }
    }
    let (mut items, orphans) = parts.finish(catalogue);
    items.extend(orphans);
    let fine = |item: &Item| {
        item.def(catalogue).kind != crate::items::Kind::Chassis && item.fine(catalogue)
    };
    items.into_iter().filter(fine).collect()
}

/// Trees of parts as they're read, a line at a time.
#[derive(Default)]
struct Parts {
    /// Each item not yet put into its parent's slot, with its depth and the slot's name.
    stack: Vec<(usize, String, Item)>,
    /// The trees read whole.
    roots: Vec<Item>,
    /// Below an item that's gone: lines deeper than this are its parts, gone with it.
    skipping: Option<usize>,
    /// The engine family and version of the car's long block, for ECUs saved before there
    /// was one of each (`old_ecu`).
    engine: Option<(usize, usize)>,
    /// Differentials saved in a gearbox that has no slot for one any more (an inline
    /// gearbox's, before protocol 70: it's at the rear axle now, in the rear subframe).
    orphans: Vec<Item>,
}

impl Parts {
    /// A `part` line (after the word).
    fn line(&mut self, catalogue: &Catalogue, rest: &str) -> Result<(), String> {
        let fields: Vec<&str> = rest.split('\t').collect();
        let [depth, slot, id, paint, variant, knobs @ ..] = fields.as_slice() else {
            return Err(format!("a broken line: part\t{rest}"));
        };
        let depth: usize = depth
            .parse()
            .map_err(|_| format!("a broken line: part\t{rest}"))?;
        if self.skipping.is_some_and(|gone| depth > gone) {
            return Ok(());
        }
        self.skipping = None;
        self.settle(catalogue, depth);
        // Old porting keeps its valves' wear until it's in its head (`into_old_head`).
        let old_porting = id.starts_with(OLD_PORTING);
        let id = match id.strip_prefix(OLD_PORTING) {
            Some(part) => format!("porting.{part}"),
            None => (RENAMED.iter())
                .find(|(old, _)| old == id)
                .map_or(id, |(_, new)| new)
                .to_string(),
        };
        let id = old_ecu(catalogue, &id, self.engine).unwrap_or(id);
        let id = id.as_str();
        let found = catalogue.defs.iter().position(|def| def.id == *id);
        let Some(def) = found.or_else(|| self.old_manifold(catalogue, id)) else {
            bevy::log::warn!("A saved part {id} isn't in the catalogue any more");
            self.skipping = Some(depth);
            return Ok(());
        };
        let made = &catalogue.defs[def];
        let mut item = Item::new(catalogue, def, 0);
        item.paint = made.painted.then(|| {
            (catalogue.paints.iter().position(|p| p.name == *paint))
                .unwrap_or_else(|| catalogue.chassis.first().map_or(0, |c| c.paint))
        });
        item.variant = variant_of(made, variant).unwrap_or(0);
        for knob in knobs {
            let Some((name, value)) = knob.split_once('=') else {
                continue;
            };
            if name == WEAR && made.knobs.iter().all(|k| k.name != WEAR) {
                let values = value.split(',').map(|w| w.parse().unwrap_or(0.0)).collect();
                item.wear = if made.wear.is_some() || old_porting {
                    trimmed(values)
                } else {
                    Vec::new()
                };
                continue;
            }
            let place = made.knobs.iter().position(|k| k.name == name);
            if let (Some(place), Ok(value)) = (place, value.parse::<f32>()) {
                item.knobs[place] = value;
            }
        }
        self.stack.push((depth, slot.to_string(), item));
        Ok(())
    }

    /// Puts every item on the stack at `depth` or deeper into its parent's slot (by the
    /// slot's name: the first free one called so, else of its kind), a tree's root among the
    /// roots. An item without a slot for it is dropped.
    fn settle(&mut self, catalogue: &Catalogue, depth: usize) {
        while self.stack.last().is_some_and(|(d, ..)| *d >= depth) {
            let (_, slot, item) = self.stack.pop().expect("an item");
            let Some((_, _, parent)) = self.stack.last_mut() else {
                self.roots.push(item);
                continue;
            };
            let slots = &catalogue.defs[parent.def].slots;
            // A slot renamed since: the first free one of the part's kind.
            let kind = catalogue.defs[item.def].kind;
            let named = slots.iter().any(|s| s.name == slot && s.kind == kind);
            let free = (slots.iter().zip(&parent.slots)).position(|(s, fitted)| {
                fitted.is_none()
                    && if named {
                        s.name == slot
                    } else {
                        s.kind == kind
                    }
            });
            match free {
                Some(place) => parent.slots[place] = Some(item),
                None if slot == OLD_TURBO_SLOT && mount_old_turbo(catalogue, parent, &item) => {}
                None if into_old_head(catalogue, parent, &item) => {}
                None if catalogue.defs[item.def].kind == Kind::Differential => {
                    self.orphans.push(item);
                }
                None => bevy::log::warn!(
                    "A saved part {} has no slot \"{slot}\" any more",
                    catalogue.defs[item.def].id
                ),
            }
        }
    }

    /// The definition an exhaust manifold saved before version 4 (`OLD_MANIFOLD`) is now:
    /// the manifold without turbo mounts made for the engine it's on (the item on top of
    /// the stack; loose: the first family's first version) whose primaries are nearest what
    /// the old part made of the block's.
    fn old_manifold(&self, catalogue: &Catalogue, id: &str) -> Option<usize> {
        let (_, times) = OLD_HEADERS
            .iter()
            .find(|(old, _)| id.strip_prefix(OLD_MANIFOLD) == Some(old))?;
        let engine = self.stack.last().map(|(_, _, item)| item);
        let block = engine
            .filter(|engine| engine.def(catalogue).kind == Kind::Engine)
            .and_then(|engine| engine.fitted(catalogue, Kind::LongBlock).next())
            .map(|block| &block.def(catalogue).does);
        let (family, version) = match block {
            Some(&Does::Block { family, version }) => (family, version),
            _ => (0, 0),
        };
        let made = catalogue.families.get(family)?;
        let headers = made
            .versions
            .get(version)?
            .block
            .exhaust
            .manifold
            .primary_length;
        let wanted = crate::physics::engine::parts::round_to(headers * times, 0.01);
        nearest_manifold(catalogue, family, wanted, false)
    }

    /// The trees read, and the differentials that had no slot any more (`orphans`).
    fn finish(mut self, catalogue: &Catalogue) -> (Vec<Item>, Vec<Item>) {
        self.settle(catalogue, 0);
        (self.roots, self.orphans)
    }
}

/// The definition of `family`'s manifold with turbo mounts (or without) whose primaries are
/// nearest `length` (m; the first of those as near).
fn nearest_manifold(
    catalogue: &Catalogue,
    family: usize,
    length: f32,
    mounts: bool,
) -> Option<usize> {
    let made = catalogue.families.get(family)?;
    let off = |m: usize| (made.manifolds[m].manifold.primary_length - length).abs();
    let manifold = (0..made.manifolds.len())
        .filter(|&m| made.manifolds[m].mounts == mounts)
        .min_by(|&a, &b| off(a).total_cmp(&off(b)))?;
    let does = Does::Manifold { family, manifold };
    catalogue.defs.iter().position(|def| def.does == does)
}

/// A turbo saved in an engine's old turbo slot (before version 4) onto `engine`: on every
/// mount of its manifold, which becomes the family's turbo manifold nearest it if it has
/// no mounts; the engine's empty slots that must have a part get the one standing for
/// nothing (no supercharger). Whether it went on.
fn mount_old_turbo(catalogue: &Catalogue, engine: &mut Item, turbo: &Item) -> bool {
    if turbo.def(catalogue).kind != Kind::Turbo || engine.def(catalogue).kind != Kind::Engine {
        return false;
    }
    let slots = catalogue.defs[engine.def].slots.clone();
    let Some(manifold) = (slots.iter().zip(&mut engine.slots))
        .find(|(slot, _)| slot.kind == Kind::Manifold)
        .and_then(|(_, manifold)| manifold.as_mut())
    else {
        return false;
    };
    let Does::Manifold {
        family,
        manifold: m,
    } = manifold.def(catalogue).does
    else {
        return false;
    };
    let mounted = |item: &Item| (item.def(catalogue).slots.iter()).any(|s| s.kind == Kind::Turbo);
    if !mounted(manifold) {
        let length = catalogue.families[family].manifolds[m]
            .manifold
            .primary_length;
        let Some(def) = nearest_manifold(catalogue, family, length, true) else {
            return false;
        };
        *manifold = Item::new(catalogue, def, 0);
    }
    let mounts = catalogue.defs[manifold.def].slots.clone();
    for (slot, place) in mounts.iter().zip(&mut manifold.slots) {
        if slot.kind == Kind::Turbo {
            *place = Some(turbo.clone());
        }
    }
    for (slot, place) in slots.iter().zip(&mut engine.slots) {
        let nothing = (catalogue.defs.iter()).position(|def| def.absent && def.kind == slot.kind);
        if let (true, None, Some(nothing)) = (slot.required, &place, nothing) {
            *place = Some(Item::new(catalogue, nothing, 0));
        }
    }
    true
}

/// Every engine part slot of `item`'s tree that must be filled and isn't (one added since
/// the car was saved, like the lambda sensor's in version 5) filled with its first part.
fn fill_new_slots(catalogue: &Catalogue, item: &mut Item) {
    let slots = catalogue.defs[item.def].slots.clone();
    for (slot, place) in slots.iter().zip(&mut item.slots) {
        match (place.as_mut(), slot.kind) {
            (Some(child), _) => fill_new_slots(catalogue, child),
            (None, Kind::EnginePart(s)) if slot.required => {
                let does = Does::EnginePart { slot: s, part: 0 };
                if let Some(def) = catalogue.defs.iter().position(|def| def.does == does) {
                    *place = Some(Item::new(catalogue, def, 0));
                }
            }
            _ => {}
        }
    }
}

/// A car saved before the rear driveline was made of parts (protocol 70) given its
/// chassis' factory rear subframe (and with it, if it has a differential mount, the
/// factory's prop shaft): the differential its inline gearbox had (one of `orphans`) goes
/// into the mount, so a limited slip stays one.
fn fill_rear(catalogue: &Catalogue, car: &mut Item, orphans: &mut Vec<Item>) {
    let Does::Chassis(chassis) = car.def(catalogue).does else {
        return;
    };
    let slots = catalogue.defs[car.def].slots.clone();
    let Some(place) = (slots.iter()).position(|slot| slot.kind == Kind::RearSubframe) else {
        return;
    };
    if car.slots[place].is_some() {
        return;
    }
    let differential = (!orphans.is_empty()).then(|| orphans.remove(0));
    let Some((subframe, prop)) = crate::items::factory_rear(catalogue, chassis, differential)
    else {
        return;
    };
    let mounted = subframe
        .fitted(catalogue, Kind::Differential)
        .next()
        .is_some();
    car.slots[place] = Some(subframe);
    if mounted {
        for (slot, item) in slots.iter().zip(&mut car.slots) {
            if slot.kind == Kind::PropShaft && item.is_none() {
                *item = prop.clone();
            }
        }
    }
}

/// A car saved before front drive shafts were parts (protocol 71) given its chassis'
/// stock ones (`<chassis>.drive-shafts`), new.
fn fill_front_shafts(catalogue: &Catalogue, car: &mut Item) {
    let slots = catalogue.defs[car.def].slots.clone();
    let id = format!("{}.drive-shafts", catalogue.defs[car.def].id);
    for (slot, place) in slots.iter().zip(&mut car.slots) {
        if slot.kind == Kind::DriveShafts
            && place.is_none()
            && let Some(def) = crate::items::by_id(catalogue, &id)
        {
            *place = Some(Item::new(catalogue, def, 0));
        }
    }
}

/// What an ECU saved before version 5 (`ecu.stock`, `ecu.programmable`) is now, on a car
/// whose engine is `engine`'s family and version (the first's if not known): the version's
/// own stock ECU, or the programmable one for its cylinders.
fn old_ecu(catalogue: &Catalogue, id: &str, engine: Option<(usize, usize)>) -> Option<String> {
    let (family, version) = engine.unwrap_or((0, 0));
    let version = catalogue.families.get(family)?.versions.get(version)?;
    let id = match id {
        "ecu.stock" => &catalogue.ecus.get(version.preset.ecu)?.id,
        "ecu.programmable" => {
            let cylinders = version.block.cylinders;
            &(catalogue.ecus.iter())
                .filter(|ecu| !ecu.model.locked && ecu.model.cylinders >= cylinders)
                .min_by_key(|ecu| ecu.model.cylinders)?
                .id
        }
        _ => return None,
    };
    Some(id.clone())
}

/// The engine family and version of the first long block in `text` (saved parts).
fn first_block(catalogue: &Catalogue, text: &str) -> Option<(usize, usize)> {
    text.lines().find_map(|line| {
        let id = line.strip_prefix("part\t")?.split('\t').nth(2)?;
        let def = catalogue.defs.iter().find(|def| def.id == id)?;
        match def.does {
            Does::Block { family, version } => Some((family, version)),
            _ => None,
        }
    })
}

/// A part saved in a long block that now goes into its cylinder head (before version 5)
/// into `block`'s head, which is the factory one of its version if it has none yet.
/// Whether it went in.
fn into_old_head(catalogue: &Catalogue, block: &mut Item, part: &Item) -> bool {
    let Does::Block { family, version } = block.def(catalogue).does else {
        return false;
    };
    let kind = part.def(catalogue).kind;
    let slots = catalogue.defs[block.def].slots.clone();
    let Some(place) = (slots.iter()).position(|slot| slot.kind == Kind::Head) else {
        return false;
    };
    if block.slots[place].is_none() {
        let head = catalogue.families[family].versions[version].preset.head;
        let does = Does::Head { family, head };
        let Some(def) = catalogue.defs.iter().position(|def| def.does == does) else {
            return false;
        };
        block.slots[place] = Some(Item::new(catalogue, def, 0));
    }
    let head = block.slots[place].as_mut().expect("a head");
    let holes = catalogue.defs[head.def].slots.clone();
    let free = (holes.iter().zip(&head.slots))
        .position(|(slot, fitted)| slot.kind == kind && fitted.is_none());
    match free {
        Some(free) => {
            let mut part = part.clone();
            if part.def(catalogue).wear.is_none() && !part.wear.is_empty() {
                head.wear = std::mem::take(&mut part.wear);
            }
            head.slots[free] = Some(part);
            true
        }
        None => false,
    }
}

/// What a size is saved as: rims' and tires' labels (their lists may grow), else its place.
fn variant_text(def: &crate::items::PartDef, variant: usize) -> String {
    match def.does {
        Does::Rim => Rim::all().nth(variant).map(|rim| rim.label()),
        Does::Tire(_) => TireSize::all().nth(variant).map(|size| size.label()),
        _ => None,
    }
    .unwrap_or_else(|| variant.to_string())
}

fn variant_of(def: &crate::items::PartDef, text: &str) -> Option<usize> {
    match def.does {
        Does::Rim => Rim::all().position(|rim| rim.label() == text),
        Does::Tire(_) => TireSize::all().position(|size| size.label() == text),
        _ => text.parse().ok(),
    }
}

fn write_item(catalogue: &Catalogue, item: &Item, depth: usize, slot: &str, text: &mut String) {
    let def = item.def(catalogue);
    let paint = item
        .paint
        .and_then(|paint| catalogue.paints.get(paint))
        .map_or("-", |paint| paint.name.as_str());
    *text += &format!(
        "part\t{depth}\t{slot}\t{}\t{paint}\t{}",
        def.id,
        variant_text(def, item.variant)
    );
    for (knob, value) in def.knobs.iter().zip(&item.knobs) {
        *text += &format!("\t{}={value}", knob.name);
    }
    if !item.wear.is_empty() {
        let wear: Vec<String> = item.wear.iter().map(f32::to_string).collect();
        *text += &format!("\t{WEAR}={}", wear.join(","));
    }
    *text += "\n";
    for (slot, fitted) in def.slots.iter().zip(&item.slots) {
        if let Some(fitted) = fitted {
            write_item(catalogue, fitted, depth + 1, &slot.name, text);
        }
    }
}

/// Reads a car written by `write`, as `catalogue` has its parts now: what it's built of
/// (checked as a car from a player's game is) and its condition.
pub fn read(catalogue: &Shared, text: &str) -> Result<(CarSpec, Condition), String> {
    let (car_text, tune) = match text.split_once(&format!("\n{TUNE}\n")) {
        Some((car, tune)) => (car, tune),
        None => (text, ""),
    };
    let mut condition = Condition {
        fuel: 0.0,
        octane: crate::physics::fuel::PUMP_OCTANE,
        treads: [[0.0; RIBS]; 4],
        damage: Damage::default(),
        pads: [0.0; 4],
        clutch: 0.0,
        gears: [0.0; MAX_GEARS],
        shafts: [0.0; SHAFTS],
    };
    let mut fuel = None;
    let mut version = 1;
    // The car's parts, then (from the `cargo` line on) what it carries.
    let mut car_parts = Parts {
        engine: first_block(catalogue, car_text),
        ..Parts::default()
    };
    let mut cargo_parts: Option<Parts> = None;
    for line in car_text.lines() {
        let line = line.trim_end();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line == CARGO {
            let engine = car_parts.engine;
            cargo_parts.get_or_insert_with(|| Parts {
                engine,
                ..Parts::default()
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("part\t") {
            cargo_parts
                .as_mut()
                .unwrap_or(&mut car_parts)
                .line(catalogue, rest)?;
            continue;
        }
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or_default();
        let numbers: Vec<f32> = words.filter_map(|w| w.parse().ok()).collect();
        match (key, numbers.as_slice()) {
            ("version", [v]) if *v as u32 > VERSION => {
                return Err(format!("a car saved by a newer game (version {v})"));
            }
            ("version", [v]) => version = *v as u32,
            ("fuel", [litres, octane]) => fuel = Some((*litres, *octane)),
            ("tread", [corner, wear @ ..]) if wear.len() == RIBS => {
                if let Some(tread) = condition.treads.get_mut(*corner as usize) {
                    tread.copy_from_slice(wear);
                }
            }
            ("engine", [gasket, bearings, turbo]) => {
                let damage = &mut condition.damage;
                (damage.head_gasket, damage.bearings, damage.turbo) = (*gasket, *bearings, *turbo);
            }
            ("cylinder", [i, piston, valves, rod]) if (*i as usize) < MAX_CYLINDERS => {
                let cylinder = &mut condition.damage.cylinders[*i as usize];
                (cylinder.piston, cylinder.valves, cylinder.rod) = (*piston, *valves, *rod);
            }
            _ => {}
        }
    }
    let (cars, mut orphans) = car_parts.finish(catalogue);
    let mut car = cars.into_iter().next().ok_or("no chassis")?;
    fill_new_slots(catalogue, &mut car);
    fill_rear(catalogue, &mut car, &mut orphans);
    fill_front_shafts(catalogue, &mut car);
    let mut cargo = cargo_parts.map_or_else(Vec::new, |parts| {
        let (cargo, loose) = parts.finish(catalogue);
        orphans.extend(loose);
        cargo
    });
    cargo.extend(orphans);
    let loose = |item: &Item| {
        item.def(catalogue).kind != crate::items::Kind::Chassis && item.fine(catalogue)
    };
    let cargo: Vec<Item> = cargo.into_iter().filter(loose).collect();
    let cargo = crate::cargo::fitting(catalogue, &car, cargo);
    let (build, parts) = car
        .resolve(catalogue)
        .map_err(|e| format!("not a car: {}", e.0))?;
    let mut ecu = Ecu::blank(&parts.hardware(), parts.ecu_model());
    ecu.read_text(tune);
    // Saved before the second cam profile had maps of its own: the first's.
    if !tune.contains("table ve_high") {
        ecu.cams.ve = ecu.ve.clone();
        ecu.cams.lambda = ecu.lambda.clone();
        ecu.cams.spark = ecu.spark.clone();
    }
    ecu.model = parts.ecu_model();
    // A locked ECU runs its own maps.
    if let (true, Some(tune)) = (ecu.model.locked, &parts.ecu_def().tune) {
        ecu = tune.clone();
    }
    let spec = CarSpec {
        car,
        build,
        parts,
        ecu,
        cargo,
    };
    // Checked as a car from a player's game.
    let mut spec: CarSpec = decode_with(&encode(&spec), Some(catalogue)).ok_or(
        "not a car the catalogue allows (a part in the wrong place, a setting out of range)",
    )?;
    let (litres, octane) = fuel.unwrap_or((spec.build.chassis().tank.litres, condition.octane));
    let finite = |x: f32| x.is_finite().then_some(x);
    condition.fuel = finite(litres).unwrap_or(0.0).max(0.0);
    condition.octane = finite(octane)
        .unwrap_or(condition.octane)
        .clamp(80.0, 110.0);
    if version < 3 {
        // The car's wear, onto its parts.
        condition.put_on(catalogue, &mut spec.car);
    }
    let wear = |x: &mut f32| *x = finite(*x).unwrap_or(0.0).clamp(0.0, 1.0);
    condition.treads.iter_mut().flatten().for_each(wear);
    let damage = &mut condition.damage;
    [
        &mut damage.head_gasket,
        &mut damage.bearings,
        &mut damage.turbo,
    ]
    .into_iter()
    .for_each(wear);
    for cylinder in &mut damage.cylinders {
        [
            &mut cylinder.piston,
            &mut cylinder.valves,
            &mut cylinder.rod,
        ]
        .into_iter()
        .for_each(wear);
    }
    if version >= 3 {
        condition = Condition::of_parts(catalogue, &spec.car, condition.fuel, condition.octane);
    }
    Ok((spec, condition))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;
    use crate::terrain::Terrain;

    fn car(chassis: usize, option: usize) -> (CarSpec, Car) {
        let setup = CarSetup::factory(chassis, option);
        let spec = CarSpec::of(&setup);
        let car = Car::spawn_at(0, setup, &Terrain::flat(), 0.0, 0.0, 0.0);
        (spec, car)
    }

    /// Every stock car, worn, comes back as it was.
    #[test]
    fn a_saved_car_reads_back_as_it_was() {
        let catalogue = crate::catalogue::builtin();
        for (chassis, made) in catalogue.chassis.iter().enumerate() {
            for (option, programmable) in
                (0..made.engines.len()).flat_map(|o| [(o, false), (o, true)])
            {
                let (mut spec, mut car) = car(chassis, option);
                // A programmable ECU with a tune of its own (a locked one runs its own maps),
                // a painted bonnet, some wear and fuel burnt.
                if programmable {
                    let cylinders = spec.parts.hardware().cylinders;
                    spec.parts.ecu = (catalogue.ecus.iter())
                        .position(|e| !e.model.locked && e.model.cylinders >= cylinders)
                        .unwrap();
                    spec.ecu.model = spec.parts.ecu_model();
                    spec.car = crate::items::assemble(
                        &catalogue,
                        Some(&spec.car),
                        &spec.build,
                        &spec.parts,
                    )
                    .unwrap();
                    spec.ecu.rev_limit -= 123.5;
                    spec.ecu.spark.set_cell(&[3, 4], 17.25);
                }
                // And a silencer in the trunk.
                let silencer = (catalogue.defs.iter())
                    .position(|def| def.id == "silencer.sport")
                    .unwrap();
                spec.cargo = vec![Item::new(&catalogue, silencer, 0)];
                car.fuel = Fuel::with(17.3, 97.5);
                car.corners[2].tread.wear = [0.1, 0.2, 0.3, 0.2, 0.15];
                car.drivetrain.engine.damage.cylinders[1].piston = 0.4;
                car.drivetrain.engine.damage.head_gasket = 0.05;
                let text = write(&catalogue, &spec, &Condition::of(&car));
                let (read, condition) = read(&catalogue, &text).expect("reads back");
                // The wear on the parts.
                Condition::of(&car).put_on(&catalogue, &mut spec.car);
                assert_eq!(read, spec, "{text}");
                assert_eq!(condition, Condition::of(&car));
            }
        }
    }

    /// Cars saved before the rear driveline was made of parts get their chassis' factory
    /// rear subframe; the roadster's limited slip, saved in its inline gearbox, goes into
    /// the rear subframe's mount, and its prop shaft comes with it.
    /// The shafts' wear comes back as saved, on the shafts; a car saved before front drive
    /// shafts were parts gets new stock ones.
    #[test]
    fn shaft_wear_reads_back_and_old_cars_get_front_shafts() {
        let catalogue = crate::catalogue::builtin();
        let bothy = (catalogue.trims.iter())
            .find(|t| t.id == "bothy.2-4-awd")
            .unwrap();
        let setup = CarSetup::of_trim(&catalogue, &bothy.id).unwrap();
        let spec = CarSpec::of(&setup);
        let mut car = Car::spawn_at(0, setup, &Terrain::flat(), 0.0, 0.0, 0.0);
        car.drivetrain.shaft_wear = [1.0, 0.25, 0.5];
        let text = write(&catalogue, &spec, &Condition::of(&car));
        // Read with the wear on the parts.
        let worn = |shafts| {
            let mut worn = spec.clone();
            let condition = Condition {
                shafts,
                ..Condition::of(&car)
            };
            condition.put_on(&catalogue, &mut worn.car);
            worn
        };
        let (back, condition) = read(&catalogue, &text).expect("reads back");
        assert_eq!(back, worn([1.0, 0.25, 0.5]));
        assert_eq!(condition.shafts, [1.0, 0.25, 0.5]);
        let old: String = (text.lines())
            .filter(|line| !line.contains("\tFront drive shafts\t"))
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(old.len() < text.len());
        let (back, condition) = read(&catalogue, &old).expect("reads back");
        assert_eq!(back, worn([0.0, 0.25, 0.5]));
        assert_eq!(condition.shafts, [0.0, 0.25, 0.5]);
    }

    #[test]
    fn cars_saved_before_rear_subframes_get_their_factory_one() {
        let catalogue = crate::catalogue::builtin();
        let merrow = catalogue.find_chassis("Merrow").unwrap();
        for (chassis, option) in [(0, 1), (merrow, 0)] {
            let (spec, car) = car(chassis, option);
            let text = write(&catalogue, &spec, &Condition::of(&car));
            // As the game wrote it then: no rear subframe (nor what's in it) or prop
            // shaft, and an inline gearbox with its differential.
            let mut old = String::new();
            let mut skipping = false;
            for line in text.lines() {
                let fields: Vec<&str> = line.split('\t').collect();
                if let ["part", depth, slot, ..] = fields[..] {
                    if skipping && depth != "1" && depth != "0" {
                        continue;
                    }
                    skipping = slot == "Rear subframe";
                    if skipping || slot == "Prop shaft" {
                        continue;
                    }
                    old += line;
                    old += "\n";
                    if slot == "Final drive" && chassis == merrow {
                        old += "part\t2\tDifferential\tdifferential.limited-slip\t-\t0\n";
                    }
                    continue;
                }
                old += line;
                old += "\n";
            }
            assert!(!old.contains("subframe") && old.len() < text.len(), "{old}");
            let (read, _) = read(&catalogue, &old).expect("reads back");
            assert_eq!(read, spec, "{old}");
        }
    }

    #[test]
    fn the_wear_goes_onto_the_parts_it_is_in() {
        let catalogue = crate::catalogue::builtin();
        let (spec, mut car) = car(0, 0);
        let damage = &mut car.drivetrain.engine.damage;
        damage.cylinders[1].piston = 0.4;
        damage.cylinders[2].rod = 0.3;
        damage.cylinders[0].valves = 0.2;
        damage.bearings = 0.1;
        car.corners[3].tread.wear = [0.1, 0.2, 0.3, 0.2, 0.15];
        let condition = Condition::of(&car);
        let mut tree = spec.car.clone();
        condition.put_on(&catalogue, &mut tree);
        let worn = |id: &str| {
            (tree.all().into_iter())
                .find(|item| item.def(&catalogue).id == id)
                .map(|item| item.wear.clone())
                .unwrap()
        };
        assert_eq!(worn("pistons.stock"), [0.0, 0.4]);
        assert_eq!(worn("connecting-rods.stock"), [0.0, 0.0, 0.3]);
        assert_eq!(worn("b16.head"), [0.2]);
        assert_eq!(worn("crankshaft.stock"), [0.1]);
        assert!(worn("head-gasket.stock").is_empty());
        let back = Condition::of_parts(&catalogue, &tree, condition.fuel, condition.octane);
        assert_eq!(back, condition);
        // A car saved before the parts carried it: its wear goes onto them.
        let text = write(&catalogue, &spec, &Condition::of(&car));
        let old: String = (text.lines())
            .map(|line| match line.split_once("\twear=") {
                Some((before, _)) => before.to_string(),
                None if line.starts_with("version") => "version 2\ncylinder 1 0.4 0 0".into(),
                None => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let (old, condition) = read(&catalogue, &old).unwrap();
        assert_eq!(condition.damage.cylinders[1].piston, 0.4);
        let pistons = (old.car.all().into_iter())
            .find(|item| item.def(&catalogue).id == "pistons.stock")
            .unwrap();
        assert_eq!(pistons.wear, [0.0, 0.4]);
    }

    #[test]
    fn a_part_gone_from_the_catalogue_goes_from_the_car() {
        let catalogue = crate::catalogue::builtin();
        let (spec, car) = car(0, 0);
        let text = write(&catalogue, &spec, &Condition::of(&car));
        // The passenger seat isn't there any more: the car comes without it.
        let seat = (spec.car.slots.iter().flatten())
            .map(|item| &catalogue.defs[item.def])
            .find(|def| matches!(def.kind, crate::items::Kind::Seat(_)))
            .unwrap();
        let line = text
            .lines()
            .find(|line| line.contains("Passenger seat"))
            .expect("a passenger seat");
        let gone = text.replace(line, &line.replace(&seat.id, "seat.gone"));
        let (read, _) = read(&catalogue, &gone).expect("still a car");
        assert!(read.car.all().len() < spec.car.all().len());
        // An engine gone is no car.
        let engine = text.lines().find(|l| l.contains("\tEngine\t")).unwrap();
        let id = engine.split('\t').nth(3).unwrap();
        let broken = text.replace(engine, &engine.replace(id, "engine.gone"));
        assert!(read_err(&catalogue, &broken));
    }

    #[test]
    fn loose_parts_read_back() {
        let catalogue = crate::catalogue::builtin();
        let car = &catalogue.trims[1].car;
        let wheel = car
            .fitted(&catalogue, crate::items::Kind::Wheel)
            .next()
            .unwrap();
        let engine = car
            .fitted(&catalogue, crate::items::Kind::Engine)
            .next()
            .unwrap();
        let items = vec![wheel.clone(), engine.clone()];
        let text = write_items(&catalogue, &items);
        assert_eq!(read_items(&catalogue, &text), items);
        // A car isn't a loose part.
        assert!(
            read_items(
                &catalogue,
                &write_items(&catalogue, std::slice::from_ref(car))
            )
            .is_empty()
        );
    }

    /// `text` as a game before version 5 wrote it: the cylinder head's parts in the long
    /// block (its porting the old "Cylinder head" with the valves' wear), no head, no
    /// lambda sensor.
    fn before_heads(text: &str) -> String {
        let mut out = Vec::new();
        let mut wear = None;
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["version", ..] => out.push("version 4".to_string()),
                // No lambda sensor before version 5.
                ["part", _, "Lambda sensor", ..] => {}
                ["part", "3", "Cylinder head", _, rest @ ..] => {
                    wear = rest.iter().find(|f| f.starts_with(WEAR)).copied();
                }
                ["part", "4", "Porting", id, rest @ ..] => {
                    let id = id.replace("porting.", OLD_PORTING);
                    let mut line = format!("part\t3\tCylinder head\t{id}\t{}", rest.join("\t"));
                    if let Some(wear) = wear {
                        line += &format!("\t{wear}");
                    }
                    out.push(line);
                }
                ["part", "4", rest @ ..] => out.push(format!("part\t3\t{}", rest.join("\t"))),
                _ => out.push(line.to_string()),
            }
        }
        out.join("\n")
    }

    #[test]
    fn ecus_saved_before_there_was_one_of_each_are_the_engines_own() {
        let catalogue = crate::catalogue::builtin();
        let ecu_of = |chassis: usize, old: &str| {
            let (spec, car) = car(chassis, 0);
            let text = write(&catalogue, &spec, &Condition::of(&car));
            let id = &spec.parts.ecu_def().id;
            let text = text.replace(&format!("\t{id}\t"), &format!("\t{old}\t"));
            let (read, _) = read(&catalogue, &text).expect("reads");
            read.parts.ecu_def().id.clone()
        };
        assert_eq!(ecu_of(0, "ecu.stock"), "ecu.b16");
        assert_eq!(ecu_of(1, "ecu.stock"), "ecu.r80");
        assert_eq!(ecu_of(0, "ecu.programmable"), "ecu.programmable-4");
        assert_eq!(ecu_of(1, "ecu.programmable"), "ecu.programmable-10");
    }

    #[test]
    fn the_b18_saved_is_the_b20() {
        let catalogue = crate::catalogue::builtin();
        let (spec, car) = car(0, 1);
        assert_eq!(spec.parts.ecu_def().id, "ecu.b20");
        let text = write(&catalogue, &spec, &Condition::of(&car));
        let old = text
            .replace("\tb20", "\tb18")
            .replace("\tecu.b20", "\tecu.b18");
        assert!(
            old.contains("\tb18.head\t") && old.contains("\tecu.b18\t"),
            "{old}"
        );
        let (read, _) = read(&catalogue, &old).expect("reads");
        assert_eq!(read.car, spec.car);
    }

    #[test]
    fn cars_saved_before_the_heads_were_parts_read_as_they_were() {
        let catalogue = crate::catalogue::builtin();
        let (mut spec, mut car) = car(0, 0);
        car.drivetrain.engine.damage.cylinders[0].valves = 0.2;
        let ported = |c: &Catalogue| c.defs.iter().position(|d| d.id == "porting.ported");
        let camshafts = |c: &Catalogue| c.defs.iter().position(|d| d.id == "camshafts.sport");
        let setup = CarSetup::factory(0, 0);
        let mut parts = setup.engine.parts.clone();
        parts.chosen[19] = 1;
        parts.chosen[5] = 1;
        spec.car =
            crate::items::assemble(&catalogue, Some(&setup.car), &setup.build, &parts).unwrap();
        assert!(ported(&catalogue).is_some() && camshafts(&catalogue).is_some());
        let text = write(&catalogue, &spec, &Condition::of(&car));
        let old = before_heads(&text);
        assert!(
            old.contains("cylinder-head.ported") && !old.contains(".head\t"),
            "{old}"
        );
        let (read, condition) = read(&catalogue, &old).expect("reads");
        Condition::of(&car).put_on(&catalogue, &mut spec.car);
        assert_eq!(read.car, spec.car, "{old}");
        assert_eq!(condition.damage.cylinders[0].valves, 0.2);
    }

    /// `text` as a game before version 4 wrote it: the engine's manifold the old part
    /// `old`, and the turbo on its mounts (if any) in the old turbo slot.
    fn before_mounts(text: &str, old: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let turbo = (lines.iter()).find_map(|line| line.strip_prefix("part\t3\tTurbo"));
        let mut out = Vec::new();
        for line in lines {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["version", ..] => out.push("version 3".to_string()),
                ["part", "2", "Exhaust manifold", _, rest @ ..] => out.push(format!(
                    "part\t2\tExhaust manifold\t{old}\t{}",
                    rest.join("\t")
                )),
                ["part", "3", slot, ..] if slot.starts_with("Turbo") => {}
                ["part", "2", "Supercharger", ..] if turbo.is_some() => {
                    let turbo = turbo.unwrap();
                    let turbo = turbo.split_once('\t').map_or(turbo, |(_, rest)| rest);
                    out.push(format!("part\t2\tTurbo or supercharger\t{turbo}"));
                }
                ["part", "2", "Supercharger", rest @ ..] => out.push(format!(
                    "part\t2\tTurbo or supercharger\t{}",
                    rest.join("\t")
                )),
                _ => out.push(line.to_string()),
            }
        }
        out.join("\n")
    }

    /// The stock car `chassis`, `option` with the family's manifold `manifold` and the
    /// turbo `turbo` on it (0: none), worn.
    fn built(chassis: usize, option: usize, manifold: usize, turbo: usize) -> (CarSpec, Car) {
        let setup = CarSetup::factory(chassis, option);
        let mut parts = setup.engine.parts.clone();
        parts.manifold = manifold;
        parts.chosen[crate::catalogue::TURBO] = turbo;
        let built = setup.changed(setup.build.clone(), parts, setup.engine.build.clone());
        let mut car = Car::spawn_at(0, built.clone(), &Terrain::flat(), 0.0, 0.0, 0.0);
        car.drivetrain.engine.damage.turbo = 0.25;
        (CarSpec::of(&built), car)
    }

    #[test]
    fn cars_saved_before_the_manifolds_had_mounts_read_as_they_were() {
        let catalogue = crate::catalogue::builtin();
        let ids = |spec: &CarSpec| -> Vec<String> {
            (spec.car.all().into_iter())
                .map(|item| item.def(&catalogue).id.clone())
                .filter(|id| id.starts_with("b.") || id.starts_with("r.") || id.contains("turbo"))
                .collect()
        };
        let cases = [
            // The 1.6 with a turbo on the stock manifold; the 1.8 with long tubes; the
            // V10 with short headers (gone: its stock ones), and with long tubes and a
            // turbo (the pair of turbo manifolds, nearer than the 10-into-1).
            ((0, 0, 3, 2), "exhaust-manifold.stock", (3, 2)),
            ((0, 1, 2, 0), "exhaust-manifold.long-tube-headers", (2, 0)),
            ((1, 0, 0, 0), "exhaust-manifold.short-headers", (0, 0)),
            ((1, 0, 2, 1), "exhaust-manifold.long-tube-headers", (2, 1)),
        ];
        for ((chassis, option, manifold, turbo), old, (now, turbo_now)) in cases {
            let (spec, car) = built(chassis, option, manifold, turbo);
            let text = write(&catalogue, &spec, &Condition::of(&car));
            let (read, condition) = read(&catalogue, &before_mounts(&text, old)).unwrap();
            let (expected, _) = built(chassis, option, now, turbo_now);
            assert_eq!(ids(&read), ids(&expected), "{old}");
            assert_eq!(read.parts.manifold, now);
            assert_eq!(read.parts.chosen[crate::catalogue::TURBO], turbo_now);
            let worn = if turbo > 0 { 0.25 } else { 0.0 };
            assert_eq!(condition.damage.turbo, worn, "{old}");
        }
        // The 1.6's turbo car reads back exactly.
        let (spec, car) = built(0, 0, 3, 2);
        let text = write(&catalogue, &spec, &Condition::of(&car));
        let (read, _) = read(&catalogue, &before_mounts(&text, "exhaust-manifold.stock")).unwrap();
        let mut worn = spec.clone();
        Condition::of(&car).put_on(&catalogue, &mut worn.car);
        assert_eq!(read, worn);
        // A loose old manifold is the first family's.
        let loose = "part\t0\t-\texhaust-manifold.short-headers\t-\t0\n";
        let items = read_items(&catalogue, loose);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].def(&catalogue).id, "b.headers.short");
    }

    fn read_err(catalogue: &Shared, text: &str) -> bool {
        read(catalogue, text).is_err()
    }
}
