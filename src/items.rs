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

use crate::catalogue::{Bolting, Catalogue};
use crate::chassis::{
    BRAKE_BALANCES, BRAKE_FORCES, CarBuild, GearboxKind, Misfit, PRESSURES, TOES,
};
use crate::physics::engine::parts::{Effect, Parts};
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
    /// A four-door's rear doors (behind the front ones, `Body::rear_door`).
    RearDoor,
}

impl Panel {
    pub const ALL: [Panel; 12] = [
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
        Panel::RearDoor,
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
    /// A cylinder head with its valves and cams, made for an engine family; in a long
    /// block, the engine shop's work.
    Head,
    /// One of the catalogue's engine slots (`Catalogue::slots`, by its place).
    EnginePart(usize),
    /// An exhaust manifold, made for an engine family: it decides how the exhaust is
    /// gathered, and carries the turbos (if it has mounts for them).
    Manifold,
    /// A turbo, on one of a manifold's mounts.
    Turbo,
    /// An adapter plate, on the face of a gearbox that isn't transverse: it bolts the
    /// gearbox to an engine of another bellhousing pattern.
    Adapter,
}

/// What a part does, as far as the game's code knows kinds of parts.
#[derive(Clone, Debug, PartialEq)]
pub enum Does {
    /// The catalogue's chassis (by its place).
    Chassis(usize),
    /// A seat: only there (its mass counts).
    Nothing,
    /// A body panel, and what it does to the air.
    Body(Aero),
    /// A wheel: holds a rim and a tire.
    Wheel,
    /// A rim in any of the sizes there are (`Rim::all`, the item's `variant`).
    Rim,
    /// A tire of the catalogue's compound (by its place), in any of the sizes there are
    /// (`TireSize::all`, the item's `variant`).
    Tire(usize),
    Brake(crate::physics::brakes::Brake),
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
    /// The catalogue's ECU (by its place in `ecus`).
    Ecu(usize),
    Engine,
    /// A version of an engine family (both by their places in the catalogue).
    Block {
        family: usize,
        version: usize,
    },
    /// The cylinder head `head` of an engine family (both by their places in the
    /// catalogue: `Family::heads`).
    Head {
        family: usize,
        head: usize,
    },
    /// Part `part` of the catalogue's engine slot `slot`.
    EnginePart {
        slot: usize,
        part: usize,
    },
    /// The exhaust manifold `manifold` of an engine family (both by their places in the
    /// catalogue: `Family::manifolds`).
    Manifold {
        family: usize,
        manifold: usize,
    },
    /// The catalogue's adapter plate (by its place in `adapters`).
    Adapter(usize),
}

/// What a body part does to the air flowing round the car (see `CarSetup::aero`). The
/// chassis' own drag is with its stock body; a piece taken off adds its panel's
/// `Panel::missing_drag`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aero {
    /// Drag area it adds (m²; less than nothing: it smooths the flow).
    pub drag: f32,
    /// Downforce area (lift coefficient times area, m²) it gives over the front and the
    /// rear axle, with the body at its design ride height.
    pub downforce: [f32; 2],
    /// Ground effect: the share its downforce grows by per centimetre the body comes nearer
    /// the ground at its axle (a splitter, skirts, a diffuser; none for a wing).
    pub ground: f32,
    /// A wing whose angle is its first knob (degrees): `downforce` and `drag` are at
    /// `WING_REFERENCE` (see `Aero::at_angle`).
    pub wing: bool,
}

/// The angle a wing's numbers are given at, and the angle at which it gives no downforce
/// (degrees).
pub const WING_REFERENCE: f32 = 10.0;
pub const WING_ZERO_LIFT: f32 = -4.0;

impl Aero {
    /// What it does with its knobs at `knobs`: a wing's downforce goes with its angle of
    /// attack from where it lifts nothing, its drag partly with the square of that (what
    /// the downforce costs) and partly not (its profile).
    pub fn at(&self, knobs: &[f32]) -> Self {
        let Some(&angle) = knobs.first().filter(|_| self.wing) else {
            return *self;
        };
        let share = ((angle - WING_ZERO_LIFT) / (WING_REFERENCE - WING_ZERO_LIFT)).max(0.0);
        Self {
            drag: self.drag * (0.4 + 0.6 * share * share),
            downforce: self.downforce.map(|d| d * share),
            ..*self
        }
    }
}

impl Panel {
    /// Drag area a missing piece adds (m²): an open engine bay, a gap where a door was;
    /// mirrors only add drag, so without them there's less.
    pub fn missing_drag(self) -> f32 {
        match self {
            Panel::FrontBumper => 0.05,
            Panel::Bonnet => 0.04,
            Panel::Fender => 0.015,
            Panel::Door | Panel::RearDoor => 0.02,
            Panel::Mirror => -0.008,
            Panel::Hatch => 0.04,
            Panel::RearBumper => 0.02,
            Panel::FrontLip | Panel::SideSkirt | Panel::RearSpoiler | Panel::Diffuser => 0.0,
        }
    }
}

/// What wears on a part, as the simulation knows wear (`saved::Condition`): the part
/// carries it (`Item::wear`), so a part taken off keeps its wear and a new one is new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wear {
    /// Each cylinder's piston and rings (knock).
    Pistons,
    /// Each cylinder's connecting rod (fatigue).
    Rods,
    /// Each cylinder's valves (meeting the pistons when they float): in the head.
    Valves,
    /// The head gasket (overheating, pressure over what it seals).
    HeadGasket,
    /// The crank's bearings (hot oil).
    Bearings,
    /// The turbo (overspeed, a hot turbine).
    Turbo,
    /// A tire's tread, each rib's.
    Tread,
    /// A brake's pads.
    Pads,
    /// A clutch's friction plate.
    Clutch,
    /// A gearbox's gears and synchros, each forward gear's.
    Gears,
}

impl Wear {
    pub const ALL: [Wear; 10] = [
        Wear::Pistons,
        Wear::Rods,
        Wear::Valves,
        Wear::HeadGasket,
        Wear::Bearings,
        Wear::Turbo,
        Wear::Tread,
        Wear::Pads,
        Wear::Clutch,
        Wear::Gears,
    ];
}

/// The oversizes a block can be bored to (mm over its bore): a block's and its pistons'
/// sizes (`Item::variant`), which must match.
pub const OVERSIZES: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// What an oversize is called: "stock bore", "+0.50 mm".
pub fn oversize_label(step: usize) -> String {
    match OVERSIZES.get(step) {
        Some(&mm) if mm > 0.0 => format!("+{mm:.2} mm"),
        _ => "stock bore".into(),
    }
}

/// The most wear values an item carries (a cylinder's each, or a tire's ribs).
pub const MOST_WEAR: usize = 12;

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
    /// Another slot of the same part that must have a part for this one to (a lip goes on
    /// a bumper), by its place.
    pub needs: Option<usize>,
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
    /// The engines it's made for (families' names or versions' codes); none: any.
    pub engines: Vec<String>,
    /// The box it's packed in: length, width, height (m).
    pub size: [f32; 3],
    pub mass: f32,
    /// In cents.
    pub price: u32,
    /// Painted (in the catalogue's `paints`, each item its own).
    pub painted: bool,
    /// Stands for nothing fitted, in a slot the engine needs filled (no turbo): taking a
    /// part off puts it in, and it's never sold or carried.
    pub absent: bool,
    /// What wears on it, if anything the simulation wears.
    pub wear: Option<Wear>,
    /// Comes in the bore sizes (`OVERSIZES`): a block, and the pistons for it.
    pub bores: bool,
    /// What it needs another part of the car to provide (a Roots blower, a bonnet with a
    /// cut-out to stick through), and what it provides.
    pub needs: Vec<String>,
    pub provides: Vec<String>,
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

/// Parts sold as one, put together as the factory does (an engine with its long block
/// and bolt-ons): a shop's ware like a single part.
#[derive(Clone, Debug, PartialEq)]
pub struct Assembly {
    pub id: String,
    pub name: String,
    pub item: Item,
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
    /// How worn it is, as its definition's `wear` says (0 new, 1 failed; a value per
    /// cylinder or rib, trailing zeros left out: none, new).
    pub wear: Vec<f32>,
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
        if self.bores {
            return OVERSIZES.len();
        }
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
            wear: Vec::new(),
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
            let spring = self.nth(catalogue, Kind::Spring, 2 * axle)?;
            build.ride_height[axle] = spring.knobs.first().copied().unwrap_or(0.0);
        }
        if let Some(wing) = self
            .fitted(catalogue, Kind::Body(Panel::RearSpoiler))
            .next()
            && let Does::Body(Aero { wing: true, .. }) = wing.def(catalogue).does
        {
            build.wing = wing.knobs.first().copied().unwrap_or(0.0);
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
        let head = block.nth(catalogue, Kind::Head, 0)?;
        let Does::Head {
            family: head_family,
            head: h,
        } = head.def(catalogue).does
        else {
            return Err(Unfit("not a cylinder head".into()));
        };
        if head_family != family {
            return Err(Unfit("the cylinder head is made for another engine".into()));
        }
        let mut parts = Parts::of_version(catalogue, family, version);
        parts.head = h;
        parts.bore = block.variant.min(OVERSIZES.len() - 1);
        parts.ecu = ecu;
        parts.intake_timing = engine.knobs[0];
        parts.exhaust_timing = engine.knobs[1];
        parts.boost = engine.knobs[2];
        if let Some(why) = engine_misfit(catalogue, engine) {
            return Err(Unfit(why));
        }
        let manifold = engine.nth(catalogue, Kind::Manifold, 0)?;
        let Does::Manifold {
            family: made_for,
            manifold: m,
        } = manifold.def(catalogue).does
        else {
            return Err(Unfit("not an exhaust manifold".into()));
        };
        if made_for != family {
            return Err(Unfit(
                "the exhaust manifold is made for another engine".into(),
            ));
        }
        parts.manifold = m;
        // The turbos on its mounts (all the same, `engine_misfit` saw to that).
        let turbo = (manifold.fitted(catalogue, Kind::Turbo).next())
            .map(|turbo| &turbo.def(catalogue).does);
        // Engine parts sit in the long block, its head, on the engine and in the car, turbos on the
        // manifold's mounts (none: the slot's first part, which stands for none).
        for (slot, chosen) in parts.chosen.iter_mut().enumerate() {
            if catalogue.slots[slot].mounted {
                *chosen = match turbo {
                    Some(&Does::EnginePart { slot: s, part }) if s == slot => part,
                    _ => 0,
                };
                continue;
            }
            let kind = Kind::EnginePart(slot);
            let part = [block, head, engine, self]
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

/// Why `item`'s tree breaks the engines' rules, if it does: a part in an engine of another
/// family than it's made for (`PartDef::engines`), a manifold whose turbo mounts aren't
/// all empty or all the same turbo set alike, an engine with a turbo and a supercharger.
pub fn engine_misfit(catalogue: &Catalogue, item: &Item) -> Option<String> {
    let def = catalogue.defs.get(item.def)?;
    let defs = |item: &Item| catalogue.defs.get(item.def);
    if def.kind == Kind::Manifold {
        let mounts: Vec<&Option<Item>> = (def.slots.iter().zip(&item.slots))
            .filter(|(slot, _)| slot.kind == Kind::Turbo)
            .map(|(_, turbo)| turbo)
            .collect();
        let turbos: Vec<&Item> = mounts.iter().filter_map(|turbo| turbo.as_ref()).collect();
        if let Some(first) = turbos.first() {
            if turbos.len() < mounts.len() {
                return Some(format!("The {} takes a turbo on every mount", def.name));
            }
            if (turbos.iter()).any(|turbo| turbo.def != first.def || turbo.knobs != first.knobs) {
                return Some(format!("The turbos on the {} must be the same", def.name));
            }
        }
    }
    if def.kind == Kind::Engine {
        let block = (def.slots.iter().zip(&item.slots))
            .find(|(slot, _)| slot.kind == Kind::LongBlock)
            .and_then(|(_, block)| block.as_ref());
        if let Some(Does::Block { family, version }) = block.and_then(defs).map(|d| &d.does)
            && let Some(family) = catalogue.families.get(*family)
            && let Some(version) = family.versions.get(*version)
        {
            let fits = |engines: &[String]| {
                engines.is_empty()
                    || (engines.iter()).any(|e| *e == family.name || *e == version.code)
            };
            if let Some(part) = (item.all().into_iter())
                .filter_map(defs)
                .find(|part| !fits(&part.engines))
            {
                return Some(format!(
                    "The {} isn't made for the {} {}",
                    part.name.to_lowercase(),
                    version.code,
                    version.name
                ));
            }
        }
        let parts = item.all();
        let turbo = (parts.iter())
            .filter_map(|part| defs(part))
            .any(|part| part.kind == Kind::Turbo);
        let supercharger = (parts.iter()).filter_map(|part| defs(part)).any(|part| {
            let Does::EnginePart { slot, part } = part.does else {
                return false;
            };
            (catalogue.slots.get(slot).and_then(|s| s.parts.get(part))).is_some_and(|part| {
                (part.effects.iter()).any(|effect| matches!(effect, Effect::Supercharger(_)))
            })
        });
        if turbo && supercharger {
            return Some("A turbo and a supercharger don't go together".into());
        }
    }
    (item.slots.iter().flatten()).find_map(|child| engine_misfit(catalogue, child))
}

/// Why a car's gearbox doesn't bolt to its engine (`drivetrain_misfit`), and what would
/// make it.
#[derive(Clone, Debug, PartialEq)]
pub struct Misbolt {
    pub why: String,
    pub fix: Option<Fix>,
}

/// What makes a gearbox bolt to an engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// This adapter plate (in the catalogue's `adapters`) on the gearbox.
    Fit(usize),
    /// The adapter plate off: the two bolt straight together.
    Remove,
}

/// Why `car`'s (a car's tree) gearbox doesn't bolt to its engine, if it doesn't (see
/// `Catalogue::bolting`): another bellhousing pattern and no adapter plate for the two on
/// the gearbox, or one that isn't needed. A car without an engine or gearbox has nothing
/// to bolt.
pub fn drivetrain_misfit(catalogue: &Catalogue, car: &Item) -> Option<Misbolt> {
    let gearbox = car.fitted(catalogue, Kind::Gearbox).next()?;
    let Does::Gearbox(g) = gearbox.def(catalogue).does else {
        return None;
    };
    let engine = car.fitted(catalogue, Kind::Engine).next()?;
    let block = engine.fitted(catalogue, Kind::LongBlock).next()?;
    let Does::Block { family, .. } = block.def(catalogue).does else {
        return None;
    };
    let adapter = (gearbox.fitted(catalogue, Kind::Adapter).next()).and_then(|plate| {
        match plate.def(catalogue).does {
            Does::Adapter(a) => Some(a),
            _ => None,
        }
    });
    let (made, engine) = (&catalogue.gearboxes[g], &catalogue.families[family]);
    let misbolt = |why: String, fix| Some(Misbolt { why, fix });
    match (catalogue.bolting(g, family), adapter) {
        (Bolting::Direct, None) => None,
        (Bolting::Direct, Some(_)) => misbolt(
            format!(
                "The {} bolts straight to the {}: the adapter plate comes off",
                made.name, engine.name
            ),
            Some(Fix::Remove),
        ),
        (Bolting::Adapter(a), Some(fitted)) if a == fitted => None,
        (Bolting::Adapter(a), _) => misbolt(
            format!(
                "The {} bolts to the {} only through the {}",
                made.name,
                engine.name,
                catalogue.adapters[a].name.to_lowercase()
            ),
            Some(Fix::Fit(a)),
        ),
        (Bolting::Never, _) => misbolt(
            if made.kind == GearboxKind::Transverse {
                format!(
                    "The {} has a {} bellhousing and doesn't go on the {}: a transverse \
                     gearbox stays with its engine family (they don't all turn the same way)",
                    made.name, made.bellhousing, engine.name
                )
            } else {
                format!(
                    "The {} has a {} bellhousing, and no adapter plate bolts it to the {}",
                    made.name, made.bellhousing, engine.name
                )
            },
            None,
        ),
    }
}

/// What a Roots blower needs (and a bonnet with a cut-out provides, or no bonnet at all).
pub const BONNET_CUT_OUT: &str = "bonnet with a cut-out";

/// What cam phasers and a switching cam need (and a long block with its cams in the head
/// provides).
pub const OVERHEAD_CAMS: &str = "head with overhead cams";

/// A part of `car` (a car's tree) that needs what no part of it provides: the part's name
/// and what it needs. A car without a bonnet has room for whatever sticks through one.
pub fn unmet_need<'a>(catalogue: &'a Catalogue, car: &Item) -> Option<(&'a str, &'a str)> {
    let all = car.all();
    let defs = || all.iter().filter_map(|item| catalogue.defs.get(item.def));
    let no_bonnet = (car.fitted(catalogue, Kind::Body(Panel::Bonnet)).next()).is_none();
    let provided = |need: &String| {
        (no_bonnet && need == BONNET_CUT_OUT) || defs().any(|def| def.provides.contains(need))
    };
    defs().find_map(|def| {
        let need = def.needs.iter().find(|need| !provided(need))?;
        Some((def.name.as_str(), need.as_str()))
    })
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
            Panel::Fender | Panel::Door | Panel::RearDoor | Panel::Mirror | Panel::SideSkirt => {
                side
            }
            _ => Side::Middle,
        };
        if let Some(item) = item {
            pieces.push((panel, side, color(item.paint)));
        }
    }
    let wing =
        (car.fitted(catalogue, Kind::Body(Panel::RearSpoiler)).next()).is_some_and(|spoiler| {
            matches!(
                spoiler.def(catalogue).does,
                Does::Body(Aero { wing: true, .. })
            )
        });
    crate::body::Dress {
        shell: color(car.paint),
        pieces,
        wing,
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
    swap(catalogue, &mut car, Kind::Spring, paint, |n, spring| {
        set_knob(catalogue, spring, build.ride_height[n / 2]);
        def(Kind::Spring, Does::Spring(build.suspension))
    })?;
    // A wing set to the build's angle.
    let slots = car.def(catalogue).slots.clone();
    for (slot, item) in slots.iter().zip(&mut car.slots) {
        if let (Kind::Body(Panel::RearSpoiler), Some(item)) = (slot.kind, item)
            && matches!(
                item.def(catalogue).does,
                Does::Body(Aero { wing: true, .. })
            )
        {
            set_knob(catalogue, item, build.wing);
        }
    }
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
            block.variant = parts.bore;
            swap(catalogue, block, Kind::Head, paint, |_, head| {
                for slot in 0..slots {
                    swap(catalogue, head, Kind::EnginePart(slot), paint, |_, _| {
                        engine_part(slot)
                    })?;
                }
                let does = Does::Head {
                    family: parts.family,
                    head: parts.head,
                };
                def(Kind::Head, does)
            })?;
            for slot in 0..slots {
                swap(
                    catalogue,
                    block,
                    Kind::EnginePart(slot),
                    paint,
                    |_, part| {
                        // Pistons the block's size.
                        let d = engine_part(slot)?;
                        part.variant = if catalogue.defs[d].bores {
                            parts.bore
                        } else {
                            0
                        };
                        Ok(d)
                    },
                )?;
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
        swap(catalogue, engine, Kind::Manifold, paint, |_, _| {
            let does = Does::Manifold {
                family: parts.family,
                manifold: parts.manifold,
            };
            def(Kind::Manifold, does)
        })?;
        // The turbo on every one of its mounts (the same items if they're the same), or
        // on none.
        let turbo = (0..slots)
            .find(|&slot| catalogue.slots[slot].mounted && parts.chosen[slot] != 0)
            .map(|slot| def(Kind::Turbo, engine_does(parts, slot)))
            .transpose()?;
        let holders = engine.def(catalogue).slots.clone();
        for (slot, manifold) in holders.iter().zip(&mut engine.slots) {
            let (Kind::Manifold, Some(manifold)) = (slot.kind, manifold) else {
                continue;
            };
            let mounts = manifold.def(catalogue).slots.clone();
            let mut mounted = false;
            for (slot, place) in mounts.iter().zip(&mut manifold.slots) {
                if slot.kind != Kind::Turbo {
                    continue;
                }
                mounted = true;
                *place = match turbo {
                    Some(d) if place.as_ref().is_some_and(|old| old.def == d) => place.take(),
                    Some(d) => Some(Item::new(catalogue, d, paint)),
                    None => None,
                };
            }
            if turbo.is_some() && !mounted {
                return Err(Unfit(format!(
                    "the {} has no mounts for a turbo",
                    manifold.def(catalogue).name.to_lowercase()
                )));
            }
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

/// `item`'s first knob, if it has one, set to `value` (within its range).
fn set_knob(catalogue: &Catalogue, item: &mut Item, value: f32) {
    if let (Some(knob), Some(def)) = (
        item.knobs.first_mut(),
        catalogue
            .defs
            .get(item.def)
            .and_then(|def| def.knobs.first()),
    ) {
        *knob = value.clamp(def.low, def.high);
    }
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

/// A spring's knob: the ride height its perch sets (mm over the design height; less than
/// nothing lowers the body), within `low`..`high`, from `default`.
pub fn spring_knobs(low: f32, high: f32, default: f32) -> Vec<Knob> {
    vec![knob("Ride height", low, high, default)]
}

/// A wing's knob: its angle of attack (degrees).
pub fn wing_knobs() -> Vec<Knob> {
    vec![knob("Wing angle", 0.0, 16.0, 6.0)]
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
        assert_eq!(
            ids,
            [
                "vireo.1-6",
                "vireo.2-0",
                "vireo.1-6r",
                "merrow.r80",
                "tessel.2-0",
                "tessel.2-0r",
                "aldine.2-0",
                "aldine.2-4",
                "bothy.2-4",
            ]
        );
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
        assert_eq!(sport.engine.parts.hardware().name, "B20 2.0 SOHC four");
        // A shop's change keeps the steel rims steel.
        let mut build = base.build.clone();
        build.clutch = 1;
        let parts = base.engine.parts.clone();
        let changed = base.changed(build, parts, base.engine.build.clone());
        assert_eq!(rims(&changed.car), ["rim.steel"; 4]);
    }

    #[test]
    fn the_roadster_is_its_trim() {
        let catalogue = builtin();
        let trim = (catalogue.trims.iter())
            .find(|trim| trim.id == "merrow.r80")
            .expect("the roadster's trim");
        let (build, parts) = trim.car.resolve(&catalogue).expect("a car");
        assert_eq!(parts.hardware().name, "R80 8.0 V10");
        assert_ne!(build.wheels[0].tire, build.wheels[1].tire);
        assert!(build.limited_slip);
        assert_eq!(
            assemble(&catalogue, None, &build, &parts).as_ref(),
            Ok(&trim.car)
        );
        // Two seats and a boot lid; its engine is sold whole.
        let slots = &trim.car.def(&catalogue).slots;
        assert!(
            slots
                .iter()
                .all(|slot| slot.kind != Kind::Seat(Seat::RearBench))
        );
        assert!(slots.iter().any(|slot| slot.name == "Boot lid"));
        assert!(catalogue.assemblies.iter().any(|a| a.id == "engine.r80"));
        // The hatch's transverse gearboxes aren't made for it.
        let chassis = &catalogue.chassis[build.chassis].name;
        let fitting: Vec<&str> = (catalogue.defs.iter())
            .filter(|def| def.kind == Kind::Gearbox && crate::fitting::made_for(def, chassis))
            .map(|def| def.id.as_str())
            .collect();
        assert_eq!(fitting, ["gearbox.6-speed-heavy-duty-inline"]);
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
        parts.manifold = parts.turbo_manifold().unwrap();
        parts.boost = 0.9;
        parts.ecu = (catalogue.ecus.iter())
            .position(|ecu| ecu.id == "ecu.programmable-4")
            .unwrap();
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
