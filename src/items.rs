//! Cars as parts: every part there is has a definition in the catalogue (`PartDef`, with
//! a stable string id), and a car is a tree of `Item`s, owned pieces of those definitions:
//! the chassis with a part in each of its slots, some parts with slots of their own (an
//! engine has a long block and bolt-ons, a wheel a rim and a tire, a gearbox a final drive
//! and a differential). A stock car is a `Trim`: such a tree as the factory builds it.
//!
//! The simulation still builds a car from a `CarBuild` and the engine's `Parts`:
//! `Item::resolve` works those out of a car's tree, and `assemble` puts a `CarBuild` and
//! `Parts` back into a tree (keeping what they don't say, such as each body part's
//! paint), for the shops that still change cars that way. Where a tree says more than a
//! `CarBuild` can (another spring on the right than on the left), the left one counts.

use crate::catalogue::Catalogue;
use crate::chassis::{BRAKE_BALANCES, BRAKE_FORCES, CarBuild, Misfit, PRESSURES, TOES};
use crate::physics::engine::ecu::EcuModel;
use crate::physics::engine::parts::Parts;
use crate::wheels::{Rim, TireSize, Wheel};

/// Body panels, each painted, all optional (a missing one only adds drag, once that's
/// simulated).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    FrontBumper,
    FrontLip,
    Bonnet,
    Fender,
    Door,
    Mirror,
    SideSkirt,
    Hatch,
    RearSpoiler,
    RearBumper,
    Diffuser,
}

impl Panel {
    pub const ALL: [Panel; 11] = [
        Panel::FrontBumper,
        Panel::FrontLip,
        Panel::Bonnet,
        Panel::Fender,
        Panel::Door,
        Panel::Mirror,
        Panel::SideSkirt,
        Panel::Hatch,
        Panel::RearSpoiler,
        Panel::RearBumper,
        Panel::Diffuser,
    ];
}

/// Seats: the driver's (a slot that must have one) and the passenger's are front seats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Front,
    RearBench,
}

impl Seat {
    pub const ALL: [Seat; 2] = [Seat::Front, Seat::RearBench];
}

/// What kind of slot a part goes into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A car's shell, the root of its tree.
    Chassis,
    Body(Panel),
    Seat(Seat),
    /// A rim with a tire on it.
    Wheel,
    Rim,
    Tire,
    Brake,
    Spring,
    Damper,
    AntiRollBar,
    Gearbox,
    FinalDrive,
    Differential,
    Clutch,
    Ecu,
    /// A whole engine: its long block and the bolt-ons.
    Engine,
    /// The block with what's inside it, the engine shop's work.
    LongBlock,
    /// One of the catalogue's engine slots (`Catalogue::slots`, by its place).
    EnginePart(usize),
}

/// What a part does, as far as the game's code knows kinds of parts.
#[derive(Clone, Debug, PartialEq)]
pub enum Does {
    /// The catalogue's chassis (by its place).
    Chassis(usize),
    /// A body panel or a seat: only there, for now.
    Nothing,
    /// A wheel: holds a rim and a tire.
    Wheel,
    /// A rim in any of the sizes there are (`Rim::all`, the item's `variant`).
    Rim,
    /// A tire of the catalogue's compound (by its place), in any of the sizes there are
    /// (`TireSize::all`, the item's `variant`).
    Tire(usize),
    Brake,
    /// A spring and a damper from the catalogue's suspension kits (by its place).
    Spring(usize),
    Damper(usize),
    /// The anti-roll bar of the front (0) or rear (1) axle, in the chassis' sizes for it
    /// (the item's `variant`).
    AntiRollBar(usize),
    /// The catalogue's gearbox (by its place).
    Gearbox(usize),
    FinalDrive,
    Differential {
        limited_slip: bool,
    },
    /// The catalogue's clutch (by its place).
    Clutch(usize),
    Ecu(EcuModel),
    Engine,
    /// A version of an engine family (both by their places in the catalogue).
    Block {
        family: usize,
        version: usize,
    },
    /// Part `part` of the catalogue's engine slot `slot`.
    EnginePart {
        slot: usize,
        part: usize,
    },
}

/// A setting a part has: what it's called, the range it can be set in and where it
/// starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Knob {
    pub name: String,
    pub low: f32,
    pub high: f32,
    pub default: f32,
}

/// A slot a part has for another.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotDef {
    pub name: String,
    pub kind: Kind,
    /// Must have a part (else the car can't be driven, or the part not built).
    pub required: bool,
}

/// A part there is.
#[derive(Clone, Debug, PartialEq)]
pub struct PartDef {
    /// Stable, for saved cars and items: e.g. `vireo.bonnet`, `pistons.forged`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub does: Does,
    /// The chassis it's made for (their names); none: any.
    pub fits: Vec<String>,
    /// The box it's packed in: length, width, height (m).
    pub size: [f32; 3],
    pub mass: f32,
    /// In cents.
    pub price: u32,
    /// Painted (in the catalogue's `paints`, each item its own).
    pub painted: bool,
    pub knobs: Vec<Knob>,
    pub slots: Vec<SlotDef>,
}

/// A stock car: a chassis as the factory builds it, the showrooms' wares.
#[derive(Clone, Debug, PartialEq)]
pub struct Trim {
    pub id: String,
    pub name: String,
    pub car: Item,
}

/// An owned part, and what's fitted into its slots.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Its definition, by its place in the catalogue's `defs`.
    pub def: usize,
    /// Its paint (in the catalogue's `paints`), if it's painted.
    pub paint: Option<usize>,
    /// Which of the sizes it comes in (rims, tires, anti-roll bars).
    pub variant: usize,
    /// Its definition's knobs, as set.
    pub knobs: Vec<f32>,
    /// A part (or none) for each of its definition's slots.
    pub slots: Vec<Option<Item>>,
}

/// Why a tree doesn't make a car.
#[derive(Clone, Debug, PartialEq)]
pub struct Unfit(pub String);

impl From<Misfit> for Unfit {
    fn from(misfit: Misfit) -> Self {
        Unfit(misfit.0)
    }
}

/// Corners in the order the chassis' slots have them; axle 0 front.
pub const CORNERS: [&str; 4] = ["front left", "front right", "rear left", "rear right"];

impl PartDef {
    /// How many sizes it comes in (one for most parts).
    pub fn variants(&self, catalogue: &Catalogue, chassis: Option<usize>) -> usize {
        match self.does {
            Does::Rim => Rim::all().count(),
            Does::Tire(_) => TireSize::all().count(),
            Does::AntiRollBar(axle) => {
                chassis.map_or(0, |c| catalogue.chassis[c].bars[axle].sizes.len())
            }
            _ => 1,
        }
    }
}

impl Item {
    /// A new item of `def` in `catalogue`: its knobs where they start, its slots empty,
    /// painted in `paint` if it's painted.
    pub fn new(catalogue: &Catalogue, def: usize, paint: usize) -> Self {
        let made = &catalogue.defs[def];
        Self {
            def,
            paint: made.painted.then_some(paint),
            variant: 0,
            knobs: made.knobs.iter().map(|knob| knob.default).collect(),
            slots: vec![None; made.slots.len()],
        }
    }

    pub fn def<'a>(&self, catalogue: &'a Catalogue) -> &'a PartDef {
        &catalogue.defs[self.def]
    }

    /// The parts in the slots of `kind`, in order (empty slots skipped).
    pub fn fitted<'a>(
        &'a self,
        catalogue: &'a Catalogue,
        kind: Kind,
    ) -> impl Iterator<Item = &'a Item> + 'a {
        let slots = &self.def(catalogue).slots;
        (slots.iter().zip(&self.slots))
            .filter(move |(slot, _)| slot.kind == kind)
            .filter_map(|(_, item)| item.as_ref())
    }

    /// The `n`th part in a slot of `kind`.
    fn nth<'a>(
        &'a self,
        catalogue: &'a Catalogue,
        kind: Kind,
        n: usize,
    ) -> Result<&'a Item, Unfit> {
        self.fitted(catalogue, kind)
            .nth(n)
            .ok_or_else(|| Unfit(format!("no {kind:?} in {}", self.def(catalogue).name)))
    }

    /// Every item in the tree, this one first.
    pub fn all(&self) -> Vec<&Item> {
        let mut all = vec![self];
        for item in self.slots.iter().flatten() {
            all.extend(item.all());
        }
        all
    }

    /// What a car's tree comes to: the `CarBuild` and the engine's `Parts` (as installed)
    /// the simulation builds a car from.
    pub fn resolve(
        &self,
        catalogue: &crate::catalogue::Shared,
    ) -> Result<(CarBuild, Parts), Unfit> {
        let Does::Chassis(chassis) = self.def(catalogue).does else {
            return Err(Unfit("not a chassis".into()));
        };
        let made = &catalogue.chassis[chassis];
        let mut build = CarBuild::factory_in(catalogue, chassis);
        build.paint = self.paint.unwrap_or(made.paint);
        build.brake_force = self.knobs[0];
        build.brake_balance = self.knobs[1];
        for axle in 0..2 {
            let wheel = self.nth(catalogue, Kind::Wheel, 2 * axle)?;
            let rim = wheel.nth(catalogue, Kind::Rim, 0)?;
            let tire = wheel.nth(catalogue, Kind::Tire, 0)?;
            let Does::Tire(compound) = tire.def(catalogue).does else {
                return Err(Unfit("not a tire".into()));
            };
            build.wheels[axle] = Wheel {
                rim: Rim::all()
                    .nth(rim.variant)
                    .ok_or(Unfit("no such rim".into()))?,
                tire: (TireSize::all().nth(tire.variant)).ok_or(Unfit("no such tire".into()))?,
                compound,
            };
            build.pressure[axle] = tire.knobs[0];
            let damper = self.nth(catalogue, Kind::Damper, 2 * axle)?;
            build.camber[axle] = damper.knobs[0];
            build.toe[axle] = damper.knobs[1];
            build.bars[axle] = self.nth(catalogue, Kind::AntiRollBar, axle)?.variant;
        }
        let Does::Spring(kit) = self.nth(catalogue, Kind::Spring, 0)?.def(catalogue).does else {
            return Err(Unfit("not a spring".into()));
        };
        build.suspension = kit;
        let gearbox = self.nth(catalogue, Kind::Gearbox, 0)?;
        let Does::Gearbox(g) = gearbox.def(catalogue).does else {
            return Err(Unfit("not a gearbox".into()));
        };
        build.gearbox = g;
        build.final_drive = gearbox.nth(catalogue, Kind::FinalDrive, 0)?.knobs[0];
        let differential = gearbox.nth(catalogue, Kind::Differential, 0)?;
        let Does::Differential { limited_slip } = differential.def(catalogue).does else {
            return Err(Unfit("not a differential".into()));
        };
        build.limited_slip = limited_slip;
        let Does::Clutch(clutch) = self.nth(catalogue, Kind::Clutch, 0)?.def(catalogue).does else {
            return Err(Unfit("not a clutch".into()));
        };
        build.clutch = clutch;
        let Does::Ecu(ecu) = self.nth(catalogue, Kind::Ecu, 0)?.def(catalogue).does else {
            return Err(Unfit("not an ECU".into()));
        };
        let engine = self.nth(catalogue, Kind::Engine, 0)?;
        let block = engine.nth(catalogue, Kind::LongBlock, 0)?;
        let Does::Block { family, version } = block.def(catalogue).does else {
            return Err(Unfit("not a block".into()));
        };
        let mut parts = Parts::of_version(catalogue, family, version);
        parts.ecu = ecu;
        parts.intake_timing = engine.knobs[0];
        parts.exhaust_timing = engine.knobs[1];
        parts.boost = engine.knobs[2];
        // Engine parts sit in the long block, on the engine and in the car.
        for (slot, chosen) in parts.chosen.iter_mut().enumerate() {
            let kind = Kind::EnginePart(slot);
            let part = [block, engine, self]
                .iter()
                .find_map(|holder| holder.fitted(catalogue, kind).next())
                .ok_or_else(|| Unfit(format!("no {}", catalogue.slots[slot].name)))?;
            let Does::EnginePart { part, .. } = part.def(catalogue).does else {
                return Err(Unfit("not an engine part".into()));
            };
            *chosen = part;
        }
        let parts = build.install(parts)?;
        Ok((build, parts))
    }
}

/// What's seen of a car's body: the colours of the body parts fitted to `car` (a car's
/// tree), each by its slot (the first of a pair of slots is the left one), and the
/// shell's.
pub fn dress(catalogue: &Catalogue, car: &Item) -> crate::body::Dress {
    use crate::body::Side;
    let color = |paint: Option<usize>| {
        let paint = paint.or(car.paint).unwrap_or(0);
        catalogue
            .paints
            .get(paint)
            .map_or([0.5; 3], |paint| paint.color)
    };
    let mut seen = Vec::new();
    let mut pieces = Vec::new();
    for (slot, item) in car.def(catalogue).slots.iter().zip(&car.slots) {
        let Kind::Body(panel) = slot.kind else {
            continue;
        };
        let side = match seen.iter().filter(|&&p| p == panel).count() {
            0 => Side::Left,
            _ => Side::Right,
        };
        seen.push(panel);
        let side = match panel {
            Panel::Fender | Panel::Door | Panel::Mirror | Panel::SideSkirt => side,
            _ => Side::Middle,
        };
        if let Some(item) = item {
            pieces.push((panel, side, color(item.paint)));
        }
    }
    crate::body::Dress {
        shell: color(car.paint),
        pieces,
    }
}

/// The stock car of `catalogue`'s `chassis` with its factory engine `option`, if
/// there's one.
pub fn trim_of(catalogue: &Catalogue, chassis: usize, option: usize) -> Option<&Trim> {
    let code = catalogue.chassis.get(chassis)?.engines.get(option)?;
    catalogue.trims.iter().find(|trim| {
        let car = &trim.car;
        let block = (car.fitted(catalogue, Kind::Engine).next())
            .and_then(|engine| engine.fitted(catalogue, Kind::LongBlock).next());
        car.def(catalogue).does == Does::Chassis(chassis)
            && block.is_some_and(|block| match block.def(catalogue).does {
                Does::Block { family, version } => {
                    catalogue.families[family].versions[version].code == *code
                }
                _ => false,
            })
    })
}

/// The definition doing `does` (of `kind`, made for `chassis` if it's made for one).
pub fn find(catalogue: &Catalogue, kind: Kind, does: &Does, chassis: &str) -> Option<usize> {
    let fits = |def: &PartDef| def.fits.is_empty() || def.fits.iter().any(|c| c == chassis);
    (catalogue.defs.iter()).position(|def| def.kind == kind && def.does == *does && fits(def))
}

/// `build` and `parts` (from `resolve`, changed) put into `car`, a car's tree (of the same
/// chassis; else its trim's): every part they say anything about is swapped for one doing
/// that, knobs set; the rest stays (body parts, seats, brakes). A new paint goes on every
/// painted part, as the paint shop does.
pub fn assemble(
    catalogue: &crate::catalogue::Shared,
    car: Option<&Item>,
    build: &CarBuild,
    parts: &Parts,
) -> Result<Item, Unfit> {
    let chassis = &catalogue.chassis[build.chassis].name;
    let same = |item: &&Item| item.def(catalogue).does == Does::Chassis(build.chassis);
    let mut car = match car.filter(same) {
        Some(car) => car.clone(),
        None => (catalogue.trims.iter())
            .find(|trim| trim.car.def(catalogue).does == Does::Chassis(build.chassis))
            .ok_or_else(|| Unfit(format!("no trim of the {chassis}")))?
            .car
            .clone(),
    };
    let def = |kind: Kind, does: Does| {
        find(catalogue, kind, &does, chassis)
            .ok_or_else(|| Unfit(format!("no part doing {does:?}")))
    };
    let paint = build.paint;
    let repaint = car.paint != Some(paint);
    // A part in each slot of `kind` (the `n`th's definition made by `make(n)`), keeping
    // what's in it if that does the same (steel rims stay steel rims).
    fn swap(
        catalogue: &Catalogue,
        holder: &mut Item,
        kind: Kind,
        paint: usize,
        mut make: impl FnMut(usize, &mut Item) -> Result<usize, Unfit>,
    ) -> Result<(), Unfit> {
        let slots = holder.def(catalogue).slots.clone();
        let mut n = 0;
        for (slot, item) in slots.iter().zip(&mut holder.slots) {
            if slot.kind != kind {
                continue;
            }
            let mut fresh = item
                .clone()
                .unwrap_or_else(|| Item::new(catalogue, 0, paint));
            let def = make(n, &mut fresh)?;
            let does = &catalogue.defs[def].does;
            if item
                .as_ref()
                .is_none_or(|old| catalogue.defs[old.def].does != *does)
            {
                let mut new = Item::new(catalogue, def, paint);
                new.variant = fresh.variant;
                if new.knobs.len() == fresh.knobs.len() {
                    new.knobs = fresh.knobs;
                }
                fresh = new;
                // Set again, now on the right definition.
                make(n, &mut fresh)?;
            }
            *item = Some(fresh);
            n += 1;
        }
        Ok(())
    }
    car.knobs[0] = build.brake_force;
    car.knobs[1] = build.brake_balance;
    let tire = |wheel: &Wheel| {
        def(Kind::Tire, Does::Tire(wheel.compound)).map(|d| {
            let variant = TireSize::all().position(|t| t == wheel.tire);
            (d, variant)
        })
    };
    swap(catalogue, &mut car, Kind::Wheel, paint, |n, wheel| {
        let axle = n / 2;
        let w = build.wheels[axle];
        swap(catalogue, wheel, Kind::Rim, paint, |_, rim| {
            rim.variant = Rim::all()
                .position(|r| r == w.rim)
                .ok_or(Unfit("no such rim".into()))?;
            def(Kind::Rim, Does::Rim)
        })?;
        swap(catalogue, wheel, Kind::Tire, paint, |_, item| {
            let (d, variant) = tire(&w)?;
            item.variant = variant.ok_or(Unfit("no such tire".into()))?;
            if let Some(pressure) = item.knobs.first_mut() {
                *pressure = build.pressure[axle];
            }
            Ok(d)
        })?;
        def(Kind::Wheel, Does::Wheel)
    })?;
    swap(catalogue, &mut car, Kind::Spring, paint, |_, _| {
        def(Kind::Spring, Does::Spring(build.suspension))
    })?;
    swap(catalogue, &mut car, Kind::Damper, paint, |n, damper| {
        if let [camber, toe, ..] = &mut damper.knobs[..] {
            (*camber, *toe) = (build.camber[n / 2], build.toe[n / 2]);
        }
        def(Kind::Damper, Does::Damper(build.suspension))
    })?;
    swap(
        catalogue,
        &mut car,
        Kind::AntiRollBar,
        paint,
        |axle, bar| {
            bar.variant = build.bars[axle];
            def(Kind::AntiRollBar, Does::AntiRollBar(axle))
        },
    )?;
    swap(catalogue, &mut car, Kind::Gearbox, paint, |_, gearbox| {
        swap(catalogue, gearbox, Kind::FinalDrive, paint, |_, drive| {
            if let Some(ratio) = drive.knobs.first_mut() {
                *ratio = build.final_drive;
            }
            def(Kind::FinalDrive, Does::FinalDrive)
        })?;
        swap(catalogue, gearbox, Kind::Differential, paint, |_, _| {
            let does = Does::Differential {
                limited_slip: build.limited_slip,
            };
            def(Kind::Differential, does)
        })?;
        def(Kind::Gearbox, Does::Gearbox(build.gearbox))
    })?;
    swap(catalogue, &mut car, Kind::Clutch, paint, |_, _| {
        def(Kind::Clutch, Does::Clutch(build.clutch))
    })?;
    swap(catalogue, &mut car, Kind::Ecu, paint, |_, _| {
        def(Kind::Ecu, Does::Ecu(parts.ecu))
    })?;
    let engine_part = |slot: usize| def(Kind::EnginePart(slot), engine_does(parts, slot));
    let slots = catalogue.slots.len();
    swap(catalogue, &mut car, Kind::Engine, paint, |_, engine| {
        if let [intake, exhaust, boost, ..] = &mut engine.knobs[..] {
            (*intake, *exhaust, *boost) = (parts.intake_timing, parts.exhaust_timing, parts.boost);
        }
        swap(catalogue, engine, Kind::LongBlock, paint, |_, block| {
            for slot in 0..slots {
                swap(catalogue, block, Kind::EnginePart(slot), paint, |_, _| {
                    engine_part(slot)
                })?;
            }
            let does = Does::Block {
                family: parts.family,
                version: parts.version,
            };
            def(Kind::LongBlock, does)
        })?;
        for slot in 0..slots {
            swap(catalogue, engine, Kind::EnginePart(slot), paint, |_, _| {
                engine_part(slot)
            })?;
        }
        def(Kind::Engine, Does::Engine)
    })?;
    for slot in 0..slots {
        swap(
            catalogue,
            &mut car,
            Kind::EnginePart(slot),
            paint,
            |_, _| engine_part(slot),
        )?;
    }
    if repaint {
        paint_all(&mut car, paint);
    }
    Ok(car)
}

fn engine_does(parts: &Parts, slot: usize) -> Does {
    Does::EnginePart {
        slot,
        part: parts.chosen[slot],
    }
}

/// Every painted part of the tree in `paint`.
pub fn paint_all(item: &mut Item, paint: usize) {
    if item.paint.is_some() {
        item.paint = Some(paint);
    }
    for child in item.slots.iter_mut().flatten() {
        paint_all(child, paint);
    }
}

/// The knobs every chassis has (the brakes' force and balance, until there are brake
/// bias valves to have them).
pub fn chassis_knobs() -> Vec<Knob> {
    vec![
        knob(
            "Brake force",
            *BRAKE_FORCES.start(),
            *BRAKE_FORCES.end(),
            1.0,
        ),
        knob(
            "Brake balance",
            *BRAKE_BALANCES.start(),
            *BRAKE_BALANCES.end(),
            0.0,
        ),
    ]
}

/// A damper's knobs: its camber (degrees; its range is the kit's) and toe.
pub fn damper_knobs() -> Vec<Knob> {
    vec![
        knob("Camber", -6.0, 2.0, -1.0),
        knob("Toe", *TOES.start(), *TOES.end(), 0.0),
    ]
}

/// A tire's knob: its cold pressure.
pub fn tire_knobs() -> Vec<Knob> {
    vec![knob("Pressure", *PRESSURES.start(), *PRESSURES.end(), 2.2)]
}

/// An engine's knobs: the cam gears' timing (intake advanced, exhaust retarded) and the
/// wastegate's spring.
pub fn engine_knobs() -> Vec<Knob> {
    vec![
        knob("Intake cam timing", -10.0, 10.0, 0.0),
        knob("Exhaust cam timing", -10.0, 10.0, 0.0),
        knob("Boost", 0.3, 1.5, 0.6),
    ]
}

pub fn knob(name: &str, low: f32, high: f32, default: f32) -> Knob {
    Knob {
        name: name.into(),
        low,
        high,
        default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::builtin;

    #[test]
    fn the_trim_is_the_factory_car() {
        let catalogue = builtin();
        let trim = &catalogue.trims[0];
        let (build, parts) = trim.car.resolve(&catalogue).expect("a car");
        assert_eq!(build, CarBuild::factory(0));
        assert_eq!(parts, build.factory_engine(0));
    }

    #[test]
    fn the_trims_differ_in_more_than_the_engine() {
        let catalogue = builtin();
        let ids: Vec<&str> = catalogue.trims.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["vireo.1-6", "vireo.1-8"]);
        let rims = |car: &Item| -> Vec<String> {
            (car.fitted(&catalogue, Kind::Wheel))
                .flat_map(|wheel| wheel.fitted(&catalogue, Kind::Rim))
                .map(|rim| rim.def(&catalogue).id.clone())
                .collect()
        };
        let spoiler = |car: &Item| {
            (car.fitted(&catalogue, Kind::Body(Panel::RearSpoiler)))
                .next()
                .is_some()
        };
        let [base, sport] = [0, 1].map(|option| crate::car::CarSetup::factory(0, option));
        assert_eq!(rims(&base.car), ["rim.steel"; 4]);
        assert_eq!(rims(&sport.car), ["rim.alloy"; 4]);
        assert!(!spoiler(&base.car) && spoiler(&sport.car));
        assert!(!base.build.limited_slip && sport.build.limited_slip);
        assert_eq!(sport.build.wheels[0].rim.diameter, 15);
        assert_eq!(sport.engine.parts.hardware().name, "B18 1.8 four");
        // A shop's change keeps the steel rims steel.
        let mut build = base.build.clone();
        build.clutch = 1;
        let parts = base.engine.parts.clone();
        let changed = base.changed(build, parts, base.engine.build.clone());
        assert_eq!(rims(&changed.car), ["rim.steel"; 4]);
    }

    #[test]
    fn a_car_assembled_from_what_it_resolves_to_is_the_same() {
        let catalogue = builtin();
        let car = &catalogue.trims[0].car;
        let (build, parts) = car.resolve(&catalogue).unwrap();
        assert_eq!(
            assemble(&catalogue, Some(car), &build, &parts).as_ref(),
            Ok(car)
        );
        assert_eq!(assemble(&catalogue, None, &build, &parts).as_ref(), Ok(car));
    }

    #[test]
    fn changes_go_into_the_tree() {
        let catalogue = builtin();
        let car = &catalogue.trims[0].car;
        let (mut build, mut parts) = car.resolve(&catalogue).unwrap();
        build.clutch = 2;
        build.suspension = 1;
        build.camber = [-2.0, -1.5];
        build.wheels[1].compound = 3;
        build.bars = [2, 3];
        build.limited_slip = false;
        build.final_drive = 4.1;
        build.paint = 2;
        parts.chosen[crate::catalogue::TURBO] = 2;
        parts.boost = 0.9;
        parts.ecu = EcuModel::Programmable;
        let parts = build.install(parts).unwrap();
        let changed = assemble(&catalogue, Some(car), &build, &parts).unwrap();
        assert_eq!(changed.resolve(&catalogue), Ok((build, parts)));
        // Every painted part has the new paint.
        let painted: Vec<_> = (changed.all().into_iter())
            .filter_map(|item| item.paint)
            .collect();
        assert!(painted.len() > 10 && painted.iter().all(|&p| p == 2));
        // The parts not changed are the same items.
        let seats = |car: &Item| car.fitted(&catalogue, Kind::Seat(Seat::Front)).count();
        assert_eq!(seats(&changed), 2);
    }
}
