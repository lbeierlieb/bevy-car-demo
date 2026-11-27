//! A car as the server keeps it between sessions: text, readable whatever the catalogue
//! has become. What it's built of goes by the definitions' string ids, slot and knob names,
//! paint names and sizes' labels (a definition gone takes its item with it, a knob gone
//! is dropped, a new one starts where it starts), each part's wear (the engine's on the
//! parts it's in, the tires' on the tires: see `Condition::put_on`), then the ECU's tune
//! (all of it). The fuel is the car's own.
//!
//! ```text
//! # car-sim car
//! version 3
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

use crate::car::Car;
use crate::catalogue::{Catalogue, Shared};
use crate::items::{Does, Item, Kind, Wear};
use crate::net::wire::{decode_with, encode};
use crate::physics::engine::damage::{Damage, MAX_CYLINDERS};
use crate::physics::engine::ecu::Ecu;
use crate::physics::fuel::Fuel;
use crate::physics::tire::RIBS;
use crate::spec::CarSpec;
use crate::wheels::{Rim, TireSize};

/// The format's version.
const VERSION: u32 = 3;

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
}

impl Condition {
    pub fn of(car: &Car) -> Self {
        Self {
            fuel: car.fuel.litres(),
            octane: car.fuel.octane(),
            treads: car.corners.each_ref().map(|corner| corner.tread.wear),
            damage: car.drivetrain.engine.damage,
            pads: car.corners.each_ref().map(|corner| corner.brake.wear),
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
    }

    /// Puts the wear on the parts of `car` (a car's tree) that carry it (`Item::wear`): the
    /// engine's on the parts of it each kind of wear is in (pistons, rods, the head's
    /// valves, the head gasket, the crank's bearings, the turbo), each tire's tread on the
    /// tire at its corner.
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
                // By the corner (below).
                Wear::Tread | Wear::Pads => return,
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
    }

    /// The condition of a car whose parts carry `car`'s wear (see `put_on`), with `fuel`
    /// litres of `octane` in the tank.
    pub fn of_parts(catalogue: &Catalogue, car: &Item, fuel: f32, octane: f32) -> Self {
        let mut damage = Damage::default();
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
                Wear::Turbo => damage.turbo = at(0),
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
        Self {
            fuel,
            octane,
            treads,
            damage,
            pads,
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
    let items = parts.finish(catalogue);
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
        let Some(def) = catalogue.defs.iter().position(|def| def.id == *id) else {
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
                item.wear = made.wear.map_or_else(Vec::new, |_| trimmed(values));
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
    /// slot's name: the first free one called so), a tree's root among the roots. An item
    /// without a slot for it is dropped.
    fn settle(&mut self, catalogue: &Catalogue, depth: usize) {
        while self.stack.last().is_some_and(|(d, ..)| *d >= depth) {
            let (_, slot, item) = self.stack.pop().expect("an item");
            let Some((_, _, parent)) = self.stack.last_mut() else {
                self.roots.push(item);
                continue;
            };
            let slots = &catalogue.defs[parent.def].slots;
            let free = (slots.iter().zip(&parent.slots))
                .position(|(s, fitted)| s.name == slot && fitted.is_none());
            match free {
                Some(place) => parent.slots[place] = Some(item),
                None => bevy::log::warn!(
                    "A saved part {} has no slot \"{slot}\" any more",
                    catalogue.defs[item.def].id
                ),
            }
        }
    }

    /// The trees read.
    fn finish(mut self, catalogue: &Catalogue) -> Vec<Item> {
        self.settle(catalogue, 0);
        self.roots
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
    };
    let mut fuel = None;
    let mut version = 1;
    // The car's parts, then (from the `cargo` line on) what it carries.
    let mut car_parts = Parts::default();
    let mut cargo_parts: Option<Parts> = None;
    for line in car_text.lines() {
        let line = line.trim_end();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line == CARGO {
            cargo_parts.get_or_insert_default();
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
    let car = car_parts
        .finish(catalogue)
        .into_iter()
        .next()
        .ok_or("no chassis")?;
    let cargo = cargo_parts.map_or_else(Vec::new, |parts| parts.finish(catalogue));
    let loose = |item: &Item| {
        item.def(catalogue).kind != crate::items::Kind::Chassis && item.fine(catalogue)
    };
    let cargo: Vec<Item> = cargo.into_iter().filter(loose).collect();
    let cargo = crate::cargo::fitting(catalogue, &car, cargo);
    let (build, parts) = car
        .resolve(catalogue)
        .map_err(|e| format!("not a car: {}", e.0))?;
    let mut ecu = Ecu::blank(&parts.hardware(), parts.ecu);
    ecu.read_text(tune);
    ecu.model = parts.ecu;
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
            for option in 0..made.engines.len() {
                let (mut spec, mut car) = car(chassis, option);
                // A tune of its own, a painted bonnet, some wear and fuel burnt.
                spec.ecu.rev_limit -= 123.5;
                spec.ecu.spark.set_cell(&[3, 4], 17.25);
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
        assert_eq!(worn("cylinder-head.stock"), [0.2]);
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

    fn read_err(catalogue: &Shared, text: &str) -> bool {
        read(catalogue, text).is_err()
    }
}
