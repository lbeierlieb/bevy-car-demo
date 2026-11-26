//! The window of car workshops, tuning shops and tire shops (in the workshop, `bay`): the
//! car's slots the shop works on, and for each what's fitted, taking it off, new parts the
//! shop sells (in a size and paint), parts brought in the trunk or held here. Tire shops
//! also make new wheels of a rim and a tire, make loose wheels and take them apart. Every
//! change is asked of the server (`fitting::Work`), which answers with the car.

use bevy_egui::egui;

use crate::car::Car;
use crate::catalogue::Catalogue;
use crate::fitting::{
    Path, Supply, Work, fits_into, item_at, made_for, new_paints, slot_kinds, wares, wheel_of,
};
use crate::items::{Does, Item, Kind};
use crate::menu::kit;
use crate::net::Connection;
use crate::owned::name;
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
}

/// The screens open, and what's picked on them.
pub struct Fitter {
    screens: Vec<Screen>,
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
        }
    }
}

impl Fitter {
    /// Back a screen: false at the first (back to the car).
    pub fn back(&mut self) -> bool {
        if self.screens.len() > 1 {
            self.screens.pop();
            true
        } else {
            false
        }
    }
}

/// What the window needs besides the screens.
pub struct Bench<'a> {
    pub catalogue: &'a Catalogue,
    pub shop: &'a Shop,
    pub car: &'a Car,
    /// What the shop holds for the player.
    pub held: &'a [Item],
    pub connection: &'a mut Connection,
}

/// Where slots of each kind are listed on the car's screen.
fn group(kind: Kind) -> &'static str {
    match kind {
        Kind::Body(_) => "Body",
        Kind::Seat(_) => "Seats",
        Kind::Wheel | Kind::Brake => "Wheels and brakes",
        Kind::Spring | Kind::Damper | Kind::AntiRollBar => "Suspension",
        Kind::Gearbox | Kind::Clutch | Kind::FinalDrive | Kind::Differential => "Drivetrain",
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
        _ => name(catalogue, item),
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
    let mut go = None;
    match &screen {
        Screen::Slot(path) if path.is_empty() => car_screen(ui, &bench, &mut ask, &mut go),
        Screen::Slot(path) => slot_screen(ui, fitter, &bench, path, &mut ask, &mut go),
        Screen::Ware(path, n) => ware_screen(ui, fitter, &bench, path, *n, &mut ask),
        Screen::NewWheel(path) => new_wheel_screen(ui, fitter, &bench, path, &mut ask),
        Screen::MakeWheel => make_wheel_screen(ui, fitter, &bench, &mut ask),
    }
    if let Some(work) = ask {
        let done_with = matches!(screen, Screen::Ware(..) | Screen::NewWheel(_));
        bench.connection.refused = None;
        bench.connection.work(vec![work]);
        // Back to the slot (or to the car from making a wheel): places in lists change.
        if done_with || screen == Screen::MakeWheel {
            fitter.screens.pop();
        }
    } else if let Some(screen) = go {
        fitter.screens.push(screen);
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
            if let Some(item) = item_at(car, path) {
                fitter.variant = item.variant;
                fitter.paint = item.paint.unwrap_or(fitter.paint);
            }
        }
        Screen::NewWheel(path) => {
            let fitted = item_at(car, path);
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
    go: &mut Option<Screen>,
) {
    let catalogue = bench.catalogue;
    let car = &bench.car.setup.car;
    let (Some((kind, _)), Some(holder)) = (
        slot_kinds(catalogue, car, path),
        item_at(car, &path[..path.len() - 1]),
    ) else {
        kit::note(ui, "That's gone.");
        return;
    };
    let slot = &holder.def(catalogue).slots[usize::from(path[path.len() - 1])];
    let fitted = item_at(car, path);
    kit::section(ui, &slot.name);
    kit::info(ui, "Fitted", &fitted_name(catalogue, fitted));
    if let Some(item) = fitted.filter(|item| !item.def(catalogue).absent) {
        kit::info(
            ui,
            "Mass",
            &format!("{:.1} kg", crate::cargo::mass(catalogue, item)),
        );
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
        if (!slot.required || nothing) && kit::button(ui, "Take it off").clicked() {
            *ask = Some(Work::Remove { slot: path.clone() });
        }
    }
    kit::section(ui, &format!("New from {}", bench.shop.name));
    let made = wares(catalogue, bench.shop, kind, chassis_name(bench));
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
    let fits = |item: &Item| {
        let def = item.def(catalogue);
        def.kind == kind && made_for(def, chassis)
    };
    let trunk = &bench.car.setup.cargo;
    let mut any = false;
    for (i, item) in trunk.iter().enumerate().filter(|(_, item)| fits(item)) {
        any = true;
        if kit::button_with(ui, "Fit it, from the trunk", &name(catalogue, item)).clicked() {
            *ask = Some(Work::Fit {
                slots: vec![path.clone()],
                supply: Supply::Trunk(i as u16),
            });
        }
    }
    for (i, item) in bench.held.iter().enumerate().filter(|(_, item)| fits(item)) {
        any = true;
        if kit::button_with(ui, "Fit it, held here", &name(catalogue, item)).clicked() {
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
    let wheel = (path.len() >= 2)
        .then(|| item_at(car, &path[..path.len() - 1]))
        .flatten();
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
) {
    let catalogue = bench.catalogue;
    let car = &bench.car.setup.car;
    let Some((kind, _)) = slot_kinds(catalogue, car, path) else {
        return;
    };
    let made = wares(catalogue, bench.shop, kind, chassis_name(bench));
    let Some(ware) = made.get(n) else {
        return;
    };
    kit::section(ui, &ware_name(catalogue, ware));
    let supply = match *ware {
        Supply::Part { def, .. } => {
            let d = usize::from(def);
            let made = &catalogue.defs[d];
            let options = sizes(bench, path, d);
            if options.len() > 1 || options.first().is_some_and(|&v| v != 0) {
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
    fit_buttons(ui, bench, path, supply, ask);
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
