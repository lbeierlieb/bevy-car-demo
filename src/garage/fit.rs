//! The window of car workshops, tuning shops, tire shops and engine shops (in the
//! workshop, `bay`): the car's slots the shop works on, and for each what's fitted (and how
//! worn), taking it off, new parts the shop sells (in a size and paint), parts brought in
//! the trunk or held here. Tire shops also make new wheels of a rim and a tire, make loose
//! wheels and take them apart. Engine shops also work on an engine or long block brought
//! (in the trunk or held there), and rebore blocks (with pistons of the new size). An
//! exhaust manifold's screen says what it is (primaries, branches, collectors, mounts) and
//! lists its mounts; turbos go on, and come off, every mount at once (new ones, or as many
//! alike brought). A new part that would leave the gearbox not bolting to the engine says
//! why, and is offered along with the adapter plate that makes it (or with the plate
//! off), where the shop can do that. Every change is asked of the server (`fitting::Work`), which answers
//! with the car.

use bevy_egui::egui;

use crate::car::Car;
use crate::catalogue::Catalogue;
use crate::fitting::{
    HELD, IN_TRUNK, Path, Supply, Work, adapter_slot, bore_name, engine_on, fits_into, item_in,
    loose, made_for, made_for_engine, mounts, new_paints, slot_kinds_in, wares, wheel_of,
    with_fitted,
};
use crate::items::{Does, Fix, Item, Kind, OVERSIZES, drivetrain_misfit};
use crate::menu::kit;
use crate::net::Connection;
use crate::owned::name;
use crate::physics::engine::hardware::Collector;
use crate::shops::{Shop, ShopKind};
use crate::wheels::{self, Rim, TireSize, Wheel};

/// The window's screens, in a stack (the first the car's slots).
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    /// A slot (the car's own slots at none): what's in it, and what can go in.
    Slot(Path),
    /// A new part for a slot (its place among the shop's wares for it).
    Ware(Path, usize),
    /// A new wheel, rim and tire, for a wheel's slot (tire shops).
    NewWheel(Path),
    /// A loose wheel of a rim brought (tire shops).
    MakeWheel,
    /// A loose part (`[IN_TRUNK, i]` or `[HELD, i]`): its slots the shop works on (engine
    /// shops: an engine or long block brought).
    Loose(Path),
    /// The block at a path bored over (engine shops).
    Rebore(Path),
}

/// The screens open, and what's picked on them.
pub struct Fitter {
    screens: Vec<Screen>,
    /// Per screen under the top one, the row that went on from it: focused again on
    /// coming back (as in the central menu).
    came_from: Vec<Option<egui::Id>>,
    /// Focus this row next pass (coming back to a screen).
    refocus: Option<egui::Id>,
    /// The screen the picks are for (made afresh for another).
    picks_for: Option<Screen>,
    /// A new part's size (`PartDef::variants`) and paint.
    variant: usize,
    paint: usize,
    /// A new wheel's rim and tire: definition and size each.
    rim: (usize, Rim),
    tire: (usize, TireSize),
    /// A loose wheel's rim, among the rims brought (trunk first, then what's held).
    brought_rim: usize,
    /// The final drive's ratio as set here (asked for as it's moved).
    ratio: Option<f32>,
    /// The oversize a block is to be bored to.
    bore: usize,
}

impl Default for Fitter {
    fn default() -> Self {
        let tire = TireSize {
            width: 185,
            aspect: 60,
            rim: 14,
        };
        Self {
            screens: Vec::new(),
            came_from: Vec::new(),
            refocus: None,
            picks_for: None,
            variant: 0,
            paint: 0,
            rim: (
                0,
                Rim {
                    diameter: 14,
                    width: 60,
                },
            ),
            tire: (0, tire),
            brought_rim: 0,
            ratio: None,
            bore: 1,
        }
    }
}

impl Fitter {
    /// Whether a screen past the first is open (back goes to it, not to the car).
    pub fn deeper(&self) -> bool {
        self.screens.len() > 1
    }

    /// Back a screen: false at the first (back to the car).
    pub fn back(&mut self) -> bool {
        if self.screens.len() > 1 {
            self.pop();
            true
        } else {
            false
        }
    }

    /// Asks for the focus on the row gone on from, when back on a screen: before the
    /// window's panel claims the focus (it keeps a focus asked for before its widget is
    /// drawn).
    pub fn refocus(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.refocus.take() {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    fn push(&mut self, screen: Screen, ctx: &egui::Context) {
        let focused = ctx.memory(|m| m.focused());
        // A click doesn't focus: the row clicked then.
        let clicked = ctx.interaction_snapshot(|i| i.clicked);
        self.came_from.push(focused.or(clicked));
        self.screens.push(screen);
        // The new screen starts at its first row (rows on different screens can share
        // egui's ids, which would keep the focus where it was).
        if let Some(id) = focused {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    }

    fn pop(&mut self) {
        self.screens.pop();
        self.refocus = self.came_from.pop().flatten();
    }

    /// Back to the first screen, on the row gone on from there.
    fn back_to_first(&mut self) {
        self.screens.truncate(1);
        self.refocus = self.came_from.first().copied().flatten();
        self.came_from.clear();
    }
}

/// What the window needs besides the screens.
pub struct Bench<'a> {
    pub catalogue: &'a Catalogue,
    pub shop: &'a Shop,
    pub car: &'a Car,
    /// The car's tree with its parts as worn as the car is now (`Condition::put_on`).
    pub worn: &'a Item,
    /// What the shop holds for the player.
    pub held: &'a [Item],
    pub connection: &'a mut Connection,
}

impl<'a> Bench<'a> {
    /// The car's tree, the trunk and what's held: where paths go.
    fn tree(&self) -> (&'a Item, &'a [Item], &'a [Item]) {
        (self.worn, &self.car.setup.cargo, self.held)
    }

    fn at(&self, path: &[u8]) -> Option<&'a Item> {
        item_in(self.tree(), path)
    }

    fn kinds(&self, path: &[u8]) -> Option<(Kind, Option<Kind>)> {
        slot_kinds_in(self.catalogue, self.tree(), path)
    }

    /// What the shop sells for the slot of `kind` at `path`: made for the car's chassis
    /// and for the engine there (exhaust manifolds).
    fn wares_for(&self, kind: Kind, path: &[u8]) -> Vec<Supply> {
        let engine = engine_on(self.catalogue, self.tree(), path);
        let made = wares(self.catalogue, self.shop, kind, chassis_name(self));
        (made.into_iter())
            .filter(|ware| match ware {
                Supply::Part { def, .. } => made_for_engine(
                    self.catalogue,
                    &self.catalogue.defs[usize::from(*def)],
                    engine,
                ),
                _ => true,
            })
            .collect()
    }
}

/// How worn a part is, if it carries wear and has any: "worn 40 %" (its worst cylinder's
/// or rib's), "failed".
pub fn wear_note(item: &Item) -> Option<String> {
    let worst = item.wear.iter().copied().fold(0.0, f32::max);
    if worst >= 1.0 {
        Some("failed".into())
    } else if worst >= 0.005 {
        Some(format!("worn {:.0} %", 100.0 * worst))
    } else {
        None
    }
}

/// A part's name with how worn it is, if it is.
fn worn_name(catalogue: &Catalogue, item: &Item) -> String {
    let name = name(catalogue, item);
    match wear_note(item) {
        Some(wear) => format!("{name} ({wear})"),
        None => name,
    }
}

/// Where slots of each kind are listed on the car's screen.
fn group(kind: Kind) -> &'static str {
    match kind {
        Kind::Body(_) => "Body",
        Kind::Seat(_) => "Seats",
        Kind::Wheel | Kind::Brake => "Wheels and brakes",
        Kind::Spring | Kind::Damper | Kind::AntiRollBar => "Suspension",
        Kind::Gearbox | Kind::Clutch | Kind::FinalDrive | Kind::Differential | Kind::Adapter => {
            "Drivetrain"
        }
        Kind::EnginePart(_) => "Exhaust and cooling",
        _ => "Engine",
    }
}

const GROUPS: [&str; 7] = [
    "Engine",
    "Drivetrain",
    "Wheels and brakes",
    "Suspension",
    "Exhaust and cooling",
    "Body",
    "Seats",
];

/// What a fitted part is called in a slot's row, short (a part standing for nothing:
/// "none"; a wheel by its tire's size and rim; an engine by its block).
fn fitted_name(catalogue: &Catalogue, item: Option<&Item>) -> String {
    let Some(item) = item else {
        return "nothing".into();
    };
    let def = item.def(catalogue);
    if def.absent {
        return "none".into();
    }
    match def.does {
        Does::Wheel => {
            let rim = item.fitted(catalogue, Kind::Rim).next();
            let tire = item.fitted(catalogue, Kind::Tire).next();
            let size = tire.and_then(|t| TireSize::all().nth(t.variant));
            match (rim, size) {
                (Some(rim), Some(size)) => {
                    let rim = rim.def(catalogue).name.to_lowercase();
                    format!("{} on {}", size.label(), rim.trim_end_matches(" rim"))
                }
                _ => "incomplete".into(),
            }
        }
        _ => worn_name(catalogue, item),
    }
}

/// A new part's name, in the size picked.
fn ware_name(catalogue: &Catalogue, ware: &Supply) -> String {
    match ware {
        Supply::Part { def, .. } => catalogue.defs[usize::from(*def)].name.clone(),
        Supply::Assembly(a) => catalogue.assemblies[usize::from(*a)].name.clone(),
        _ => String::new(),
    }
}

/// The slots like the one at `path` at every corner (the car's slots of its kind, if
/// there are four, with what's under them): those on the same axle, and all four.
fn corners(catalogue: &Catalogue, car: &Item, path: &[u8]) -> Option<(Vec<Path>, Vec<Path>)> {
    let top = usize::from(*path.first()?);
    let slots = &car.def(catalogue).slots;
    let kind = slots.get(top)?.kind;
    let same: Vec<usize> = (slots.iter().enumerate())
        .filter(|(_, slot)| slot.kind == kind)
        .map(|(i, _)| i)
        .collect();
    if same.len() != 4 {
        return None;
    }
    let corner = same.iter().position(|&i| i == top)?;
    let at = |i: usize| {
        let mut p = vec![i as u8];
        p.extend_from_slice(&path[1..]);
        p
    };
    let axle = same[corner / 2 * 2..corner / 2 * 2 + 2]
        .iter()
        .map(|&i| at(i))
        .collect();
    Some((axle, same.iter().map(|&i| at(i)).collect()))
}

/// Which axle the slot at `path` is on (its corner's), if it's at a corner.
fn axle_of(catalogue: &Catalogue, car: &Item, path: &[u8]) -> usize {
    let Some(&top) = path.first() else {
        return 0;
    };
    let slots = &car.def(catalogue).slots;
    let Some(kind) = slots.get(usize::from(top)).map(|slot| slot.kind) else {
        return 0;
    };
    let before = (slots.iter().take(usize::from(top)))
        .filter(|slot| slot.kind == kind)
        .count();
    if before >= 2 { 1 } else { 0 }
}

/// The window's contents (in the shop's panel).
pub fn window(ui: &mut egui::Ui, fitter: &mut Fitter, bench: Bench) {
    if fitter.screens.is_empty() {
        fitter.screens.push(Screen::Slot(Vec::new()));
    }
    if let Some(progress) = bench.connection.baking() {
        kit::progress(ui, "Baking the new engine's maps…", progress);
    } else if bench.connection.waiting() {
        kit::note(ui, "The workshop is on it…");
    }
    let screen = fitter.screens.last().cloned().expect("a screen");
    if fitter.picks_for.as_ref() != Some(&screen) {
        pick_for(fitter, &bench, &screen);
        fitter.picks_for = Some(screen.clone());
    }
    let mut ask = None;
    let mut asks = None;
    let mut go = None;
    match &screen {
        Screen::Slot(path) if path.is_empty() => car_screen(ui, &bench, &mut ask, &mut go),
        Screen::Slot(path) => slot_screen(ui, fitter, &bench, path, &mut ask, &mut asks, &mut go),
        Screen::Ware(path, n) => ware_screen(ui, fitter, &bench, path, *n, &mut ask, &mut asks),
        Screen::NewWheel(path) => new_wheel_screen(ui, fitter, &bench, path, &mut ask),
        Screen::MakeWheel => make_wheel_screen(ui, fitter, &bench, &mut ask),
        Screen::Loose(path) => loose_screen(ui, &bench, path, &mut go),
        Screen::Rebore(path) => rebore_screen(ui, fitter, &bench, path, &mut asks),
    }
    let works = ask.map(|work| vec![work]).or(asks);
    if let Some(works) = works {
        let done_with = matches!(
            screen,
            Screen::Ware(..) | Screen::NewWheel(_) | Screen::Rebore(_)
        );
        bench.connection.refused = None;
        bench.connection.work(works);
        let in_loose = (fitter.screens.iter()).any(|screen| match screen {
            Screen::Slot(path) | Screen::Ware(path, _) | Screen::Loose(path) => loose(path),
            _ => false,
        });
        if in_loose {
            // Places in the trunk and what's held may change: back to the car.
            fitter.back_to_first();
        } else if done_with || screen == Screen::MakeWheel {
            // Back to the slot (or to the car from making a wheel): places in lists change.
            fitter.pop();
        }
    } else if let Some(screen) = go {
        fitter.push(screen, ui.ctx());
    }
}

/// Where the picks start on `screen`: the size and paint of what's fitted there.
fn pick_for(fitter: &mut Fitter, bench: &Bench, screen: &Screen) {
    let catalogue = bench.catalogue;
    let car = &bench.car.setup.car;
    fitter.paint = car.paint.unwrap_or(0);
    fitter.variant = 0;
    fitter.brought_rim = 0;
    fitter.ratio = None;
    match screen {
        Screen::Ware(path, _) => {
            if let Some(item) = bench.at(path) {
                fitter.variant = item.variant;
                fitter.paint = item.paint.unwrap_or(fitter.paint);
            }
        }
        Screen::Rebore(path) => {
            let now = bench.at(path).map_or(0, |block| block.variant);
            fitter.bore = (now + 1).min(OVERSIZES.len() - 1);
        }
        Screen::NewWheel(path) => {
            let fitted = bench.at(path);
            let wheel = fitted.and_then(|wheel| wheel_of(catalogue, wheel));
            let wheel = wheel.unwrap_or(bench.car.setup.build.wheels[0]);
            // What's on the wheel now if the shop sells it, else the first it has.
            let first = |kind: Kind| {
                let made = wares(catalogue, bench.shop, kind, chassis_name(bench));
                let defs: Vec<usize> = (made.iter())
                    .filter_map(|ware| match ware {
                        Supply::Part { def, .. } => Some(usize::from(*def)),
                        _ => None,
                    })
                    .collect();
                let now = fitted.and_then(|w| w.fitted(catalogue, kind).next());
                now.map(|item| item.def)
                    .filter(|def| defs.contains(def))
                    .or(defs.first().copied())
                    .unwrap_or(0)
            };
            fitter.rim = (first(Kind::Rim), wheel.rim);
            fitter.tire = (first(Kind::Tire), wheel.tire);
        }
        Screen::MakeWheel => {
            fitter.tire.0 = match wares(catalogue, bench.shop, Kind::Tire, "").first() {
                Some(Supply::Part { def, .. }) => usize::from(*def),
                _ => 0,
            };
        }
        _ => {}
    }
}

fn chassis_name<'a>(bench: &Bench<'a>) -> &'a str {
    &bench.catalogue.chassis[bench.car.setup.build.chassis].name
}

/// The car's slots the shop works on, by group; for a tire shop, loose wheels too.
fn car_screen(ui: &mut egui::Ui, bench: &Bench, ask: &mut Option<Work>, go: &mut Option<Screen>) {
    let catalogue = bench.catalogue;
    let car = &bench.car.setup.car;
    let slots = &car.def(catalogue).slots;
    for heading in GROUPS {
        let rows: Vec<usize> = (0..slots.len())
            .filter(|&i| {
                let kind = slots[i].kind;
                group(kind) == heading && fits_into(bench.shop.kind, kind, None)
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        kit::section(ui, heading);
        for i in rows {
            let fitted = fitted_name(catalogue, car.slots[i].as_ref());
            if kit::submenu_with(ui, &slots[i].name, &fitted).clicked() {
                *go = Some(Screen::Slot(vec![i as u8]));
            }
        }
    }
    // Parts brought with slots this shop works on (an engine or long block at an engine
    // shop).
    let works_on = |item: &Item| {
        let def = item.def(catalogue);
        (def.slots.iter()).any(|slot| fits_into(bench.shop.kind, slot.kind, Some(def.kind)))
    };
    let trunk = (bench.car.setup.cargo.iter().enumerate()).map(|(i, item)| (IN_TRUNK, i, item));
    let held = (bench.held.iter().enumerate()).map(|(i, item)| (HELD, i, item));
    let brought: Vec<_> = trunk
        .chain(held)
        .filter(|(_, _, item)| works_on(item) && bench.shop.kind == ShopKind::Engine)
        .collect();
    if bench.shop.kind == ShopKind::Engine {
        kit::section(ui, "Brought");
        if brought.is_empty() {
            kit::note(ui, "No engine or long block in the trunk or kept here.");
        }
        for (list, i, item) in brought {
            let place = if list == IN_TRUNK {
                "In the trunk"
            } else {
                "Kept here"
            };
            if kit::submenu_with(ui, place, &worn_name(catalogue, item)).clicked() {
                *go = Some(Screen::Loose(vec![list, i as u8]));
            }
        }
    }
    if bench.shop.kind == ShopKind::Tire {
        kit::section(ui, "Loose wheels");
        let trunk = &bench.car.setup.cargo;
        let mut any = false;
        for (i, item) in trunk.iter().enumerate() {
            if item.def(catalogue).kind == Kind::Wheel {
                any = true;
                let label = format!("Take apart: {}", name(catalogue, item));
                if kit::button(ui, &label).clicked() {
                    *ask = Some(Work::Split { item: i as u16 });
                }
            }
        }
        if !any {
            kit::note(ui, "None in the trunk.");
        }
        if kit::submenu(ui, "Make a wheel of a rim brought").clicked() {
            *go = Some(Screen::MakeWheel);
        }
    }
    kit::note(
        ui,
        match bench.shop.kind {
            ShopKind::Tire => {
                "Tires go onto rims and off them only here. What comes off goes into the \
                 trunk while there's room, else the shop holds it."
            }
            ShopKind::Engine => {
                "Only long blocks: what's inside them, rebores, and engines swapped (with an \
                 adapter plate on the gearbox where it needs one). What comes off goes into the trunk while there's room, else the shop keeps it \
                 (one engine and one long block for you at most). A changed engine runs as \
                 it was until its maps are worked out."
            }
            _ => {
                "What comes off goes into the trunk while there's room, else the shop holds it \
                 for you. Long blocks are the engine shop's work, rims and tires a tire \
                 shop's."
            }
        },
    );
}

/// A slot: what's fitted (and what's on it the shop works on), taking it off, new parts and
/// parts brought.
fn slot_screen(
    ui: &mut egui::Ui,
    fitter: &mut Fitter,
    bench: &Bench,
    path: &Path,
    ask: &mut Option<Work>,
    asks: &mut Option<Vec<Work>>,
    go: &mut Option<Screen>,
) {
    let catalogue = bench.catalogue;
    let holder_path = &path[..path.len() - 1];
    let (Some((kind, _)), Some(holder)) = (bench.kinds(path), bench.at(holder_path)) else {
        kit::note(ui, "That's gone.");
        return;
    };
    let slot = &holder.def(catalogue).slots[usize::from(path[path.len() - 1])];
    let fitted = bench.at(path);
    // A turbo's mount: every mount of the manifold takes one alike.
    let mounts = mounts(catalogue, bench.tree(), path);
    kit::section(ui, &slot.name);
    kit::info(ui, "Fitted", &fitted_name(catalogue, fitted));
    if kind == Kind::Turbo
        && let Some(litres) = behind_mount(bench, holder_path)
    {
        kit::info(ui, "A turbo here is sized for", &litres);
    }
    if let Some(item) = fitted.filter(|item| !item.def(catalogue).absent) {
        kit::info(
            ui,
            "Mass",
            &format!("{:.1} kg", crate::cargo::mass(catalogue, item)),
        );
        if item.def(catalogue).wear.is_some() {
            let wear = wear_note(item).unwrap_or_else(|| "as new".into());
            kit::info(ui, "Condition", &wear);
        }
        if kind == Kind::Manifold {
            manifold_readouts(ui, catalogue, item.def(catalogue));
            let all = mounts_of(catalogue, item, path);
            if !all.is_empty() {
                let on = turbos_on(catalogue, item).unwrap_or_else(|| "none".into());
                kit::info(ui, "Turbos", &on);
            }
        }
        rebore_row(ui, bench, path, item, go);
        // What's on it that this shop works on (an engine's bolt-ons, a wheel's rim and
        // tire).
        let inner = &item.def(catalogue).slots;
        for (i, sub) in inner.iter().enumerate() {
            if fits_into(bench.shop.kind, sub.kind, Some(kind)) {
                let mut deeper = path.clone();
                deeper.push(i as u8);
                let on = fitted_name(catalogue, item.slots[i].as_ref());
                if kit::submenu_with(ui, &sub.name, &on).clicked() {
                    *go = Some(Screen::Slot(deeper));
                }
            }
        }
        if let (Some(knob), Some(&now)) = (item.def(catalogue).knobs.first(), item.knobs.first())
            && kind == Kind::FinalDrive
        {
            let ratio = fitter.ratio.get_or_insert(now);
            let range = knob.low.max(2.5)..=knob.high.min(5.5);
            if kit::slider(ui, "Ratio", ratio, range, 0.05, |v| format!("{v:.2}:1")).changed() {
                *ask = Some(Work::Knob {
                    at: path.clone(),
                    knob: 0,
                    value: *ratio,
                });
            }
            kit::note(
                ui,
                "A shorter final drive (a higher number) pulls harder and tops out sooner.",
            );
        }
        let nothing = catalogue.defs.iter().any(|d| d.absent && d.kind == kind);
        let turbos = mounts_of(catalogue, item, path);
        if kind == Kind::Manifold && item.fitted(catalogue, Kind::Turbo).next().is_some() {
            // The turbos come off all together.
            if kit::button(ui, "Take the turbos off").clicked() {
                let on: Vec<Path> = turbos
                    .into_iter()
                    .filter(|p| bench.at(p).is_some())
                    .collect();
                *asks = Some(removes(&on));
            }
        }
        if mounts.len() > 1 {
            if kit::button(ui, "Take it off every mount").clicked() {
                *asks = Some(removes(&mounts));
            }
        } else if (!slot.required || nothing || loose(path))
            && kit::button(ui, "Take it off").clicked()
        {
            *ask = Some(Work::Remove { slot: path.clone() });
        }
    }
    if mounts.len() > 1 {
        kit::note(
            ui,
            &format!(
                "A turbo goes on every mount at once ({}), the same on each.",
                mounts.len()
            ),
        );
    }
    kit::section(ui, &format!("New from {}", bench.shop.name));
    let made = bench.wares_for(kind, path);
    if kind == Kind::Wheel && bench.shop.kind == ShopKind::Tire {
        if kit::submenu(ui, "A new wheel: rim and tire").clicked() {
            *go = Some(Screen::NewWheel(path.clone()));
        }
    } else if made.is_empty() {
        kit::note(ui, "Nothing for this here.");
    }
    for (n, ware) in made.iter().enumerate() {
        if kit::submenu(ui, &ware_name(catalogue, ware)).clicked() {
            *go = Some(Screen::Ware(path.clone(), n));
        }
    }
    kit::section(ui, "Brought");
    let chassis = chassis_name(bench);
    let engine = engine_on(catalogue, bench.tree(), path);
    let fits = |item: &Item| {
        let def = item.def(catalogue);
        def.kind == kind && made_for(def, chassis) && made_for_engine(catalogue, def, engine)
    };
    let trunk = &bench.car.setup.cargo;
    if mounts.len() > 1 {
        // Turbos brought go on every mount at once: as many alike as there are mounts.
        let brought: Vec<(u8, usize, &Item)> = (trunk.iter().enumerate())
            .map(|(i, item)| (IN_TRUNK, i, item))
            .chain((bench.held.iter().enumerate()).map(|(i, item)| (HELD, i, item)))
            .filter(|(_, _, item)| fits(item))
            .collect();
        let mut defs: Vec<usize> = brought.iter().map(|(_, _, item)| item.def).collect();
        defs.sort_unstable();
        defs.dedup();
        for def in defs {
            let alike: Vec<(u8, usize)> = (brought.iter())
                .filter(|(_, _, item)| item.def == def)
                .map(|&(list, i, _)| (list, i))
                .collect();
            let name = &catalogue.defs[def].name;
            if alike.len() >= mounts.len() {
                let label = format!("Fit {} brought", mounts.len());
                if kit::button_with(ui, &label, name).clicked() {
                    *asks = Some(fit_brought(&mounts, &alike));
                }
            } else {
                kit::note(
                    ui,
                    &format!(
                        "{} {name} brought: {} needed alike.",
                        alike.len(),
                        mounts.len()
                    ),
                );
            }
        }
        if brought.is_empty() {
            kit::note(ui, "Nothing for this in the trunk or held here.");
        }
        return;
    }
    let mut any = false;
    for (i, item) in trunk.iter().enumerate().filter(|(_, item)| fits(item)) {
        any = true;
        let label = worn_name(catalogue, item);
        if kit::button_with(ui, "Fit it, from the trunk", &label).clicked() {
            *ask = Some(Work::Fit {
                slots: vec![path.clone()],
                supply: Supply::Trunk(i as u16),
            });
        }
    }
    for (i, item) in bench.held.iter().enumerate().filter(|(_, item)| fits(item)) {
        any = true;
        if kit::button_with(ui, "Fit it, held here", &worn_name(catalogue, item)).clicked() {
            *ask = Some(Work::Fit {
                slots: vec![path.clone()],
                supply: Supply::Held(i as u16),
            });
        }
    }
    if !any {
        kit::note(ui, "Nothing for this in the trunk or held here.");
    }
}

/// The sizes a new part for the slot at `path` can have: for a rim or tire on a wheel,
/// those that go with the tire or rim there; anti-roll bars the chassis' sizes.
fn sizes(bench: &Bench, path: &Path, def: usize) -> Vec<usize> {
    let catalogue = bench.catalogue;
    let car = &bench.car.setup.car;
    let made = &catalogue.defs[def];
    let holder = (path.len() >= 2)
        .then(|| bench.at(&path[..path.len() - 1]))
        .flatten();
    if made.bores {
        // Pistons the size the block is bored to; a new block comes with the stock bore.
        return match holder.filter(|block| block.def(catalogue).kind == Kind::LongBlock) {
            Some(block) if made.kind != Kind::LongBlock => vec![block.variant],
            _ => vec![0],
        };
    }
    let wheel = holder;
    let axle = axle_of(catalogue, car, path);
    let room = &bench.car.setup.build.chassis().room[axle];
    match made.does {
        Does::Rim => {
            let tire = wheel.and_then(|w| w.fitted(catalogue, Kind::Tire).next());
            let tire = tire.and_then(|t| TireSize::all().nth(t.variant));
            (Rim::all().enumerate())
                .filter(|(_, rim)| tire.is_none_or(|tire| tire.fits(rim)))
                .filter(|(_, rim)| rim.diameter >= room.min_rim)
                .map(|(i, _)| i)
                .collect()
        }
        Does::Tire(compound) => {
            let rim = wheel.and_then(|w| w.fitted(catalogue, Kind::Rim).next());
            let rim = rim.and_then(|r| Rim::all().nth(r.variant));
            (TireSize::all().enumerate())
                .filter(|(_, tire)| {
                    rim.is_none_or(|rim| {
                        room.misfit(&Wheel {
                            rim,
                            tire: *tire,
                            compound,
                        })
                        .is_none()
                    })
                })
                .map(|(i, _)| i)
                .collect()
        }
        _ => (0..made
            .variants(catalogue, Some(bench.car.setup.build.chassis))
            .max(1))
            .collect(),
    }
}

/// A size's name.
fn size_name(bench: &Bench, def: usize, variant: usize) -> String {
    let catalogue = bench.catalogue;
    if catalogue.defs[def].bores {
        return bore_name(variant);
    }
    match catalogue.defs[def].does {
        Does::Rim => Rim::all()
            .nth(variant)
            .map_or_else(String::new, |r| r.label()),
        Does::Tire(_) => TireSize::all()
            .nth(variant)
            .map_or_else(String::new, |t| t.label()),
        Does::AntiRollBar(a) => bench.car.setup.build.chassis().bars[a.min(1)].label(variant),
        _ => String::new(),
    }
}

/// Fit buttons for `supply`: here, and at the axle's or every corner when it's a corner's.
fn fit_buttons(
    ui: &mut egui::Ui,
    bench: &Bench,
    path: &Path,
    supply: Supply,
    ask: &mut Option<Work>,
) {
    let car = &bench.car.setup.car;
    // A turbo goes on every mount of its manifold at once.
    let mounts = mounts(bench.catalogue, bench.tree(), path);
    if mounts.len() > 1 {
        if kit::button(ui, "Fit one on every mount").clicked() {
            *ask = Some(Work::Fit {
                slots: mounts,
                supply,
            });
        }
        return;
    }
    if kit::button(ui, "Fit it here").clicked() {
        *ask = Some(Work::Fit {
            slots: vec![path.clone()],
            supply: supply.clone(),
        });
    }
    if let Some((axle, all)) = corners(bench.catalogue, car, path) {
        if kit::button(ui, "Fit both on this axle").clicked() {
            *ask = Some(Work::Fit {
                slots: axle,
                supply: supply.clone(),
            });
        }
        if kit::button(ui, "Fit all four").clicked() {
            *ask = Some(Work::Fit { slots: all, supply });
        }
    }
}

/// A new part: its size and paint, and fitting it.
fn ware_screen(
    ui: &mut egui::Ui,
    fitter: &mut Fitter,
    bench: &Bench,
    path: &Path,
    n: usize,
    ask: &mut Option<Work>,
    asks: &mut Option<Vec<Work>>,
) {
    let catalogue = bench.catalogue;
    let car = &bench.car.setup.car;
    let Some((kind, _)) = bench.kinds(path) else {
        return;
    };
    let made = bench.wares_for(kind, path);
    let Some(ware) = made.get(n) else {
        return;
    };
    kit::section(ui, &ware_name(catalogue, ware));
    let supply = match *ware {
        Supply::Part { def, .. } => {
            let d = usize::from(def);
            let made = &catalogue.defs[d];
            let options = sizes(bench, path, d);
            if made.bores {
                // Only the one size that goes here.
                fitter.variant = options.first().copied().unwrap_or(0);
                kit::info(ui, "Size", &size_name(bench, d, fitter.variant));
            } else if options.len() > 1 || options.first().is_some_and(|&v| v != 0) {
                if !options.contains(&fitter.variant) {
                    fitter.variant = options[0];
                }
                kit::pick(ui, "Size", &mut fitter.variant, &options, |v| {
                    size_name(bench, d, v)
                });
            } else if options.is_empty() {
                kit::warning(ui, "No size of it goes here.");
                return;
            }
            if made.painted {
                let paints = new_paints(catalogue, car);
                if !paints.contains(&fitter.paint) {
                    fitter.paint = paints[0];
                }
                kit::pick(ui, "Paint", &mut fitter.paint, &paints, |p| {
                    catalogue.paints[p].name.clone()
                });
            }
            kit::info(ui, "Mass", &format!("{:.1} kg", made.mass));
            if made.kind == Kind::Manifold {
                manifold_readouts(ui, catalogue, made);
                let mounts = (made.slots.iter())
                    .filter(|s| s.kind == Kind::Turbo)
                    .count();
                if let Some(old) = bench.at(path)
                    && let Some(on) = turbos_on(catalogue, old)
                {
                    let fitted = old.fitted(catalogue, Kind::Turbo).count();
                    kit::note(
                        ui,
                        &if fitted == mounts {
                            format!("The {on} on the one fitted move over onto it.")
                        } else {
                            format!(
                                "The {on} on the one fitted come off with it: a turbo goes \
                                 on every mount, and it has {mounts}."
                            )
                        },
                    );
                }
            }
            if made.kind == Kind::Turbo
                && let Some(litres) = behind_mount(bench, &path[..path.len() - 1])
            {
                kit::info(ui, "Sized for", &format!("{litres} behind each mount"));
            }
            Supply::Part {
                def,
                variant: fitter.variant as u16,
                paint: fitter.paint as u16,
            }
        }
        Supply::Assembly(a) => {
            let assembly = &catalogue.assemblies[usize::from(a)];
            kit::info(
                ui,
                "Mass",
                &format!("{:.0} kg", crate::cargo::mass(catalogue, &assembly.item)),
            );
            for item in assembly.item.slots.iter().flatten() {
                kit::note(ui, &name(catalogue, item));
            }
            kit::note(
                ui,
                "The ECU stays with the car: tune it for the new engine at a dyno.",
            );
            ware.clone()
        }
        _ => return,
    };
    if !bolts_after(ui, bench, path, &supply, asks) {
        fit_buttons(ui, bench, path, supply, ask);
    }
}

/// Whether the gearbox still bolts to the engine with the new part `supply` at `path`
/// (`items::drivetrain_misfit`). If not, says why
/// and offers fitting it along with what makes it bolt, if the shop can do that.
fn bolts_after(
    ui: &mut egui::Ui,
    bench: &Bench,
    path: &Path,
    supply: &Supply,
    asks: &mut Option<Vec<Work>>,
) -> bool {
    let catalogue = bench.catalogue;
    let part = match *supply {
        Supply::Part {
            def,
            variant,
            paint,
        } => {
            let mut part = Item::new(catalogue, usize::from(def), usize::from(paint));
            part.variant = usize::from(variant);
            part
        }
        Supply::Assembly(a) => catalogue.assemblies[usize::from(a)].item.clone(),
        _ => return true,
    };
    if loose(path) {
        return true;
    }
    let Some(after) = with_fitted(catalogue, bench.worn, path, part) else {
        return true;
    };
    let Some(misbolt) = drivetrain_misfit(catalogue, &after) else {
        return true;
    };
    kit::warning(ui, &misbolt.why);
    let fit = Work::Fit {
        slots: vec![path.clone()],
        supply: supply.clone(),
    };
    let plate = adapter_slot(catalogue, &after)
        .filter(|_| fits_into(bench.shop.kind, Kind::Adapter, Some(Kind::Gearbox)));
    match (misbolt.fix, plate) {
        (Some(Fix::Fit(a)), Some(plate)) => {
            let adapter = &catalogue.adapters[a];
            let def = (catalogue.defs.iter()).position(|def| def.does == Does::Adapter(a));
            match def.filter(|_| bench.shop.sells(&adapter.id)) {
                Some(def) => {
                    if kit::button_with(ui, "Fit it with the adapter plate", &adapter.name)
                        .clicked()
                    {
                        let supply = Supply::Part {
                            def: def as u16,
                            variant: 0,
                            paint: 0,
                        };
                        let plate = Work::Fit {
                            slots: vec![plate],
                            supply,
                        };
                        *asks = Some(vec![fit, plate]);
                    }
                }
                None => kit::note(ui, &format!("{} hasn't got that plate.", bench.shop.name)),
            }
        }
        (Some(Fix::Remove), Some(plate))
            if kit::button(ui, "Fit it and take the adapter plate off").clicked() =>
        {
            *asks = Some(vec![fit, Work::Remove { slot: plate }]);
        }
        _ => {}
    }
    false
}

/// A new wheel for the slot at `path`: rim (kind, diameter, width) and tire (kind and a size
/// that goes on the rim and in the wheel well).
fn new_wheel_screen(
    ui: &mut egui::Ui,
    fitter: &mut Fitter,
    bench: &Bench,
    path: &Path,
    ask: &mut Option<Work>,
) {
    let catalogue = bench.catalogue;
    let chassis = chassis_name(bench);
    let defs = |kind| -> Vec<usize> {
        (wares(catalogue, bench.shop, kind, chassis).iter())
            .filter_map(|ware| match ware {
                Supply::Part { def, .. } => Some(usize::from(*def)),
                _ => None,
            })
            .collect()
    };
    let (rims, tires) = (defs(Kind::Rim), defs(Kind::Tire));
    if rims.is_empty() || tires.is_empty() {
        kit::note(ui, "This shop hasn't got rims and tires.");
        return;
    }
    let axle = axle_of(catalogue, &bench.car.setup.car, path);
    let room = &bench.car.setup.build.chassis().room[axle];
    kit::section(ui, "Rim");
    let name = |d: usize| catalogue.defs[d].name.clone();
    kit::pick(ui, "Rim", &mut fitter.rim.0, &rims, name);
    let diameters: Vec<u8> = wheels::RIM_DIAMETERS
        .filter(|&d| d >= room.min_rim)
        .collect();
    let rim = &mut fitter.rim.1;
    if kit::pick(ui, "Diameter", &mut rim.diameter, &diameters, |d| {
        format!("{d}\"")
    }) {
        // The tire as near its outside diameter as it goes.
        let tire = fitter.tire.1;
        fitter.tire.1 = TireSize::nearest(tire.width, rim.diameter, 2.0 * tire.radius());
        rim.width = fitter.tire.1.rim_width_near(rim.width);
    }
    let widths: Vec<u8> = wheels::RIM_WIDTHS.step_by(wheels::RIM_WIDTH_STEP).collect();
    kit::pick(ui, "Width", &mut rim.width, &widths, |w| {
        format!("{:.1}\"", f32::from(w) / 10.0)
    });
    let rim = *rim;
    kit::section(ui, "Tire");
    kit::pick(ui, "Tire", &mut fitter.tire.0, &tires, name);
    let Does::Tire(compound) = catalogue.defs[fitter.tire.0].does else {
        return;
    };
    let fitting: Vec<TireSize> = TireSize::all()
        .filter(|&tire| {
            room.misfit(&Wheel {
                rim,
                tire,
                compound,
            })
            .is_none()
        })
        .collect();
    if fitting.is_empty() {
        kit::warning(ui, "No tire goes on that rim in this wheel well.");
        return;
    }
    if !fitting.contains(&fitter.tire.1) {
        fitter.tire.1 = fitting[0];
    }
    kit::pick(ui, "Size", &mut fitter.tire.1, &fitting, |t| t.label());
    kit::note(ui, &catalogue.compounds[compound].about);
    let (Some(r), Some(t)) = (
        Rim::all().position(|r| r == rim),
        TireSize::all().position(|t| t == fitter.tire.1),
    ) else {
        return;
    };
    let supply = Supply::Wheel(
        Box::new(Supply::Part {
            def: fitter.rim.0 as u16,
            variant: r as u16,
            paint: 0,
        }),
        Box::new(Supply::Part {
            def: fitter.tire.0 as u16,
            variant: t as u16,
            paint: 0,
        }),
    );
    fit_buttons(ui, bench, path, supply, ask);
}

/// A loose wheel, of a rim brought and a tire brought or new, into the trunk.
fn make_wheel_screen(
    ui: &mut egui::Ui,
    fitter: &mut Fitter,
    bench: &Bench,
    ask: &mut Option<Work>,
) {
    let catalogue = bench.catalogue;
    let trunk = &bench.car.setup.cargo;
    let of = |kind: Kind| -> Vec<(Supply, &Item)> {
        let trunk = (trunk.iter().enumerate())
            .filter(|(_, item)| item.def(catalogue).kind == kind)
            .map(|(i, item)| (Supply::Trunk(i as u16), item));
        let held = (bench.held.iter().enumerate())
            .filter(|(_, item)| item.def(catalogue).kind == kind)
            .map(|(i, item)| (Supply::Held(i as u16), item));
        trunk.chain(held).collect()
    };
    let rims = of(Kind::Rim);
    if rims.is_empty() {
        kit::note(ui, "No rim in the trunk or held here.");
        return;
    }
    fitter.brought_rim = fitter.brought_rim.min(rims.len() - 1);
    let choices: Vec<usize> = (0..rims.len()).collect();
    kit::pick(ui, "Rim", &mut fitter.brought_rim, &choices, |i| {
        name(catalogue, rims[i].1)
    });
    let (rim_supply, rim_item) = &rims[fitter.brought_rim];
    let Some(rim) = Rim::all().nth(rim_item.variant) else {
        return;
    };
    kit::section(ui, "Tire");
    for (supply, tire) in of(Kind::Tire) {
        let fits = TireSize::all()
            .nth(tire.variant)
            .is_some_and(|t| t.fits(&rim));
        if fits && kit::button_with(ui, "With this tire", &name(catalogue, tire)).clicked() {
            *ask = Some(Work::MakeWheel(Supply::Wheel(
                Box::new(rim_supply.clone()),
                Box::new(supply),
            )));
        }
    }
    let tires: Vec<usize> = (wares(catalogue, bench.shop, Kind::Tire, "").iter())
        .filter_map(|ware| match ware {
            Supply::Part { def, .. } => Some(usize::from(*def)),
            _ => None,
        })
        .collect();
    if tires.is_empty() {
        return;
    }
    if !tires.contains(&fitter.tire.0) {
        fitter.tire.0 = tires[0];
    }
    kit::pick(ui, "New tire", &mut fitter.tire.0, &tires, |d| {
        catalogue.defs[d].name.clone()
    });
    let fitting: Vec<usize> = (TireSize::all().enumerate())
        .filter(|(_, t)| t.fits(&rim))
        .map(|(i, _)| i)
        .collect();
    if fitting.is_empty() {
        return;
    }
    if !fitting.contains(&fitter.variant) {
        fitter.variant = fitting[0];
    }
    kit::pick(ui, "Size", &mut fitter.variant, &fitting, |v| {
        TireSize::all()
            .nth(v)
            .map_or_else(String::new, |t| t.label())
    });
    if kit::button(ui, "Make it, with a new tire").clicked() {
        *ask = Some(Work::MakeWheel(Supply::Wheel(
            Box::new(rim_supply.clone()),
            Box::new(Supply::Part {
                def: fitter.tire.0 as u16,
                variant: fitter.variant as u16,
                paint: 0,
            }),
        )));
    }
    kit::note(ui, "The wheel goes into the trunk.");
}

/// A part brought (an engine or long block at an engine shop): its slots the shop works
/// on, and a rebore if it's a block.
fn loose_screen(ui: &mut egui::Ui, bench: &Bench, path: &Path, go: &mut Option<Screen>) {
    let catalogue = bench.catalogue;
    let Some(item) = bench.at(path) else {
        kit::note(ui, "That's gone.");
        return;
    };
    let def = item.def(catalogue);
    kit::section(ui, &worn_name(catalogue, item));
    kit::info(
        ui,
        "Where",
        if path.first() == Some(&IN_TRUNK) {
            "In the trunk"
        } else {
            "Kept here"
        },
    );
    rebore_row(ui, bench, path, item, go);
    for (i, slot) in def.slots.iter().enumerate() {
        if fits_into(bench.shop.kind, slot.kind, Some(def.kind)) {
            let mut deeper = path.clone();
            deeper.push(i as u8);
            let on = fitted_name(catalogue, item.slots[i].as_ref());
            if kit::submenu_with(ui, &slot.name, &on).clicked() {
                *go = Some(Screen::Slot(deeper));
            }
        }
    }
    kit::note(
        ui,
        "A loose engine may lack what it needs to run: fit it all before it goes in.",
    );
}

/// For a block at an engine shop: its bore, and a rebore.
fn rebore_row(ui: &mut egui::Ui, bench: &Bench, path: &Path, item: &Item, go: &mut Option<Screen>) {
    let def = item.def(bench.catalogue);
    if def.kind != Kind::LongBlock || !def.bores || bench.shop.kind != ShopKind::Engine {
        return;
    }
    let bore = bore_name(item.variant);
    if item.variant + 1 < OVERSIZES.len() {
        if kit::submenu_with(ui, "Rebore", &bore).clicked() {
            *go = Some(Screen::Rebore(path.clone()));
        }
    } else {
        kit::info(ui, "Bore", &format!("{bore} (as far as it goes)"));
    }
}

/// The slot of a block's pistons (the parts that come in its bore sizes).
fn pistons_slot(catalogue: &Catalogue, block: &Item) -> Option<usize> {
    (block.def(catalogue).slots.iter())
        .position(|slot| (catalogue.defs.iter()).any(|def| def.kind == slot.kind && def.bores))
}

/// A block bored over: the oversize, then pistons of it (new from the shop, or brought).
fn rebore_screen(
    ui: &mut egui::Ui,
    fitter: &mut Fitter,
    bench: &Bench,
    path: &Path,
    asks: &mut Option<Vec<Work>>,
) {
    let catalogue = bench.catalogue;
    let Some(block) = bench.at(path) else {
        kit::note(ui, "That's gone.");
        return;
    };
    let Some(slot) = pistons_slot(catalogue, block) else {
        return;
    };
    kit::section(ui, "Rebore");
    kit::info(ui, "Now", &bore_name(block.variant));
    let sizes: Vec<usize> = (block.variant + 1..OVERSIZES.len()).collect();
    if sizes.is_empty() {
        kit::note(ui, "It's bored as far as it goes.");
        return;
    }
    if !sizes.contains(&fitter.bore) {
        fitter.bore = sizes[0];
    }
    kit::pick(ui, "Bored to", &mut fitter.bore, &sizes, bore_name);
    let size = fitter.bore;
    let pistons = (block.def(catalogue).slots.get(slot)).map(|s| s.kind);
    let Some(pistons) = pistons else {
        return;
    };
    let mut pistons_path = path.clone();
    pistons_path.push(slot as u8);
    let rebore = Work::Rebore {
        at: path.clone(),
        size: size as u8,
    };
    let fit = |supply: Supply| Work::Fit {
        slots: vec![pistons_path.clone()],
        supply,
    };
    kit::section(ui, "With pistons of that size");
    for ware in wares(catalogue, bench.shop, pistons, chassis_name(bench)) {
        let Supply::Part { def, .. } = ware else {
            continue;
        };
        let label = format!("Rebore, new {}", catalogue.defs[usize::from(def)].name);
        if kit::button(ui, &label).clicked() {
            let supply = Supply::Part {
                def,
                variant: size as u16,
                paint: 0,
            };
            *asks = Some(vec![rebore.clone(), fit(supply)]);
        }
    }
    let brought = (bench.car.setup.cargo.iter().enumerate())
        .map(|(i, item)| (Supply::Trunk(i as u16), item))
        .chain((bench.held.iter().enumerate()).map(|(i, item)| (Supply::Held(i as u16), item)));
    for (supply, item) in brought {
        let def = item.def(catalogue);
        if def.kind == pistons && def.bores && item.variant == size {
            let label = format!("Rebore, fit {}", worn_name(catalogue, item));
            if kit::button(ui, &label).clicked() {
                *asks = Some(vec![rebore.clone(), fit(supply)]);
            }
        }
    }
    kit::note(
        ui,
        "A bigger bore makes a bigger engine; the block takes only pistons of its size from \
         then on, and is never bored smaller again. The old pistons come out.",
    );
}

/// What an exhaust manifold is, from its definition: its primaries, how it gathers the
/// cylinders, what they end in and its turbo mounts.
fn manifold_readouts(ui: &mut egui::Ui, catalogue: &Catalogue, def: &crate::items::PartDef) {
    let Does::Manifold { family, manifold } = def.does else {
        return;
    };
    let Some(family) = catalogue.families.get(family) else {
        return;
    };
    let Some(made) = family.manifolds.get(manifold) else {
        return;
    };
    let vee = family.layout.is_vee();
    let cylinders = family.versions.first().map_or(0, |v| v.block.cylinders);
    let m = &made.manifold;
    kit::info(
        ui,
        "Primaries",
        &format!(
            "{:.0} mm{}",
            m.primary_length * 1000.0,
            if made.cast { ", cast" } else { ", tubes" }
        ),
    );
    let per = cylinders / m.branches.max(1);
    let branches = match m.branches {
        1 => format!("all {cylinders} cylinders into one"),
        2 if vee => format!("each bank's {per} into its own"),
        n => format!("{n} of {per} cylinders"),
    };
    kit::info(ui, "Branches", &branches);
    let collector = match (m.collector, m.branches) {
        (Collector::Tuned, 1) => "a tuned collector".to_string(),
        (Collector::Tuned, n) => format!("{n} tuned collectors"),
        (Collector::Turbine, 1) => "a turbine inlet".to_string(),
        (Collector::Turbine, n) => format!("{n} turbine inlets"),
    };
    kit::info(ui, "Into", &collector);
    let mounts = match made.mounts() {
        0 => "none (no turbo goes on it)".to_string(),
        2 if vee => "2, one on each bank".to_string(),
        n => n.to_string(),
    };
    kit::info(ui, "Turbo mounts", &mounts);
}

/// How much engine a turbo on a mount of the manifold at `manifold` (a path) is sized for:
/// the displacement behind the mount, "4.0 l".
fn behind_mount(bench: &Bench, manifold: &[u8]) -> Option<String> {
    let catalogue = bench.catalogue;
    let (family, version) = engine_on(catalogue, bench.tree(), manifold)?;
    let item = bench.at(manifold)?;
    let Does::Manifold { manifold, .. } = item.def(catalogue).does else {
        return None;
    };
    let family = catalogue.families.get(family)?;
    let branches = family.manifolds.get(manifold)?.manifold.branches.max(1);
    let litres = family.versions.get(version)?.block.displacement() * 1000.0 / branches as f32;
    Some(format!("{litres:.1} l"))
}

/// The turbos on a manifold: "2 × Medium (turbo)", or none.
fn turbos_on(catalogue: &Catalogue, manifold: &Item) -> Option<String> {
    let turbos: Vec<&Item> = manifold.fitted(catalogue, Kind::Turbo).collect();
    let first = turbos.first()?;
    let name = worn_name(catalogue, first);
    Some(match turbos.len() {
        1 => name,
        n => format!("{n} × {name}"),
    })
}

/// Works putting turbos brought (alike, `picks` in the trunk and held here) on `mounts`:
/// the highest places taken first, so the others stay where they are.
fn fit_brought(mounts: &[Path], picks: &[(u8, usize)]) -> Vec<Work> {
    let mut picks = picks.to_vec();
    picks.sort_by_key(|&(list, i)| std::cmp::Reverse((list, i)));
    (mounts.iter().zip(picks))
        .map(|(mount, (list, i))| Work::Fit {
            slots: vec![mount.clone()],
            supply: if list == IN_TRUNK {
                Supply::Trunk(i as u16)
            } else {
                Supply::Held(i as u16)
            },
        })
        .collect()
}

/// The paths of the turbo mounts of `manifold`, at `path`.
fn mounts_of(catalogue: &Catalogue, manifold: &Item, path: &[u8]) -> Vec<Path> {
    (manifold.def(catalogue).slots.iter().enumerate())
        .filter(|(_, slot)| slot.kind == Kind::Turbo)
        .map(|(i, _)| {
            let mut mount = path.to_vec();
            mount.push(i as u8);
            mount
        })
        .collect()
}

/// Works taking off what's in each of `slots`.
fn removes(slots: &[Path]) -> Vec<Work> {
    (slots.iter())
        .map(|slot| Work::Remove { slot: slot.clone() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turbos_brought_are_taken_from_the_end_of_their_lists_first() {
        let mounts = [vec![3, 2, 0], vec![3, 2, 1]];
        let works = fit_brought(&mounts, &[(IN_TRUNK, 1), (HELD, 0), (IN_TRUNK, 4)]);
        let supplies: Vec<Supply> = (works.into_iter())
            .map(|work| match work {
                Work::Fit { supply, .. } => supply,
                _ => panic!("not a fit"),
            })
            .collect();
        // Two mounts, two turbos: what's held first (a list of its own), then the trunk's
        // later one, so the places of the others don't move.
        assert_eq!(supplies, [Supply::Held(0), Supply::Trunk(4)]);
    }
}
