//! Changing a car at shops, as the server does it: a player's game asks for `Work`s (fit
//! a part into a slot, take one off, set a part's knob, paint a part, load a tune, make or
//! take apart a wheel), the server checks each against the rules (which shop does what,
//! what the shop sells, what fits) and does them all or none (`Bench`), and tells the
//! player the car it has now. The games use the same rules to show what can be done.
//!
//! - Car workshops and tuning shops fit any part but a long block's insides (the engine
//!   shop's) and a wheel's rim and tire (the tire shop's); engines come out whole, and
//!   only for another. They set the final drive's ratio.
//! - Engine shops work only on long blocks: a long block into an engine, what goes inside
//!   one, rebores (a block bored over takes pistons of its size), and whole engines swapped
//!   (one kept there for the player). They work on the car's and on an engine or long
//!   block brought in the trunk or held there (a path starting `IN_TRUNK` or `HELD`); they
//!   hold one engine and one long block for a player at most.
//! - Tire shops fit wheels, and rims and tires on the car's wheels; they make loose wheels
//!   of a rim and a tire, and take them apart.
//! - Alignment shops set the chassis', dampers' and tires' knobs (brakes, camber and toe,
//!   pressure), dynos the engine's (cam timing, boost) and the ECU's tune, paint shops
//!   paint any painted part.
//! - A new part is one the shop sells (its list in the shops file), made for the car's
//!   chassis (and an engine's part for the engine's family: exhaust manifolds); parts
//!   brought come from the trunk or what the shop holds for the player.
//! - Turbos go on an exhaust manifold's mounts: on every mount the same turbo, or none
//!   (`mounts` are the paths to fill at once); a turbo and a supercharger exclude each
//!   other (`items::engine_misfit`). Another manifold takes the turbos over only onto as
//!   many mounts; else they stay on the old one.
//! - The gearbox bolts to the engine (`items::drivetrain_misfit`, `Catalogue::bolting`):
//!   of the same bellhousing pattern, or an inline gearbox through the adapter plate made
//!   for the two (on the gearbox, its slot; engine shops fit them too, as they swap
//!   engines). Works that change both go in one request.
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

/// Where a slot is: the places of the slots from the car down (the car itself: none), or
/// from a loose part: `[IN_TRUNK, i, …]` from the trunk's part `i`, `[HELD, i, …]` from the
/// part `i` the shop holds for the player.
pub type Path = Vec<u8>;

/// A path's first place for a loose part in the trunk, and one held at the shop.
pub const IN_TRUNK: u8 = 0xfe;
pub const HELD: u8 = 0xff;

/// The longest path (a loose part's two places, then the slots down).
const LONGEST_PATH: usize = 10;

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
    /// The block at `at` bored over to oversize `size` (`items::OVERSIZES`): it needs
    /// pistons of that size then (fitted in the same request).
    Rebore { at: Path, size: u8 },
}

/// The kind of slot at `path` in `car` (or a loose part), and the kind of what holds it
/// (none: it's a car's own).
pub fn slot_kinds(catalogue: &Catalogue, car: &Item, path: &[u8]) -> Option<(Kind, Option<Kind>)> {
    let (&last, rest) = path.split_last()?;
    let mut holder = car;
    for &place in rest {
        holder = holder.slots.get(usize::from(place))?.as_ref()?;
    }
    let slot = holder.def(catalogue).slots.get(usize::from(last))?;
    let holder_kind = Some(holder.def(catalogue).kind).filter(|&kind| kind != Kind::Chassis);
    Some((slot.kind, holder_kind))
}

/// The tree `path` is in (the car, or a loose part in the trunk or held at the shop) and
/// the rest of the path, from that tree down.
pub fn root_of<'a, 'p>(
    car: &'a Item,
    cargo: &'a [Item],
    held: &'a [Item],
    path: &'p [u8],
) -> Option<(&'a Item, &'p [u8])> {
    match path {
        [IN_TRUNK, i, rest @ ..] => Some((cargo.get(usize::from(*i))?, rest)),
        [HELD, i, rest @ ..] => Some((held.get(usize::from(*i))?, rest)),
        _ => Some((car, path)),
    }
}

/// Whether `path` is in a loose part.
pub fn loose(path: &[u8]) -> bool {
    matches!(path.first(), Some(&IN_TRUNK | &HELD))
}

/// `slot_kinds` and `item_at` for a path that may be in a loose part.
pub fn slot_kinds_in(
    catalogue: &Catalogue,
    (car, cargo, held): (&Item, &[Item], &[Item]),
    path: &[u8],
) -> Option<(Kind, Option<Kind>)> {
    let (root, rest) = root_of(car, cargo, held, path)?;
    slot_kinds(catalogue, root, rest)
}

pub fn item_in<'a>(
    (car, cargo, held): (&'a Item, &'a [Item], &'a [Item]),
    path: &[u8],
) -> Option<&'a Item> {
    let (root, rest) = root_of(car, cargo, held, path)?;
    item_at(root, rest)
}

/// Why a block in `item`'s tree has pistons of another size than its bore, if one has.
pub fn bores_mismatch(catalogue: &Catalogue, item: &Item) -> Option<String> {
    let def = item.def(catalogue);
    if def.kind == Kind::LongBlock && def.bores {
        for part in item.slots.iter().flatten() {
            let made = part.def(catalogue);
            if made.bores && part.variant != item.variant {
                return Some(format!(
                    "The {} are for a {}; the block has a {}",
                    made.name.to_lowercase(),
                    bore_name(part.variant),
                    bore_name(item.variant),
                ));
            }
        }
    }
    (item.slots.iter().flatten()).find_map(|part| bores_mismatch(catalogue, part))
}

/// A bore size, for people: "stock bore", "bore +0.50 mm".
pub fn bore_name(size: usize) -> String {
    match crate::items::OVERSIZES.get(size) {
        Some(&mm) if mm > 0.0 => format!("bore +{mm:.2} mm"),
        _ => "stock bore".into(),
    }
}

/// Whether a shop holding `held` for a player has more engines or long blocks than an
/// engine shop keeps (one of each).
fn engines_over(catalogue: &Catalogue, held: &[Item]) -> bool {
    [Kind::Engine, Kind::LongBlock].into_iter().any(|kind| {
        (held.iter())
            .filter(|item| item.def(catalogue).kind == kind)
            .count()
            > 1
    })
}

/// Whether an engine shop holding `held` for a player has room for `part` too (one engine
/// and one long block each at most).
pub fn engine_shop_takes(catalogue: &Catalogue, held: &[Item], part: &Item) -> bool {
    let mut all = held.to_vec();
    all.push(part.clone());
    !engines_over(catalogue, &all)
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
        // A tunnel kit is welded into the floor at car workshops.
        ShopKind::Car | ShopKind::Tuning if slot == Kind::Tunnel => kind == ShopKind::Car,
        ShopKind::Car | ShopKind::Tuning => {
            !matches!(
                slot,
                Kind::Rim | Kind::Tire | Kind::LongBlock | Kind::Chassis
            ) && !in_long_block(holder)
        }
        ShopKind::Tire => {
            slot == Kind::Wheel && holder.is_none()
                || matches!(slot, Kind::Rim | Kind::Tire) && holder == Some(Kind::Wheel)
        }
        ShopKind::Engine => {
            slot == Kind::LongBlock
                || in_long_block(holder)
                || slot == Kind::Engine && matches!(holder, None | Some(Kind::RearSubframe))
                || slot == Kind::Adapter
        }
        _ => false,
    }
}

/// Whether a part held by one of `holder` kind is inside a long block: in it, or in its
/// cylinder head.
fn in_long_block(holder: Option<Kind>) -> bool {
    matches!(holder, Some(Kind::LongBlock | Kind::Head))
}

/// Which kind of shop sets the knobs of a part of `kind`.
pub fn knobs_at(kind: Kind, shop: ShopKind) -> bool {
    match kind {
        Kind::Chassis | Kind::Damper | Kind::Tire | Kind::Spring | Kind::Body(_) => {
            shop == ShopKind::Alignment
        }
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

/// The engine family and version of the engine on the way down `path` (in the car or a
/// loose part), if there's one with a long block.
pub fn engine_on(
    catalogue: &Catalogue,
    tree: (&Item, &[Item], &[Item]),
    path: &[u8],
) -> Option<(usize, usize)> {
    let (root, rest) = root_of(tree.0, tree.1, tree.2, path)?;
    let engine = (0..=rest.len())
        .filter_map(|n| item_at(root, &rest[..n]))
        .find(|item| item.def(catalogue).kind == Kind::Engine)?;
    match engine
        .fitted(catalogue, Kind::LongBlock)
        .next()?
        .def(catalogue)
        .does
    {
        Does::Block { family, version } => Some((family, version)),
        _ => None,
    }
}

/// Whether `def` is made for an engine of `family`'s `version` (or any; parts with no
/// engine named are made for any, and anything goes where there's no engine).
pub fn made_for_engine(
    catalogue: &Catalogue,
    def: &PartDef,
    engine: Option<(usize, usize)>,
) -> bool {
    let Some((family, version)) = engine else {
        return true;
    };
    let Some(family) = catalogue.families.get(family) else {
        return true;
    };
    let code = family.versions.get(version).map(|v| v.code.as_str());
    def.engines.is_empty()
        || (def.engines.iter()).any(|e| *e == family.name || Some(e.as_str()) == code)
}

/// The turbo mounts of the manifold holding the mount at `path`, if it's a mount: a shop
/// fits a turbo on all of them at once.
pub fn mounts(catalogue: &Catalogue, tree: (&Item, &[Item], &[Item]), path: &[u8]) -> Vec<Path> {
    let Some((Kind::Turbo, _)) = slot_kinds_in(catalogue, tree, path) else {
        return Vec::new();
    };
    let (holder_path, _) = path.split_at(path.len() - 1);
    let Some(manifold) = item_in(tree, holder_path) else {
        return Vec::new();
    };
    (manifold.def(catalogue).slots.iter().enumerate())
        .filter(|(_, slot)| slot.kind == Kind::Turbo)
        .map(|(i, _)| {
            let mut mount = holder_path.to_vec();
            mount.push(i as u8);
            mount
        })
        .collect()
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

/// What of `old`, coming off, goes into `part`'s empty slots in its place: what's in
/// slots of the same kind (a gearbox's final drive and differential, its adapter plate).
/// Turbos move to another manifold only all together, onto as many mounts; else they stay
/// on the old one.
fn take_over(catalogue: &Catalogue, part: &mut Item, old: &mut Item) {
    let slots = &catalogue.defs[part.def].slots;
    let old_slots = &catalogue.defs[old.def].slots;
    let mounts = |slots: &[crate::items::SlotDef], items: &[Option<Item>]| {
        (slots.iter().zip(items))
            .filter(|(slot, item)| slot.kind == Kind::Turbo && item.is_some())
            .count()
    };
    let all_mounts = slots.iter().filter(|s| s.kind == Kind::Turbo).count();
    let turbos_move = mounts(old_slots, &old.slots) == all_mounts;
    for (slot, item) in slots.iter().zip(&mut part.slots) {
        if item.is_some() || slot.kind == Kind::Turbo && !turbos_move {
            continue;
        }
        let moved = (old_slots.iter().zip(&mut old.slots))
            .find(|(s, i)| s.kind == slot.kind && i.is_some())
            .and_then(|(_, i)| i.take());
        *item = moved;
    }
}

/// `car` (a car's tree) with `part` fitted at `path` in it as a shop fits it (what it
/// takes over from the part coming off included), for showing what that would make; none
/// if there's no such place.
pub fn with_fitted(catalogue: &Catalogue, car: &Item, path: &[u8], mut part: Item) -> Option<Item> {
    let mut car = car.clone();
    if let [at] = *path {
        convert(catalogue, &mut car, usize::from(at), &mut part);
    }
    let place = slot_mut(&mut car, path)?;
    if let Some(mut old) = place.take() {
        take_over(catalogue, &mut part, &mut old);
    }
    *place = Some(part);
    Some(car)
}

/// The path of the adapter plate's slot on `car`'s gearbox (where the engine is: see
/// `items::powertrain`), if it has one.
pub fn adapter_slot(catalogue: &Catalogue, car: &Item) -> Option<Path> {
    let mut path = Path::new();
    let holder = match crate::items::rear_engine(catalogue, car) {
        Some(at) => {
            path.push(at as u8);
            car.slots.get(at)?.as_ref()?
        }
        None => car,
    };
    let slots = &holder.def(catalogue).slots;
    let g = slots.iter().position(|slot| slot.kind == Kind::Gearbox)?;
    let gearbox = holder.slots.get(g)?.as_ref()?;
    let a = (gearbox.def(catalogue).slots.iter()).position(|slot| slot.kind == Kind::Adapter)?;
    path.extend([g as u8, a as u8]);
    Some(path)
}

/// Whether `item` is an engine subframe (it has a bay for the engine, see
/// `items::powertrain`).
pub fn engine_bay(catalogue: &Catalogue, item: &Item) -> bool {
    matches!(item.def(catalogue).does, Does::EngineSubframe(_))
}

/// What fitting `part` into `car`'s slot `at` moves, as a shop does it, if it's a rear
/// subframe: an engine subframe in place of one without a bay takes the car's engine,
/// gearbox and front drive shafts into its slots (the front-drive unit moved back as it
/// is) and the rear bench comes out (its place is the bay's); a subframe without a bay in
/// place of an engine subframe gives them back to the car's empty slots. Returns what
/// came off besides the old subframe (the rear bench).
pub fn convert(catalogue: &Catalogue, car: &mut Item, at: usize, part: &mut Item) -> Vec<Item> {
    const MOVED: [Kind; 3] = [Kind::Engine, Kind::Gearbox, Kind::DriveShafts];
    let mut off = Vec::new();
    let is_subframe =
        (car.def(catalogue).slots.get(at)).is_some_and(|s| s.kind == Kind::RearSubframe);
    if !is_subframe || part.def(catalogue).kind != Kind::RearSubframe {
        return off;
    }
    let Some(mut old) = car.slots.get_mut(at).and_then(Option::take) else {
        return off;
    };
    // The first fitted part of `kind` in `from`, into the first empty slot of its kind in
    // `to` (if there's one).
    fn shift(catalogue: &Catalogue, from: &mut Item, to: &mut Item, kind: Kind) {
        let empty = (catalogue.defs[to.def].slots.iter().zip(&to.slots))
            .position(|(slot, item)| slot.kind == kind && item.is_none());
        let full = (catalogue.defs[from.def].slots.iter().zip(&from.slots))
            .position(|(slot, item)| slot.kind == kind && item.is_some());
        if let (Some(empty), Some(full)) = (empty, full) {
            to.slots[empty] = from.slots[full].take();
        }
    }
    match (engine_bay(catalogue, &old), engine_bay(catalogue, part)) {
        (false, true) => {
            for kind in MOVED {
                shift(catalogue, car, part, kind);
            }
            let slots = car.def(catalogue).slots.clone();
            for (slot, item) in slots.iter().zip(&mut car.slots) {
                if slot.kind == Kind::Seat(crate::items::Seat::RearBench) {
                    off.extend(item.take());
                }
            }
        }
        (true, false) => {
            for kind in MOVED {
                shift(catalogue, &mut old, car, kind);
            }
        }
        _ => {}
    }
    car.slots[at] = Some(old);
    off
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
                // A part brought from where the part fitted into lies: the places after
                // it move up.
                let slots = (slots.iter())
                    .map(|path| shifted(path, supply))
                    .collect::<Result<Vec<_>, _>>()?;
                let part = self.take(supply)?;
                for path in &slots {
                    self.fit(path, part.clone())?;
                }
            }
            Work::Remove { slot } => {
                let (slot_kind, holder) = self.kinds(slot).ok_or("No such place on the car")?;
                if !fits_into(kind, slot_kind, holder) {
                    return Err(format!("A {} doesn't do that", kind.title().to_lowercase()));
                }
                // A loose part may lack what it needs to run.
                let required =
                    !loose(slot) && self.slot_def(slot).is_some_and(|slot| slot.required);
                let nothing =
                    (catalogue.defs.iter()).position(|def| def.absent && def.kind == slot_kind);
                let (tree, rest) = self.tree_mut(slot).ok_or("No such place on the car")?;
                let place = slot_mut(tree, rest).ok_or("No such place on the car")?;
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
                let fitted = self.car.fitted(catalogue, Kind::Ecu).next();
                if let Some(Does::Ecu(e)) = fitted.map(|ecu| &ecu.def(catalogue).does)
                    && let Some(def) = catalogue.ecus.get(*e)
                    && def.model.locked
                {
                    return Err(format!("The {} is locked to its own maps", def.name));
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
            Work::Rebore { at, size } => {
                if kind != ShopKind::Engine {
                    return Err("Blocks are bored at engine shops".into());
                }
                let size = usize::from(*size);
                let (tree, rest) = self.tree_mut(at).ok_or("Nothing's there")?;
                let block = item_mut(tree, rest).ok_or("Nothing's there")?;
                let def = &catalogue.defs[block.def];
                if def.kind != Kind::LongBlock || !def.bores {
                    return Err("That isn't a block to bore".into());
                }
                if size >= crate::items::OVERSIZES.len() {
                    return Err("No such bore".into());
                }
                if size <= block.variant {
                    return Err("A block is only ever bored bigger".into());
                }
                block.variant = size;
            }
        }
        Ok(())
    }

    /// The tree `path` is in (the car, or a loose part) and the rest of the path from it.
    fn tree_mut<'p>(&mut self, path: &'p [u8]) -> Option<(&mut Item, &'p [u8])> {
        match path {
            [IN_TRUNK, i, rest @ ..] => Some((self.cargo.get_mut(usize::from(*i))?, rest)),
            [HELD, i, rest @ ..] => Some((self.held.get_mut(usize::from(*i))?, rest)),
            _ => Some((&mut self.car, path)),
        }
    }

    fn kinds(&self, path: &[u8]) -> Option<(Kind, Option<Kind>)> {
        slot_kinds_in(self.catalogue, (&self.car, &self.cargo, &self.held), path)
    }

    fn slot_def(&self, path: &[u8]) -> Option<&crate::items::SlotDef> {
        let (root, path) = root_of(&self.car, &self.cargo, &self.held, path)?;
        let (&last, rest) = path.split_last()?;
        let holder = item_at(root, rest)?;
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
                if made.kind == Kind::LongBlock && variant != 0 {
                    return Err("A new block comes with the stock bore".into());
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
        let (slot_kind, holder) = self.kinds(path).ok_or("No such place on the car")?;
        let kind = self.shop.kind;
        if !fits_into(kind, slot_kind, holder) {
            return Err(match slot_kind {
                Kind::Rim | Kind::Tire => "Rims and tires are fitted at tire shops".into(),
                Kind::LongBlock => "Long blocks are the engine shop's work".into(),
                Kind::Head => "Cylinder heads are the engine shop's work".into(),
                _ if in_long_block(holder) => {
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
        // An engine subframe in or out moves the engine, gearbox and drive shafts with it:
        // a tuning shop's conversion.
        if let [at] = *path {
            let old = item_at(&self.car, path).is_some_and(|old| engine_bay(catalogue, old));
            if (old || engine_bay(catalogue, &part)) && kind != ShopKind::Tuning {
                return Err("Moving the engine to the rear or back is a tuning shop's work".into());
            }
            let off = convert(catalogue, &mut self.car, usize::from(at), &mut part);
            self.off.extend(off);
        }
        let (tree, rest) = self.tree_mut(path).ok_or("No such place on the car")?;
        let place = slot_mut(tree, rest).ok_or("No such place on the car")?;
        let old = place.take().map(|mut old| {
            take_over(catalogue, &mut part, &mut old);
            old
        });
        *place = Some(part);
        self.off.extend(old);
        Ok(())
    }

    /// Whether the car that came of the works can be driven, and if so the car with what
    /// came off put away.
    pub fn finish(mut self) -> Result<Done, String> {
        let catalogue = self.catalogue;
        if let Some(why) = needs_missing(catalogue, &self.car) {
            return Err(why);
        }
        let mut all = std::iter::once(&self.car)
            .chain(&self.cargo)
            .chain(&self.held);
        if let Some(why) = all.find_map(|item| crate::items::engine_misfit(catalogue, item)) {
            return Err(why);
        }
        if let Some(why) = crate::items::bay_problem(catalogue, &self.car) {
            return Err(why);
        }
        if !self.car.fine(catalogue) {
            return Err("That doesn't make a car".into());
        }
        if let Some(misbolt) = crate::items::drivetrain_misfit(catalogue, &self.car) {
            return Err(misbolt.why);
        }
        let trees = std::iter::once(&self.car)
            .chain(&self.cargo)
            .chain(&self.held);
        if let Some(why) = trees
            .clone()
            .find_map(|item| bores_mismatch(catalogue, item))
        {
            return Err(why);
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
        // Every brake inside its wheel's rim.
        let wheels: Vec<_> = (self.car.fitted(catalogue, Kind::Wheel))
            .filter_map(|wheel| wheel_of(catalogue, wheel))
            .collect();
        for (n, brake) in self.car.fitted(catalogue, Kind::Brake).enumerate() {
            if let (Does::Brake(brake), Some(wheel)) = (&brake.def(catalogue).does, wheels.get(n))
                && wheel.rim.diameter < brake.min_rim
            {
                return Err(format!(
                    "{}: the brake needs at least {}\" rims",
                    crate::items::CORNERS[n.min(3)],
                    brake.min_rim
                ));
            }
        }
        let ecu = parts.ecu_def();
        let cylinders = parts.hardware().cylinders;
        if cylinders > ecu.model.cylinders {
            return Err(format!("The {} can't fire {cylinders} cylinders", ecu.name));
        }
        match &ecu.tune {
            // A locked ECU runs its own maps.
            Some(tune) if ecu.model.locked => self.ecu = tune.clone(),
            // Another ECU runs the same tune as far as it goes.
            _ => {
                self.ecu.model = ecu.model;
                if self.ecu.cylinders != cylinders {
                    return Err("The tune is for another number of cylinders".into());
                }
            }
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
        if self.shop.kind == ShopKind::Engine && engines_over(catalogue, &self.held) {
            return Err(format!(
                "{} keeps one engine and one long block for you at most: take the other \
                 along, or ship it home first",
                self.shop.name
            ));
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

/// `path` as it is once the part `supply` says is taken from where it lies (the places
/// after it in the trunk or what's held move up).
fn shifted(path: &[u8], supply: &Supply) -> Result<Path, String> {
    let mut path = path.to_vec();
    let from = match supply {
        Supply::Trunk(j) => Some((IN_TRUNK, *j)),
        Supply::Held(j) => Some((HELD, *j)),
        _ => None,
    };
    if let (Some((list, j)), [first, i, ..]) = (from, path.as_mut_slice())
        && *first == list
    {
        match u16::from(*i).cmp(&j) {
            std::cmp::Ordering::Equal => return Err("A part doesn't go into itself".into()),
            std::cmp::Ordering::Greater => *i -= 1,
            std::cmp::Ordering::Less => {}
        }
    }
    Ok(path)
}

/// Why `car` lacks a part one of its parts needs, if it does.
pub fn needs_missing(catalogue: &Catalogue, car: &Item) -> Option<String> {
    if car.def(catalogue).kind == Kind::Chassis
        && let Some((part, need)) = crate::items::unmet_need(catalogue, car)
    {
        return Some(format!("The {} needs a {need}", part.to_lowercase()));
    }
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
            Work::Rebore { at, size } => {
                7u8.put(out);
                at.put(out);
                size.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let path = |r: &mut Reader| -> Option<Path> {
            let path: Path = r.get()?;
            (path.len() <= LONGEST_PATH).then_some(path)
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
            7 => Work::Rebore {
                at: path(r)?,
                size: r.get()?,
            },
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

/// `trim`'s stock car with its engine moved to the rear: the chassis' engine subframe
/// fitted at a tuning shop (for tests elsewhere).
#[cfg(test)]
pub fn mid_engined(catalogue: &Shared, trim: &str) -> CarSpec {
    let setup = crate::car::CarSetup::of_trim(catalogue, trim).expect("a trim");
    let before = CarSpec::of(&setup);
    let id = format!(
        "{}.rear-subframe.engine",
        trim.split('.').next().unwrap_or_default()
    );
    let shop = Shop {
        kind: ShopKind::Tuning,
        name: "Tuning".into(),
        at: Default::default(),
        sells: vec![id.clone()],
    };
    let slots = &before.car.def(catalogue).slots;
    let at = (slots.iter())
        .position(|slot| slot.kind == Kind::RearSubframe)
        .unwrap();
    let def = crate::items::by_id(catalogue, &id).expect("an engine subframe");
    let mut bench = Bench::new(catalogue, &shop, &before, &[]);
    bench
        .work(&Work::Fit {
            slots: vec![vec![at as u8]],
            supply: Supply::Part {
                def: def as u16,
                variant: 0,
                paint: 0,
            },
        })
        .unwrap();
    bench.finish().unwrap().spec
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

    /// Broken front drive shafts replaced at a car workshop: the car's are new (it drives
    /// again), the broken ones come off as they are; a tuning shop has stronger ones.
    #[test]
    fn a_workshop_mends_broken_drive_shafts() {
        let catalogue = builtin();
        let before = spec(1);
        let at = slots_of(&catalogue, &before.car, Kind::DriveShafts);
        assert_eq!(at.len(), 1);
        let mut broken = before.clone();
        let mut condition = crate::saved::Condition::of_parts(&catalogue, &broken.car, 40.0, 95.0);
        condition.shafts[0] = 1.0;
        condition.put_on(&catalogue, &mut broken.car);
        let workshop = shop(ShopKind::Car, &["vireo.drive-shafts"]);
        let fit = |id: &str| Work::Fit {
            slots: vec![at.clone()],
            supply: new(&catalogue, id),
        };
        let done = run(&catalogue, &workshop, &broken, &[fit("vireo.drive-shafts")]).unwrap();
        let now = crate::saved::Condition::of_parts(&catalogue, &done.spec.car, 40.0, 95.0);
        assert_eq!(now.shafts, [0.0; 3]);
        let off: Vec<&Item> = done.spec.cargo.iter().chain(&done.held).collect();
        assert!(off.iter().any(|shafts| shafts.wear == [1.0]), "{off:?}");
        assert!(
            run(
                &catalogue,
                &workshop,
                &broken,
                &[fit("vireo.drive-shafts.race")]
            )
            .is_err()
        );
        let tuning = shop(ShopKind::Tuning, &["vireo.drive-shafts.*"]);
        let done = run(
            &catalogue,
            &tuning,
            &broken,
            &[fit("vireo.drive-shafts.race")],
        )
        .unwrap();
        let line = crate::items::driveline(&catalogue, &done.spec.car);
        let stock = crate::items::driveline(&catalogue, &before.car);
        assert!(line.shafts[0] > 2.0 * stock.shafts[0], "{:?}", line.shafts);
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
    fn a_big_brake_needs_a_big_rim() {
        let catalogue = builtin();
        let workshop = shop(ShopKind::Tuning, &["vireo.brake.*"]);
        let fit = |option: usize| {
            let before = spec(option);
            let brakes = slots_of(&catalogue, &before.car, Kind::Brake);
            let work = Work::Fit {
                slots: vec![vec![brakes[0]], vec![brakes[1]]],
                supply: new(&catalogue, "vireo.brake.front-big"),
            };
            run(&catalogue, &workshop, &before, &[work])
        };
        // The 1.6's 14" steel rims don't clear it, the 2.0's 15" alloys do.
        let err = fit(0).unwrap_err();
        assert!(err.contains("15\""), "{err}");
        let done = fit(1).unwrap();
        let setup = done
            .spec
            .setup_with(crate::physics::engine::maps::EngineMaps::blank());
        assert!(setup.brakes[0].torque > CarSetup::factory(0, 1).brakes[0].torque);
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
        let line = crate::items::driveline(&catalogue, &done.spec.car);
        assert_eq!(
            line.differentials[0],
            crate::physics::drivetrain::Differential::LIMITED_SLIP
        );
        assert_eq!(done.spec.build.final_drive, before.build.final_drive);
    }

    /// The hatch made all-wheel drive a piece at a time: every step drives (the rear axle
    /// undriven until the prop shaft links it), a prop shaft needs a tunnel and something at
    /// both ends, and the tunnel kit goes in at car workshops.
    #[test]
    fn the_hatch_is_made_all_wheel_drive_a_piece_at_a_time() {
        use crate::physics::drivetrain::Drive;
        let catalogue = builtin();
        let tessel = catalogue.find_chassis("Tessel").unwrap();
        let before = CarSpec::of(&CarSetup::factory(tessel, 0));
        let workshop = shop(
            ShopKind::Car,
            &["tessel.*", "gearbox.*", "differential.*", "centre.*"],
        );
        let tuning = shop(ShopKind::Tuning, &["tessel.*"]);
        let at = |kind| slots_of(&catalogue, &before.car, kind);
        let fit = |slot: Vec<u8>, id: &str| Work::Fit {
            slots: vec![slot],
            supply: new(&catalogue, id),
        };
        let drive = |spec: &CarSpec| crate::items::driveline(&catalogue, &spec.car).drive;
        assert_eq!(drive(&before), Drive::Front);
        // The prop shaft wants the tunnel; the tunnel kit is a car workshop's work.
        let prop = fit(at(Kind::PropShaft), "tessel.prop-shaft");
        let why = run(&catalogue, &workshop, &before, std::slice::from_ref(&prop)).unwrap_err();
        assert!(why.contains("tunnel"), "{why}");
        let tunnel = fit(at(Kind::Tunnel), "tessel.tunnel-kit");
        assert!(run(&catalogue, &tuning, &before, std::slice::from_ref(&tunnel)).is_err());
        // With the tunnel but the front-drive gearbox, nothing turns the prop shaft.
        let why = run(
            &catalogue,
            &workshop,
            &before,
            &[tunnel.clone(), prop.clone()],
        );
        assert!(why.unwrap_err().contains("nothing to turn it"));
        // A piece at a time, each a car that drives its front wheels.
        let mut spec = before.clone();
        let subframe = at(Kind::RearSubframe)[0];
        for work in [
            tunnel,
            fit(at(Kind::Gearbox), "gearbox.k-5-speed-awd-transverse"),
            fit(vec![at(Kind::Gearbox)[0], 2], "centre.reactive"),
            fit(vec![subframe], "tessel.rear-subframe.awd"),
            fit(vec![subframe, 0], "differential.open"),
        ] {
            spec = run(&catalogue, &workshop, &spec, &[work]).unwrap().spec;
            assert_eq!(drive(&spec), Drive::Front);
        }
        // Without the drive shafts the prop shaft has nothing to drive.
        let why = run(&catalogue, &workshop, &spec, std::slice::from_ref(&prop)).unwrap_err();
        assert!(why.contains("nothing to drive"), "{why}");
        let shafts = fit(vec![subframe, 1], "tessel.rear-drive-shafts");
        spec = run(&catalogue, &workshop, &spec, &[shafts]).unwrap().spec;
        spec = run(&catalogue, &workshop, &spec, &[prop]).unwrap().spec;
        // All four, through the AWD gearbox's stock reactive coupling.
        assert_eq!(drive(&spec), Drive::All { front: 1.0 });
        let gearbox = spec.car.fitted(&catalogue, Kind::Gearbox).next().unwrap();
        let centre = gearbox.fitted(&catalogue, Kind::Centre).next().unwrap();
        assert_eq!(centre.def(&catalogue).id, "centre.reactive");
        // The front-drive gearbox back in with the prop shaft still there is refused.
        let back = fit(at(Kind::Gearbox), "gearbox.k-5-speed-transverse");
        assert!(run(&catalogue, &workshop, &spec, std::slice::from_ref(&back)).is_err());
        // The rear's parts weigh what they weigh: about 60 kg more than before.
        let setup = |spec: &CarSpec| {
            let (build, parts) = spec.car.resolve(&catalogue).unwrap();
            let engine = crate::physics::engine::EngineBuild::of_parts(&parts);
            CarSetup::with_car(build, parts, engine, spec.car.clone())
        };
        let gained = setup(&spec).mass_with(0.0) - setup(&before).mass_with(0.0);
        assert!((55.0..80.0).contains(&gained), "{gained} kg");
    }

    /// The saloon made all-wheel drive at the repo's car workshop in one go, then given
    /// limited slips front and rear at its tuning shop: it drives all four wheels and pulls
    /// away on a meadow better than it did.
    #[test]
    fn the_saloon_converts_at_the_shops_and_drives_all_four() {
        use crate::physics::drivetrain::{Differential, Drive};
        let catalogue = builtin();
        let shops = crate::shops::parse(crate::shops::LIST).unwrap();
        let named = |name: &str| shops.iter().find(|shop| shop.name == name).unwrap();
        let (workshop, tuning) = (named("Werkstatt Knockäcker"), named("Tuning Bastei"));
        let aldine = catalogue.find_chassis("Aldine").unwrap();
        let before = CarSpec::of(&CarSetup::factory(aldine, 1));
        let at = |kind| slots_of(&catalogue, &before.car, kind);
        let fit = |slot: Vec<u8>, id: &str| Work::Fit {
            slots: vec![slot],
            supply: new(&catalogue, id),
        };
        let (gearbox, subframe) = (at(Kind::Gearbox)[0], at(Kind::RearSubframe)[0]);
        let works = [
            fit(at(Kind::Tunnel), "aldine.tunnel-kit"),
            fit(vec![gearbox], "gearbox.k-5-speed-awd-transverse"),
            fit(vec![gearbox, 2], "centre.reactive"),
            fit(vec![subframe], "aldine.rear-subframe.awd"),
            fit(vec![subframe, 0], "differential.open"),
            fit(vec![subframe, 1], "aldine.rear-drive-shafts"),
            fit(at(Kind::PropShaft), "aldine.prop-shaft"),
        ];
        let awd = run(&catalogue, workshop, &before, &works).unwrap().spec;
        let lsd = [vec![gearbox, 1], vec![subframe, 0]]
            .map(|slot| fit(slot, "differential.limited-slip"));
        let awd = run(&catalogue, tuning, &awd, &lsd).unwrap().spec;
        let line = crate::items::driveline(&catalogue, &awd.car);
        assert_eq!(line.drive, Drive::All { front: 1.0 });
        assert_eq!(line.differentials, [Differential::LIMITED_SLIP; 2]);
        // On a meadow, floored with traction control.
        let terrain = crate::terrain::Terrain::flat_meadow();
        let speed = |spec: &CarSpec| {
            let (build, parts) = spec.car.resolve(&catalogue).unwrap();
            let engine = crate::physics::engine::EngineBuild::of_parts(&parts);
            let setup = CarSetup::with_car(build, parts, engine, spec.car.clone());
            let mut car = crate::car::Car::spawn_at(0, setup, &terrain, 50.0, 300.0, 0.0);
            car.warm_tires();
            car.parking_brake = false;
            car.drivetrain.automatic = true;
            car.drivetrain.shift_to(&car.setup.engine, 1);
            car.controls.throttle = 1.0;
            for _ in 0..(4.0 / crate::sim::STEP) as usize {
                crate::sim::step_car(&mut car, &terrain, crate::sim::STEP);
            }
            car.forward_speed().abs()
        };
        let (front, all) = (speed(&before), speed(&awd));
        println!("the saloon on a meadow in 4 s: {front:.1} m/s, converted {all:.1}");
        assert!(all > front + 2.0, "{all} against {front}");
    }

    /// The hot hatch with its engine behind the seats (`tessel.rear-subframe.engine`, a
    /// tuning shop's conversion): its engine, gearbox and front drive shafts move into the
    /// subframe in one fit, the rear bench comes out, the rear wheels are driven and the
    /// weight moves back; a second engine, the gearbox or shafts left at the front, the
    /// rear bench back in are refused, as are car workshops doing it; it reads back from
    /// the wire and a saved file, and goes back to the front with the factory subframe.
    #[test]
    fn the_hatch_gets_its_engine_behind_the_seats() {
        use crate::physics::drivetrain::{Differential, Drive};
        let catalogue = builtin();
        let before = CarSpec::of(&CarSetup::of_trim(&catalogue, "tessel.2-0r").unwrap());
        let at = |kind| slots_of(&catalogue, &before.car, kind);
        let subframe = at(Kind::RearSubframe)[0];
        let fit = |slot: Vec<u8>, id: &str| Work::Fit {
            slots: vec![slot],
            supply: new(&catalogue, id),
        };
        let convert = fit(vec![subframe], "tessel.rear-subframe.engine");
        let workshop = shop(ShopKind::Car, &["*"]);
        let why = run(
            &catalogue,
            &workshop,
            &before,
            std::slice::from_ref(&convert),
        );
        assert!(why.unwrap_err().contains("tuning shop"));
        let tuning = shop(ShopKind::Tuning, &["*"]);
        let done = run(&catalogue, &tuning, &before, std::slice::from_ref(&convert)).unwrap();
        let mid = done.spec;
        let car = &mid.car;
        for kind in [Kind::Engine, Kind::Gearbox, Kind::DriveShafts] {
            assert!(car.fitted(&catalogue, kind).next().is_none(), "{kind:?}");
        }
        let holder = crate::items::powertrain(&catalogue, car);
        assert!(!std::ptr::eq(holder, car));
        assert_eq!(holder.slots.iter().flatten().count(), 3);
        let bench = Kind::Seat(Seat::RearBench);
        assert!(car.fitted(&catalogue, bench).next().is_none());
        // Into the trunk if there's room left, else held.
        let off = mid.cargo.iter().chain(&done.held);
        assert!(off.clone().any(|item| item.def(&catalogue).kind == bench));
        assert_eq!(mid.build.bay().place, "middle, transverse");
        let line = crate::items::driveline(&catalogue, car);
        assert_eq!(line.drive, Drive::Rear);
        assert_eq!(line.differentials[1], Differential::LIMITED_SLIP);
        assert!(line.shafts[0].is_infinite() && line.shafts[1] == 1850.0);
        // The weight moves back; the K20R's long tubes still fit (no maps to bake).
        let (old, new) = (before.setup(), mid.setup());
        assert_eq!(old.engine.parts.hardware(), new.engine.parts.hardware());
        let front = |setup: &CarSetup| setup.front_weight_with(setup.tank.litres);
        let full = |setup: &CarSetup| setup.mass_with(setup.tank.litres);
        println!(
            "Tessel 2.0R: {:.0} kg, {:.1} % front; mid-engined {:.0} kg, {:.1} % front",
            full(&old),
            100.0 * front(&old),
            full(&new),
            100.0 * front(&new)
        );
        assert!((0.42..0.5).contains(&front(&new)), "{}", front(&new));
        assert!(new.inertia.y > old.inertia.y * 0.9);
        // The bay takes the hold's room where the bench was and the trunk's front.
        let mut benchless = before.car.clone();
        benchless.slots[usize::from(at(bench)[0])] = None;
        let room = |car: &Item| crate::cargo::grid(&catalogue, car).unwrap().room();
        assert!(
            room(car) < room(&benchless) / 2,
            "{} {}",
            room(car),
            room(&benchless)
        );
        // One engine to a car, its gearbox beside it, nothing left at the front to turn.
        let k20 = catalogue
            .assemblies
            .iter()
            .position(|a| a.id == "engine.k20")
            .unwrap();
        let refused = |works: &[Work], says: &str| {
            let why = run(&catalogue, &tuning, &mid, works).unwrap_err();
            assert!(why.contains(says), "{why}");
        };
        let engine = Work::Fit {
            slots: vec![at(Kind::Engine)],
            supply: Supply::Assembly(k20 as u16),
        };
        refused(&[engine], "One engine to a car");
        refused(
            &[fit(
                at(Kind::Gearbox),
                "gearbox.k-6-speed-close-ratio-transverse",
            )],
            "The gearbox goes beside the engine",
        );
        refused(
            &[fit(at(Kind::DriveShafts), "tessel.drive-shafts")],
            "front drive shafts have nothing to turn",
        );
        refused(&[fit(at(bench), "rear-bench.stock")], "rear bench's place");
        let shafts = vec![subframe, 2];
        refused(
            &[Work::Remove { slot: shafts }],
            "Nothing drives the wheels",
        );
        // Car workshops work on what's in it as on the front's: another gearbox.
        let gearbox = fit(vec![subframe, 1], "gearbox.k-5-speed-transverse");
        let swapped = run(&catalogue, &workshop, &mid, &[gearbox]).unwrap().spec;
        assert_eq!(swapped.build.gearbox().name, "K 5-speed (transverse)");
        // On the wire and saved, as it is.
        let bytes = encode(&mid);
        let read: CarSpec = crate::net::wire::decode_with(&bytes, Some(&catalogue)).unwrap();
        assert_eq!(read, mid);
        let condition = crate::saved::Condition::of_parts(&catalogue, &mid.car, 40.0, 95.0);
        let text = crate::saved::write(&catalogue, &mid, &condition);
        let (saved, _) = crate::saved::read(&catalogue, &text).unwrap();
        assert_eq!(saved.car, mid.car);
        assert_eq!(saved.build, mid.build);
        // Back to the front with the factory subframe (the bench stays where it went).
        let back = fit(vec![subframe], "tessel.rear-subframe");
        let back = run(&catalogue, &tuning, &mid, &[back]).unwrap().spec;
        assert_eq!(back.build, before.build);
        let mut expected = benchless;
        expected.slots[usize::from(subframe)] = before.car.slots[usize::from(subframe)].clone();
        assert_eq!(back.car, expected);
        assert_eq!(
            crate::items::driveline(&catalogue, &back.car).drive,
            Drive::Front
        );
    }

    /// The roadster's inline gearbox drives through its prop shaft to the differential in
    /// its rear subframe: neither comes out alone, and the hatch has no prop shaft slot.
    #[test]
    fn the_roadsters_prop_shaft_stays_and_the_vireo_takes_none() {
        use crate::physics::drivetrain::{Differential, Drive};
        let catalogue = builtin();
        let merrow = catalogue.find_chassis("Merrow").unwrap();
        let before = CarSpec::of(&CarSetup::factory(merrow, 0));
        let line = crate::items::driveline(&catalogue, &before.car);
        assert_eq!(line.drive, Drive::Rear);
        assert_eq!(line.differentials[1], Differential::LIMITED_SLIP);
        let workshop = shop(ShopKind::Car, &["merrow.*", "differential.*"]);
        let prop = slots_of(&catalogue, &before.car, Kind::PropShaft);
        let why = run(
            &catalogue,
            &workshop,
            &before,
            &[Work::Remove { slot: prop }],
        );
        assert!(why.unwrap_err().contains("Nothing drives the wheels"));
        // An open differential at the back instead is fine.
        let subframe = slots_of(&catalogue, &before.car, Kind::RearSubframe)[0];
        let open = Work::Fit {
            slots: vec![vec![subframe, 0]],
            supply: new(&catalogue, "differential.open"),
        };
        let done = run(&catalogue, &workshop, &before, &[open]).unwrap();
        let line = crate::items::driveline(&catalogue, &done.spec.car);
        assert_eq!(line.differentials[1], Differential::OPEN);
        let vireo = spec(0);
        assert!(slots_of(&catalogue, &vireo.car, Kind::PropShaft).is_empty());
        assert!(slots_of(&catalogue, &vireo.car, Kind::Tunnel).is_empty());
    }

    #[test]
    fn a_k_gearbox_doesnt_go_on_a_b_engine() {
        let catalogue = builtin();
        let before = spec(2);
        let workshop = shop(ShopKind::Tuning, &["gearbox.*", "adapter-plate.*"]);
        let gearbox = slots_of(&catalogue, &before.car, Kind::Gearbox);
        let fit = |id: &str| Work::Fit {
            slots: vec![gearbox.clone()],
            supply: new(&catalogue, id),
        };
        let err = run(
            &catalogue,
            &workshop,
            &before,
            &[fit("gearbox.k-6-speed-close-ratio-transverse")],
        )
        .unwrap_err();
        assert!(err.contains("K-series bellhousing"), "{err}");
        assert!(err.contains("B-series four"), "{err}");
        // No plate helps, and a transverse gearbox has no place for one.
        let after = with_fitted(
            &catalogue,
            &before.car,
            &gearbox,
            Item::new(
                &catalogue,
                def(&catalogue, "gearbox.k-5-speed-transverse").into(),
                0,
            ),
        )
        .unwrap();
        let misbolt = crate::items::drivetrain_misfit(&catalogue, &after).unwrap();
        assert_eq!(misbolt.fix, None);
        assert_eq!(adapter_slot(&catalogue, &after), None);
        // The B close-ratio box goes on.
        let done = run(
            &catalogue,
            &workshop,
            &before,
            &[fit("gearbox.6-speed-close-ratio-transverse")],
        )
        .unwrap();
        assert_eq!(done.spec.build.gearbox, 1);
    }

    /// A stock ECU is locked to its factory's maps, on whatever engine; a programmable one
    /// takes a tune; neither fires more cylinders than it's made for.
    #[test]
    fn stock_ecus_are_locked_and_ecus_fire_their_cylinders() {
        let catalogue = builtin();
        let before = spec(0);
        let ecu_slot = vec![slots_of(&catalogue, &before.car, Kind::Ecu)[0]];
        let engine = slots_of(&catalogue, &before.car, Kind::Engine)[0];
        let dyno = shop(ShopKind::Dyno, &[]);
        let mut tuned = before.ecu.clone();
        tuned.rev_limit -= 500.0;
        let tune = Work::Tune(Box::new(tuned.clone()));
        let err = run(&catalogue, &dyno, &before, std::slice::from_ref(&tune)).unwrap_err();
        assert!(err.contains("locked"), "{err}");
        let workshop = shop(ShopKind::Car, &["*"]);
        let fit = |id: &str| Work::Fit {
            slots: vec![ecu_slot.clone()],
            supply: new(&catalogue, id),
        };
        let programmable = run(&catalogue, &workshop, &before, &[fit("ecu.programmable-4")])
            .unwrap()
            .spec;
        let done = run(&catalogue, &dyno, &programmable, &[tune]).unwrap();
        assert_eq!(done.spec.ecu.rev_limit, tuned.rev_limit);
        // Back to a stock ECU: its own maps, not the tune.
        let b20 = run(&catalogue, &workshop, &done.spec, &[fit("ecu.b20")]).unwrap();
        let b20_tune = catalogue.ecus.iter().find(|e| e.id == "ecu.b20").unwrap();
        assert_eq!(Some(&b20.spec.ecu), b20_tune.tune.as_ref());
        // The B16's ECU keeps its maps with the B20 engine swapped in.
        let swap = Work::Fit {
            slots: vec![vec![engine]],
            supply: Supply::Assembly(1),
        };
        let swapped = run(&catalogue, &workshop, &before, &[swap]).unwrap();
        assert_eq!(swapped.spec.parts.hardware().name, "B20 2.0 SOHC four");
        assert_eq!(swapped.spec.ecu, before.ecu);
        // Four cylinders' ECU and the V10.
        let v10 = CarSpec::of(&CarSetup::factory(1, 0));
        let slot = vec![slots_of(&catalogue, &v10.car, Kind::Ecu)[0]];
        let fit_v10 = |id: &str| Work::Fit {
            slots: vec![slot.clone()],
            supply: new(&catalogue, id),
        };
        let err = run(
            &catalogue,
            &workshop,
            &v10,
            &[fit_v10("ecu.programmable-4")],
        )
        .unwrap_err();
        assert!(err.contains("10 cylinders"), "{err}");
        assert!(
            run(
                &catalogue,
                &workshop,
                &v10,
                &[fit_v10("ecu.programmable-10")]
            )
            .is_ok()
        );
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
        assert_eq!(done.spec.parts.hardware().name, "B20 2.0 SOHC four");
        let out = Work::Remove { slot: vec![engine] };
        assert!(run(&catalogue, &workshop, &before, &[out]).is_err());
    }

    /// The path of the car's long block's slot holding `id`'s kind of part, or its cylinder
    /// head's (from the engine's slot down), and the long block's.
    fn in_block(catalogue: &Catalogue, car: &Item, id: &str) -> (Path, Path) {
        let engine = slots_of(catalogue, car, Kind::Engine)[0];
        let kind = catalogue.defs[usize::from(def(catalogue, id))].kind;
        let block = vec![engine, 0];
        let place = |holder: &Path, kind: Kind| {
            let item = item_at(car, holder).unwrap();
            (item.def(catalogue).slots.iter())
                .position(|slot| slot.kind == kind)
                .map(|slot| [holder.clone(), vec![slot as u8]].concat())
        };
        let path = place(&block, kind)
            .or_else(|| place(&place(&block, Kind::Head).unwrap(), kind))
            .unwrap();
        (path, block)
    }

    const ENGINE_SHOP: &[&str] = &["pistons.*", "connecting-rods.*", "b20", "camshafts.*"];

    #[test]
    fn phasers_and_switching_cams_need_overhead_cams() {
        let catalogue = builtin();
        let before = spec(0);
        let engines = shop(ShopKind::Engine, &["*"]);
        let fit = |catalogue: &Catalogue, id: &str| {
            let (path, _) = in_block(catalogue, &before.car, id);
            Work::Fit {
                slots: vec![path],
                supply: new(catalogue, id),
            }
        };
        for id in ["cam-phasers.intake", "camshafts.switching"] {
            assert!(run(&catalogue, &engines, &before, &[fit(&catalogue, id)]).is_ok());
        }
        // The same block with its cam in the block (pushrods) provides no overhead cams.
        let pushrods = crate::catalogue::builtin_changed(|c| {
            for def in &mut c.defs {
                def.provides
                    .retain(|what| what != crate::items::OVERHEAD_CAMS);
            }
        });
        for id in ["cam-phasers.intake", "camshafts.switching"] {
            let err = run(&pushrods, &engines, &before, &[fit(&pushrods, id)]).unwrap_err();
            assert!(err.contains("overhead cams"), "{err}");
        }
        assert!(
            run(
                &pushrods,
                &engines,
                &before,
                &[fit(&pushrods, "camshafts.sport")]
            )
            .is_ok()
        );
    }

    #[test]
    fn an_engine_shop_works_on_long_blocks_only() {
        let catalogue = builtin();
        let mut before = spec(0);
        // Worn pistons, as the car was.
        let mut condition = crate::saved::Condition::of_parts(&catalogue, &before.car, 30.0, 95.0);
        condition.damage.cylinders[2].piston = 0.6;
        condition.damage.cylinders[0].rod = 0.2;
        condition.put_on(&catalogue, &mut before.car);
        let engines = shop(ShopKind::Engine, ENGINE_SHOP);
        let (pistons, _) = in_block(&catalogue, &before.car, "pistons.race");
        let fit = Work::Fit {
            slots: vec![pistons.clone()],
            supply: new(&catalogue, "pistons.race"),
        };
        let done = run(&catalogue, &engines, &before, std::slice::from_ref(&fit)).unwrap();
        assert!(
            done.spec.parts.hardware().compression_ratio
                > before.parts.hardware().compression_ratio
        );
        // The new pistons are new, the rods as worn as they were; the old pistons kept their
        // wear, in the trunk or held.
        let now = crate::saved::Condition::of_parts(&catalogue, &done.spec.car, 30.0, 95.0);
        assert_eq!(now.damage.cylinders[2].piston, 0.0);
        assert_eq!(now.damage.cylinders[0].rod, 0.2);
        let old: Vec<&Item> = done.spec.cargo.iter().chain(&done.held).collect();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].wear, [0.0, 0.0, 0.6]);
        // Nothing but long blocks: no clutch, no bolt-ons.
        let clutch = Work::Fit {
            slots: vec![slots_of(&catalogue, &before.car, Kind::Clutch)],
            supply: new(&catalogue, "clutch.race"),
        };
        let all = shop(ShopKind::Engine, &["*"]);
        assert!(run(&catalogue, &all, &before, &[clutch]).is_err());
        let engine = slots_of(&catalogue, &before.car, Kind::Engine)[0];
        let intake = Work::Fit {
            slots: vec![vec![engine, 1]],
            supply: new(&catalogue, "intake-manifold.long-runners"),
        };
        assert!(run(&catalogue, &all, &before, &[intake]).is_err());
        // A long block brought in whole goes in, with the old one's insides moved over where
        // it has none.
        let (_, block) = in_block(&catalogue, &before.car, "pistons.race");
        let swap = Work::Fit {
            slots: vec![block],
            supply: new(&catalogue, "b20"),
        };
        let done = run(&catalogue, &engines, &before, &[swap]).unwrap();
        assert_eq!(done.spec.parts.version, 1);
        let swapped = crate::saved::Condition::of_parts(&catalogue, &done.spec.car, 30.0, 95.0);
        assert_eq!(swapped.damage.cylinders[0].rod, 0.2);
    }

    #[test]
    fn a_roots_blower_needs_a_bonnet_with_a_cut_out() {
        let catalogue = builtin();
        let before = spec(0);
        let workshop = shop(ShopKind::Tuning, &["turbocharger.*", "vireo.bonnet*"]);
        let engine = slots_of(&catalogue, &before.car, Kind::Engine)[0];
        let engine_def = before.car.slots[engine as usize]
            .as_ref()
            .unwrap()
            .def(&catalogue);
        let turbo = (engine_def.slots.iter())
            .position(|slot| slot.kind == Kind::EnginePart(crate::catalogue::SUPERCHARGER))
            .unwrap() as u8;
        let blower = Work::Fit {
            slots: vec![vec![engine, turbo]],
            supply: new(&catalogue, "turbocharger.roots-blower"),
        };
        let err = run(
            &catalogue,
            &workshop,
            &before,
            std::slice::from_ref(&blower),
        )
        .unwrap_err();
        assert!(err.contains("bonnet with a cut-out"), "{err}");
        let bonnet = Work::Fit {
            slots: vec![slots_of(
                &catalogue,
                &before.car,
                Kind::Body(crate::items::Panel::Bonnet),
            )],
            supply: new(&catalogue, "vireo.bonnet.cut-out"),
        };
        let done = run(&catalogue, &workshop, &before, &[bonnet, blower.clone()]).unwrap();
        let hardware = done.spec.parts.hardware();
        assert!(hardware.supercharger.is_some_and(|s| s.roots));
        // Without a bonnet it sticks out into the open air.
        let off = Work::Remove {
            slot: slots_of(
                &catalogue,
                &before.car,
                Kind::Body(crate::items::Panel::Bonnet),
            ),
        };
        let done = run(&catalogue, &workshop, &before, &[off, blower]).unwrap();
        assert!(done.spec.parts.hardware().supercharger.is_some());
        // The centrifugal one goes under the plain bonnet.
        let centrifugal = Work::Fit {
            slots: vec![vec![engine, turbo]],
            supply: new(&catalogue, "turbocharger.centrifugal-supercharger"),
        };
        let done = run(&catalogue, &workshop, &before, &[centrifugal]).unwrap();
        assert!(
            done.spec
                .parts
                .hardware()
                .supercharger
                .is_some_and(|s| !s.roots)
        );
    }

    /// The path of the engine's exhaust manifold in `spec`'s car.
    fn manifold_at(catalogue: &Catalogue, spec: &CarSpec) -> Path {
        let engine = slots_of(catalogue, &spec.car, Kind::Engine)[0];
        let engine_def = item_at(&spec.car, &[engine]).unwrap().def(catalogue);
        let manifold = (engine_def.slots.iter())
            .position(|slot| slot.kind == Kind::Manifold)
            .unwrap();
        vec![engine, manifold as u8]
    }

    /// The turbos fitted on `spec`'s manifold, by their ids (empty mounts as "-").
    fn turbos(catalogue: &Catalogue, spec: &CarSpec) -> Vec<String> {
        let manifold = item_at(&spec.car, &manifold_at(catalogue, spec)).unwrap();
        (manifold.def(catalogue).slots.iter().zip(&manifold.slots))
            .filter(|(slot, _)| slot.kind == Kind::Turbo)
            .map(|(_, turbo)| {
                turbo
                    .as_ref()
                    .map_or("-".into(), |t| t.def(catalogue).id.clone())
            })
            .collect()
    }

    const TUNING: &[&str] = &["b.*", "r.*", "turbocharger.*"];

    #[test]
    fn long_tubes_have_no_mount_for_a_turbo() {
        let catalogue = builtin();
        let before = spec(0);
        let tuning = shop(ShopKind::Tuning, TUNING);
        let manifold = manifold_at(&catalogue, &before);
        let headers = Work::Fit {
            slots: vec![manifold.clone()],
            supply: new(&catalogue, "b.headers.long-tube"),
        };
        let done = run(&catalogue, &tuning, &before, std::slice::from_ref(&headers)).unwrap();
        assert!(turbos(&catalogue, &done.spec).is_empty());
        assert_eq!(done.spec.parts.manifold, 2);
        let carried: Vec<&str> = (done.spec.cargo.iter())
            .map(|item| item.def(&catalogue).id.as_str())
            .collect();
        assert_eq!(carried, ["b.manifold.stock"]);
        // Nowhere to put a turbo on them; on the turbo manifold, it goes on its mount.
        let mut mount = manifold.clone();
        mount.push(0);
        let turbo = Work::Fit {
            slots: vec![mount],
            supply: new(&catalogue, "turbocharger.medium"),
        };
        assert!(run(&catalogue, &tuning, &before, &[headers, turbo.clone()]).is_err());
        let turbo_manifold = Work::Fit {
            slots: vec![manifold],
            supply: new(&catalogue, "b.manifold.turbo"),
        };
        let done = run(&catalogue, &tuning, &before, &[turbo_manifold, turbo]).unwrap();
        assert_eq!(turbos(&catalogue, &done.spec), ["turbocharger.medium"]);
        let hardware = done.spec.parts.hardware();
        assert!(hardware.turbo.is_some());
        // Until milestone 2 the B16 with the turbo manifold and the medium turbo is the
        // engine the old turbo slot made (the maps baked for it).
        let mut old = crate::physics::engine::parts::Parts::turbo();
        old.chosen[crate::catalogue::PISTONS] = 0;
        old.header_room = done.spec.parts.header_room;
        assert_eq!(hardware, old.hardware());
    }

    #[test]
    fn a_turbo_goes_on_every_mount_alike() {
        let catalogue = builtin();
        let before = CarSpec::of(&CarSetup::factory(1, 0));
        let tuning = shop(ShopKind::Tuning, TUNING);
        let manifold = manifold_at(&catalogue, &before);
        let pair = Work::Fit {
            slots: vec![manifold.clone()],
            supply: new(&catalogue, "r.manifolds.turbo"),
        };
        // With the pair of turbo manifolds on, a mount is either side of it.
        let mut bench = Bench::new(&catalogue, &tuning, &before, &[]);
        bench.work(&pair).unwrap();
        let tree = (&bench.car, &bench.cargo[..], &bench.held[..]);
        let mut first = manifold.clone();
        first.push(0);
        let mounts = mounts(&catalogue, tree, &first);
        assert_eq!(mounts.len(), 2);
        assert!(super::mounts(&catalogue, tree, &manifold).is_empty());
        let both = Work::Fit {
            slots: mounts.clone(),
            supply: new(&catalogue, "turbocharger.small"),
        };
        let done = run(&catalogue, &tuning, &before, &[pair.clone(), both]).unwrap();
        assert_eq!(turbos(&catalogue, &done.spec), ["turbocharger.small"; 2]);
        assert_eq!(done.spec.parts.chosen[crate::catalogue::TURBO], 1);
        assert!(done.spec.parts.hardware().turbo.is_some());
        // One side alone, or different turbos either side, won't do.
        let fit = |mount: &Path, id: &str| Work::Fit {
            slots: vec![mount.clone()],
            supply: new(&catalogue, id),
        };
        let one = run(
            &catalogue,
            &tuning,
            &before,
            &[pair.clone(), fit(&mounts[0], "turbocharger.small")],
        )
        .unwrap_err();
        assert!(one.contains("every mount"), "{one}");
        let mixed = run(
            &catalogue,
            &tuning,
            &before,
            &[
                pair,
                fit(&mounts[0], "turbocharger.small"),
                fit(&mounts[1], "turbocharger.large"),
            ],
        )
        .unwrap_err();
        assert!(mixed.contains("the same"), "{mixed}");
        // The 10-into-1 has one mount, for one big turbo.
        let crossover = Work::Fit {
            slots: vec![manifold.clone()],
            supply: new(&catalogue, "r.manifold.turbo-crossover"),
        };
        let done = run(
            &catalogue,
            &tuning,
            &before,
            &[crossover, fit(&first, "turbocharger.large")],
        )
        .unwrap();
        assert_eq!(turbos(&catalogue, &done.spec), ["turbocharger.large"]);
    }

    #[test]
    fn turbos_move_to_another_manifold_only_all_together() {
        let catalogue = builtin();
        let before = CarSpec::of(&CarSetup::factory(1, 0));
        let tuning = shop(ShopKind::Tuning, TUNING);
        let manifold = manifold_at(&catalogue, &before);
        let fit = |id: &str, slots: Vec<Path>| Work::Fit {
            slots,
            supply: new(&catalogue, id),
        };
        let mut left = manifold.clone();
        left.push(0);
        let mut right = manifold.clone();
        right.push(1);
        let pair = [
            fit("r.manifolds.turbo", vec![manifold.clone()]),
            fit("turbocharger.small", vec![left, right]),
        ];
        // From the pair to the 10-into-1: its one mount stays empty, the turbos on the
        // pair taken off.
        let mut works = pair.to_vec();
        works.push(fit("r.manifold.turbo-crossover", vec![manifold.clone()]));
        let done = run(&catalogue, &tuning, &before, &works).unwrap();
        assert_eq!(turbos(&catalogue, &done.spec), ["-"]);
        let off = (done.spec.cargo.iter().chain(&done.held))
            .find(|item| item.def(&catalogue).id == "r.manifolds.turbo")
            .unwrap();
        assert_eq!(off.fitted(&catalogue, Kind::Turbo).count(), 2);
        // From one pair of turbo manifolds to another alike, they move over.
        let mut works = pair.to_vec();
        works.push(fit("r.manifolds.turbo", vec![manifold]));
        let done = run(&catalogue, &tuning, &before, &works).unwrap();
        assert_eq!(turbos(&catalogue, &done.spec), ["turbocharger.small"; 2]);
    }

    #[test]
    fn a_turbo_and_a_supercharger_dont_go_together() {
        let catalogue = builtin();
        let before = spec(0);
        let tuning = shop(ShopKind::Tuning, TUNING);
        let manifold = manifold_at(&catalogue, &before);
        let mut mount = manifold.clone();
        mount.push(0);
        let engine = manifold[0];
        let engine_def = item_at(&before.car, &[engine]).unwrap().def(&catalogue);
        let charger = (engine_def.slots.iter())
            .position(|slot| slot.kind == Kind::EnginePart(crate::catalogue::SUPERCHARGER))
            .unwrap() as u8;
        let works = [
            Work::Fit {
                slots: vec![manifold],
                supply: new(&catalogue, "b.manifold.turbo"),
            },
            Work::Fit {
                slots: vec![mount],
                supply: new(&catalogue, "turbocharger.small"),
            },
            Work::Fit {
                slots: vec![vec![engine, charger]],
                supply: new(&catalogue, "turbocharger.centrifugal-supercharger"),
            },
        ];
        assert!(run(&catalogue, &tuning, &before, &works[..2]).is_ok());
        let err = run(&catalogue, &tuning, &before, &works).unwrap_err();
        assert!(err.contains("supercharger"), "{err}");
    }

    #[test]
    fn a_manifold_fits_only_its_engine_family() {
        let catalogue = builtin();
        let before = CarSpec::of(&CarSetup::factory(1, 0));
        let tuning = shop(ShopKind::Tuning, TUNING);
        let manifold = manifold_at(&catalogue, &before);
        let fit = |id: &str| Work::Fit {
            slots: vec![manifold.clone()],
            supply: new(&catalogue, id),
        };
        let err = run(&catalogue, &tuning, &before, &[fit("b.manifold.stock")]).unwrap_err();
        assert!(err.contains("R80"), "{err}");
        let done = run(&catalogue, &tuning, &before, &[fit("r.headers.long-tube")]).unwrap();
        assert_eq!(done.spec.parts.manifold, 1);
        // What the shop offers for the slot is only the family's.
        let r = engine_on(&catalogue, (&before.car, &[], &[]), &manifold);
        let b_stock = &catalogue.defs[usize::from(def(&catalogue, "b.manifold.stock"))];
        let r_stock = &catalogue.defs[usize::from(def(&catalogue, "r.manifolds.stock"))];
        assert!(!made_for_engine(&catalogue, b_stock, r));
        assert!(made_for_engine(&catalogue, r_stock, r));
    }

    #[test]
    fn a_rebored_block_takes_pistons_of_its_size() {
        let catalogue = builtin();
        let before = spec(0);
        let engines = shop(ShopKind::Engine, ENGINE_SHOP);
        let (pistons, block) = in_block(&catalogue, &before.car, "pistons.stock");
        let rebore = Work::Rebore {
            at: block.clone(),
            size: 2,
        };
        // Bored alone, the stock pistons don't fit any more.
        let err = run(&catalogue, &engines, &before, std::slice::from_ref(&rebore)).unwrap_err();
        assert!(err.contains("+0.50 mm"), "{err}");
        let oversize = Work::Fit {
            slots: vec![pistons.clone()],
            supply: Supply::Part {
                def: def(&catalogue, "pistons.stock"),
                variant: 2,
                paint: 0,
            },
        };
        let done = run(&catalogue, &engines, &before, &[rebore.clone(), oversize]).unwrap();
        let (was, now) = (before.parts.hardware(), done.spec.parts.hardware());
        assert!((now.bore - was.bore - 0.5e-3).abs() < 1e-6);
        assert!(now.displacement() > was.displacement());
        assert_eq!(done.spec.parts.bore, 2);
        assert_ne!(
            crate::physics::engine::maps::fingerprint(&now),
            crate::physics::engine::maps::fingerprint(&was)
        );
        // Never smaller again, nor anywhere but an engine shop.
        let back = Work::Rebore { at: block, size: 1 };
        assert!(run(&catalogue, &engines, &done.spec, &[back]).is_err());
        let workshop = shop(ShopKind::Car, &["*"]);
        assert!(run(&catalogue, &workshop, &before, &[rebore]).is_err());
        // Rebores go over the wire.
        let work = Work::Rebore {
            at: vec![HELD, 0, 0],
            size: 3,
        };
        assert_eq!(decode::<Work>(&encode(&work)), Some(work));
    }

    #[test]
    fn an_engine_kept_at_the_engine_shop_is_worked_on_and_swapped_in() {
        let catalogue = builtin();
        let before = spec(0);
        let engines = shop(ShopKind::Engine, ENGINE_SHOP);
        // The B20 engine kept there, its rods worn.
        let mut kept = catalogue.assemblies[1].item.clone();
        let rods = (kept.slots[0].as_mut().unwrap().slots.iter_mut().flatten())
            .find(|item| catalogue.defs[item.def].wear == Some(crate::items::Wear::Rods))
            .unwrap();
        rods.wear = vec![0.5; 4];
        let (pistons, _) = in_block(&catalogue, &before.car, "pistons.race");
        let at_held = |path: &Path| {
            let mut held = vec![HELD, 0];
            held.extend_from_slice(&path[1..]);
            held
        };
        // Race cams into it, as it lies there.
        let (cams, _) = in_block(&catalogue, &before.car, "camshafts.race");
        let mut bench = Bench::new(&catalogue, &engines, &before, std::slice::from_ref(&kept));
        bench
            .work(&Work::Fit {
                slots: vec![at_held(&cams)],
                supply: new(&catalogue, "camshafts.race"),
            })
            .unwrap();
        // Its pistons can come out (a loose engine may lack them).
        bench
            .work(&Work::Remove {
                slot: at_held(&pistons),
            })
            .unwrap();
        bench
            .work(&Work::Fit {
                slots: vec![at_held(&pistons)],
                supply: new(&catalogue, "pistons.high-compression"),
            })
            .unwrap();
        // Then swapped in: the car's engine is kept here instead.
        let engine = slots_of(&catalogue, &before.car, Kind::Engine);
        bench
            .work(&Work::Fit {
                slots: vec![engine],
                supply: Supply::Held(0),
            })
            .unwrap();
        let done = bench.finish().unwrap();
        assert_eq!(done.spec.parts.version, 1);
        let held: Vec<Kind> = done.held.iter().map(|i| i.def(&catalogue).kind).collect();
        assert!(held.contains(&Kind::Engine), "{held:?}");
        let now = crate::saved::Condition::of_parts(&catalogue, &done.spec.car, 30.0, 95.0);
        assert_eq!(now.damage.cylinders[3].rod, 0.5);
        // A second engine isn't kept there.
        let two = [kept.clone(), kept];
        let mut bench = Bench::new(&catalogue, &engines, &before, &two);
        bench
            .work(&Work::Fit {
                slots: vec![at_held(&cams)],
                supply: new(&catalogue, "camshafts.race"),
            })
            .unwrap();
        assert!(bench.finish().is_err());
    }

    /// The roadster as it leaves the factory, and its trim's tree with the engine `code`
    /// (whatever fits) in it, the engine taken out on its own.
    fn roadster_with(code: &str) -> (CarSpec, Item) {
        let catalogue = builtin();
        let merrow = catalogue.find_chassis("Merrow").unwrap();
        let before = CarSpec::of(&CarSetup::factory(merrow, 0));
        let build = crate::chassis::CarBuild::factory(merrow);
        let parts = build
            .install(crate::physics::engine::parts::Parts::factory(code).unwrap())
            .unwrap();
        let car = crate::items::assemble(&catalogue, Some(&before.car), &build, &parts).unwrap();
        let engine = car.fitted(&catalogue, Kind::Engine).next().unwrap().clone();
        (before, engine)
    }

    #[test]
    fn a_k_goes_into_the_roadster_only_with_its_adapter_plate() {
        let catalogue = builtin();
        let (before, k20) = roadster_with("K20");
        let engines = shop(ShopKind::Engine, &["adapter-plate.*"]);
        let engine = slots_of(&catalogue, &before.car, Kind::Engine);
        let plate = adapter_slot(&catalogue, &before.car).expect("an inline gearbox's plate");
        let swap = Work::Fit {
            slots: vec![engine],
            supply: Supply::Held(0),
        };
        let fit_plate = Work::Fit {
            slots: vec![plate.clone()],
            supply: new(&catalogue, "adapter-plate.k-to-r"),
        };
        let at = |works: &[Work], spec: &CarSpec, held: &[Item]| {
            let mut bench = Bench::new(&catalogue, &engines, spec, held);
            for work in works {
                bench.work(work)?;
            }
            bench.finish()
        };
        let held = std::slice::from_ref(&k20);
        // On its own the K doesn't bolt to the V10's gearbox, nor the plate to the V10.
        let err = at(std::slice::from_ref(&swap), &before, held).unwrap_err();
        assert!(err.contains("adapter plate"), "{err}");
        assert!(at(std::slice::from_ref(&fit_plate), &before, &[]).is_err());
        // Together they do: the plate sits on the gearbox.
        let done = at(&[swap.clone(), fit_plate.clone()], &before, held).unwrap();
        assert_eq!(done.spec.parts.hardware().name, "K20 2.0 DOHC four");
        assert_eq!(
            crate::items::drivetrain_misfit(&catalogue, &done.spec.car),
            None
        );
        let on = item_at(&done.spec.car, &plate).unwrap();
        assert_eq!(on.def(&catalogue).id, "adapter-plate.k-to-r");
        // The plate stays as long as the K does.
        let off = Work::Remove { slot: plate };
        assert!(at(std::slice::from_ref(&off), &done.spec, &done.held).is_err());
        // The V10 back in (held there) takes the plate off with it.
        let v10 = (done.held.iter())
            .position(|item| item.def(&catalogue).kind == Kind::Engine)
            .expect("the V10 held at the shop") as u16;
        let back = Work::Fit {
            slots: vec![slots_of(&catalogue, &before.car, Kind::Engine)],
            supply: Supply::Held(v10),
        };
        let err = at(std::slice::from_ref(&back), &done.spec, &done.held).unwrap_err();
        assert!(err.contains("bolts straight"), "{err}");
        let done = at(&[back, off], &done.spec, &done.held).unwrap();
        assert_eq!(done.spec.parts.hardware().name, "R80 8.0 V10");
    }

    #[test]
    fn a_b_doesnt_bolt_to_the_roadsters_gearbox() {
        let catalogue = builtin();
        let (before, b16) = roadster_with("B16");
        let engine = slots_of(&catalogue, &before.car, Kind::Engine);
        let after = with_fitted(&catalogue, &before.car, &engine, b16.clone()).unwrap();
        let why = crate::items::drivetrain_misfit(&catalogue, &after).unwrap();
        assert!(why.why.contains("no adapter plate"), "{}", why.why);
        assert_eq!(why.fix, None);
        let engines = shop(ShopKind::Engine, &["adapter-plate.*"]);
        let mut bench = Bench::new(&catalogue, &engines, &before, std::slice::from_ref(&b16));
        bench
            .work(&Work::Fit {
                slots: vec![engine],
                supply: Supply::Held(0),
            })
            .unwrap();
        assert_eq!(bench.finish().unwrap_err(), why.why);
    }

    #[test]
    fn what_a_new_part_would_make_is_seen_before() {
        let catalogue = builtin();
        let (before, k20) = roadster_with("K20");
        let engine = slots_of(&catalogue, &before.car, Kind::Engine);
        let after = with_fitted(&catalogue, &before.car, &engine, k20).unwrap();
        let misbolt = crate::items::drivetrain_misfit(&catalogue, &after).unwrap();
        let crate::items::Fix::Fit(plate) = misbolt.fix.unwrap() else {
            panic!("an adapter plate fixes it");
        };
        assert_eq!(catalogue.adapters[plate].id, "adapter-plate.k-to-r");
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
        // The hatch can't come off under the 2.0's spoiler.
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
