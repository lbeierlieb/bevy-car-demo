//! The built-in catalogue's part definitions and trims: everything the hot hatch is
//! built of, and what can go onto it.

use std::sync::Arc;

use super::{Catalogue, Shared};
use crate::chassis::CarBuild;
use crate::items::{
    Assembly, Does, Item, Kind, Knob, Panel, PartDef, Seat, SlotDef, Trim, Wear, assemble,
    chassis_knobs, damper_knobs, engine_knobs, knob, tire_knobs,
};
use crate::physics::engine::ecu::EcuModel;
use crate::physics::tire::STREET;
use crate::wheels::Wheel;

/// Where the catalogue's engine slots are: in the long block, on the engine, or in the
/// car (by their places in `slots`).
pub const LONG_BLOCK_SLOTS: [usize; 9] = [0, 1, 2, 3, 4, 5, 6, 7, 19];
pub const ENGINE_SLOTS: [usize; 6] = [8, 9, 10, 11, 12, 18];
pub const CAR_SLOTS: [usize; 5] = [16, 14, 15, 17, 13];

/// `catalogue` with its part definitions and trims.
pub fn with_items(catalogue: Catalogue) -> Catalogue {
    let mut catalogue = catalogue;
    catalogue.defs = defs(&catalogue);
    let shared: Shared = Arc::new(catalogue.clone());
    catalogue.trims = trims(&shared);
    catalogue.assemblies = engines(&catalogue);
    catalogue.rehash();
    catalogue
}

/// Lower case with dashes, for ids: "Low compression" is "low-compression".
fn slug(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

struct Def {
    id: String,
    name: String,
    kind: Kind,
    does: Does,
    fits: Vec<String>,
    size: [f32; 3],
    mass: f32,
    painted: bool,
    absent: bool,
    wear: Option<Wear>,
    bores: bool,
    knobs: Vec<Knob>,
    slots: Vec<SlotDef>,
}

impl Def {
    fn new(id: &str, name: &str, kind: Kind, does: Does, size: [f32; 3], mass: f32) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            does,
            fits: Vec::new(),
            size,
            mass,
            painted: false,
            absent: false,
            wear: None,
            bores: false,
            knobs: Vec::new(),
            slots: Vec::new(),
        }
    }

    fn fits(mut self, chassis: &str) -> Self {
        self.fits.push(chassis.into());
        self
    }

    fn painted(mut self) -> Self {
        self.painted = true;
        self
    }

    fn wears(mut self, wear: Wear) -> Self {
        self.wear = Some(wear);
        self
    }

    fn bored(mut self) -> Self {
        self.bores = true;
        self
    }

    fn knobs(mut self, knobs: Vec<Knob>) -> Self {
        self.knobs = knobs;
        self
    }

    fn slots(mut self, slots: Vec<SlotDef>) -> Self {
        self.slots = slots;
        self
    }

    fn done(self) -> PartDef {
        PartDef {
            id: self.id,
            name: self.name,
            kind: self.kind,
            does: self.does,
            fits: self.fits,
            size: self.size,
            mass: self.mass,
            price: 0,
            painted: self.painted,
            absent: self.absent,
            wear: self.wear,
            bores: self.bores,
            knobs: self.knobs,
            slots: self.slots,
        }
    }
}

fn slot(name: &str, kind: Kind, required: bool) -> SlotDef {
    SlotDef {
        name: name.into(),
        kind,
        required,
        needs: None,
    }
}

/// `slot` needing the slot at `place` filled.
fn needing(slot: SlotDef, place: usize) -> SlotDef {
    SlotDef {
        needs: Some(place),
        ..slot
    }
}

/// The hot hatch's slots: body, seats, a wheel, brake, spring and damper at each corner,
/// anti-roll bars, the drivetrain, the exhaust after the manifold, cooling, the ECU and the
/// engine.
fn hatch_slots(catalogue: &Catalogue) -> Vec<SlotDef> {
    let body = |name: &str, panel| slot(name, Kind::Body(panel), false);
    // A lip goes on the front bumper, mirrors on the doors, the spoiler on the hatch, the
    // diffuser under the rear bumper (by their places here).
    let mut slots = vec![
        body("Front bumper", Panel::FrontBumper),
        needing(body("Front lip", Panel::FrontLip), 0),
        body("Bonnet", Panel::Bonnet),
        body("Left fender", Panel::Fender),
        body("Right fender", Panel::Fender),
        body("Left door", Panel::Door),
        body("Right door", Panel::Door),
        needing(body("Left mirror", Panel::Mirror), 5),
        needing(body("Right mirror", Panel::Mirror), 6),
        body("Left side skirt", Panel::SideSkirt),
        body("Right side skirt", Panel::SideSkirt),
        body("Hatch", Panel::Hatch),
        needing(body("Rear spoiler", Panel::RearSpoiler), 11),
        body("Rear bumper", Panel::RearBumper),
        needing(body("Diffuser", Panel::Diffuser), 13),
        slot("Driver's seat", Kind::Seat(Seat::Front), true),
        slot("Passenger seat", Kind::Seat(Seat::Front), false),
        slot("Rear bench", Kind::Seat(Seat::RearBench), false),
    ];
    for corner in crate::items::CORNERS {
        slots.push(slot(&format!("Wheel, {corner}"), Kind::Wheel, true));
        slots.push(slot(&format!("Brake, {corner}"), Kind::Brake, true));
        slots.push(slot(&format!("Spring, {corner}"), Kind::Spring, true));
        slots.push(slot(&format!("Damper, {corner}"), Kind::Damper, true));
    }
    slots.extend([
        slot("Front anti-roll bar", Kind::AntiRollBar, true),
        slot("Rear anti-roll bar", Kind::AntiRollBar, true),
        slot("Gearbox", Kind::Gearbox, true),
        slot("Clutch", Kind::Clutch, true),
    ]);
    for s in CAR_SLOTS {
        slots.push(slot(&catalogue.slots[s].name, Kind::EnginePart(s), true));
    }
    slots.extend([
        slot("ECU", Kind::Ecu, true),
        slot("Engine", Kind::Engine, true),
    ]);
    slots
}

/// The box a part of the catalogue's engine slot `slot` is packed in (m) and its mass
/// (kg), roughly.
fn engine_part_size(name: &str) -> ([f32; 3], f32) {
    match name {
        "Pistons" => ([0.4, 0.3, 0.15], 2.0),
        "Crankshaft" => ([0.6, 0.2, 0.2], 14.0),
        "Connecting rods" | "Internals" => ([0.4, 0.3, 0.15], 3.0),
        "Head gasket" => ([0.5, 0.2, 0.02], 0.3),
        "Camshafts" => ([0.6, 0.1, 0.1], 4.0),
        "Valve springs" | "Cam phasers" => ([0.3, 0.2, 0.1], 1.5),
        "Intake manifold" => ([0.5, 0.3, 0.25], 4.0),
        "Exhaust manifold" => ([0.5, 0.3, 0.3], 6.0),
        "Air filter" => ([0.3, 0.3, 0.2], 1.0),
        "Throttle body" => ([0.15, 0.15, 0.15], 1.0),
        "Turbocharger" => ([0.35, 0.3, 0.3], 9.0),
        "Intercooler" => ([0.7, 0.2, 0.1], 5.0),
        "Catalyst" => ([0.5, 0.2, 0.2], 5.0),
        "Silencer" => ([0.6, 0.35, 0.2], 7.0),
        "Exhaust pipe" => ([1.5, 0.15, 0.15], 6.0),
        "Radiator" => ([0.7, 0.1, 0.45], 6.0),
        "Flywheel" => ([0.3, 0.3, 0.05], 8.0),
        "Cylinder head" => ([0.6, 0.3, 0.25], 18.0),
        _ => ([0.3, 0.3, 0.3], 3.0),
    }
}

fn defs(catalogue: &Catalogue) -> Vec<PartDef> {
    let mut defs = Vec::new();
    let mut add = |def: Def| defs.push(def.done());
    for (c, chassis) in catalogue.chassis.iter().enumerate() {
        let name = &chassis.name;
        let id = name.to_lowercase();
        let body = &chassis.body;
        let shell = [body.length, body.width, 1.4];
        add(Def::new(
            &id,
            name,
            Kind::Chassis,
            Does::Chassis(c),
            shell,
            chassis.mass,
        )
        .painted()
        .knobs(chassis_knobs())
        .slots(hatch_slots(catalogue)));
        let panels = [
            (
                "front-bumper",
                "Front bumper",
                Panel::FrontBumper,
                [1.7, 0.4, 0.4],
                6.0,
            ),
            (
                "front-lip",
                "Front lip",
                Panel::FrontLip,
                [1.5, 0.3, 0.08],
                2.0,
            ),
            ("bonnet", "Bonnet", Panel::Bonnet, [1.2, 1.4, 0.1], 12.0),
            ("fender", "Fender", Panel::Fender, [1.0, 0.3, 0.7], 4.0),
            ("door", "Door", Panel::Door, [1.3, 0.2, 1.1], 20.0),
            ("mirror", "Mirror", Panel::Mirror, [0.25, 0.15, 0.15], 1.0),
            (
                "side-skirt",
                "Side skirt",
                Panel::SideSkirt,
                [1.8, 0.15, 0.1],
                3.0,
            ),
            ("hatch", "Hatch", Panel::Hatch, [1.3, 1.0, 0.25], 15.0),
            (
                "rear-spoiler",
                "Roof spoiler",
                Panel::RearSpoiler,
                [1.1, 0.3, 0.1],
                2.0,
            ),
            (
                "rear-bumper",
                "Rear bumper",
                Panel::RearBumper,
                [1.7, 0.4, 0.4],
                6.0,
            ),
            (
                "diffuser",
                "Diffuser",
                Panel::Diffuser,
                [1.2, 0.4, 0.12],
                3.0,
            ),
        ];
        for (part, title, panel, size, mass) in panels {
            let def = Def::new(
                &format!("{id}.{part}"),
                title,
                Kind::Body(panel),
                Does::Nothing,
                size,
                mass,
            );
            add(def.fits(name).painted());
        }
        let brakes = [
            ("front-solid", "Solid front disc brake", 8.0),
            ("front-vented", "Vented front disc brake", 10.0),
            ("rear-drum", "Rear drum brake", 7.0),
            ("rear-disc", "Rear disc brake", 6.0),
        ];
        for (brake, title, mass) in brakes {
            let def = Def::new(
                &format!("{id}.brake.{brake}"),
                title,
                Kind::Brake,
                Does::Brake,
                [0.35, 0.35, 0.2],
                mass,
            );
            add(def.fits(name));
        }
        for (axle, end) in ["front", "rear"].into_iter().enumerate() {
            add(Def::new(
                &format!("{id}.anti-roll-bar.{end}"),
                &format!("{} anti-roll bar", if axle == 0 { "Front" } else { "Rear" }),
                Kind::AntiRollBar,
                Does::AntiRollBar(axle),
                [1.3, 0.3, 0.15],
                3.0,
            )
            .fits(name));
        }
        for (kit, suspension) in catalogue.suspensions.iter().enumerate() {
            let s = slug(&suspension.name);
            add(Def::new(
                &format!("{id}.spring.{s}"),
                &format!("{} spring", suspension.name),
                Kind::Spring,
                Does::Spring(kit),
                [0.3, 0.15, 0.15],
                2.0,
            )
            .fits(name));
            add(Def::new(
                &format!("{id}.damper.{s}"),
                &format!("{} damper", suspension.name),
                Kind::Damper,
                Does::Damper(kit),
                [0.6, 0.12, 0.12],
                3.0,
            )
            .fits(name)
            .knobs(damper_knobs()));
        }
    }
    add(Def::new(
        "seat.stock",
        "Stock seat",
        Kind::Seat(Seat::Front),
        Does::Nothing,
        [0.6, 0.55, 1.0],
        16.0,
    ));
    add(Def::new(
        "rear-bench.stock",
        "Stock rear bench",
        Kind::Seat(Seat::RearBench),
        Does::Nothing,
        [1.3, 0.6, 0.9],
        18.0,
    ));
    add(Def::new(
        "wheel",
        "Wheel",
        Kind::Wheel,
        Does::Wheel,
        [0.65, 0.65, 0.22],
        0.0,
    )
    .slots(vec![
        slot("Rim", Kind::Rim, true),
        slot("Tire", Kind::Tire, true),
    ]));
    add(Def::new(
        "rim.alloy",
        "Alloy rim",
        Kind::Rim,
        Does::Rim,
        [0.5, 0.5, 0.22],
        8.0,
    ));
    add(Def::new(
        "rim.steel",
        "Steel rim",
        Kind::Rim,
        Does::Rim,
        [0.5, 0.5, 0.22],
        10.0,
    ));
    for (c, compound) in catalogue.compounds.iter().enumerate() {
        add(Def::new(
            &format!("tire.{}", slug(&compound.name)),
            &format!("{} tire", compound.name),
            Kind::Tire,
            Does::Tire(c),
            [0.65, 0.65, 0.22],
            8.0,
        )
        .wears(Wear::Tread)
        .knobs(tire_knobs()));
    }
    for (g, gearbox) in catalogue.gearboxes.iter().enumerate() {
        add(Def::new(
            &format!("gearbox.{}", slug(&gearbox.name)),
            &gearbox.name,
            Kind::Gearbox,
            Does::Gearbox(g),
            [0.6, 0.45, 0.4],
            gearbox.mass,
        )
        .slots(vec![
            slot("Final drive", Kind::FinalDrive, true),
            slot("Differential", Kind::Differential, true),
        ]));
    }
    add(Def::new(
        "final-drive",
        "Final drive",
        Kind::FinalDrive,
        Does::FinalDrive,
        [0.25, 0.25, 0.1],
        3.0,
    )
    .knobs(vec![knob("Ratio", 2.0, 6.0, 4.4)]));
    for (limited_slip, id, title) in [
        (false, "differential.open", "Open differential"),
        (
            true,
            "differential.limited-slip",
            "Limited-slip differential",
        ),
    ] {
        add(Def::new(
            id,
            title,
            Kind::Differential,
            Does::Differential { limited_slip },
            [0.3, 0.3, 0.25],
            6.0,
        ));
    }
    for (k, clutch) in catalogue.clutches.iter().enumerate() {
        add(Def::new(
            &format!("clutch.{}", slug(&clutch.name)),
            &format!("{} clutch", clutch.name),
            Kind::Clutch,
            Does::Clutch(k),
            [0.3, 0.3, 0.08],
            5.0,
        ));
    }
    for (model, id, title) in [
        (EcuModel::Stock, "ecu.stock", "Stock ECU"),
        (
            EcuModel::Programmable,
            "ecu.programmable",
            "Programmable ECU",
        ),
    ] {
        add(Def::new(
            id,
            title,
            Kind::Ecu,
            Does::Ecu(model),
            [0.25, 0.2, 0.05],
            1.0,
        ));
    }
    let engine_slot = |s: usize| slot(&catalogue.slots[s].name, Kind::EnginePart(s), true);
    let mut engine_slots = vec![slot("Long block", Kind::LongBlock, true)];
    engine_slots.extend(ENGINE_SLOTS.map(engine_slot));
    add(Def::new(
        "engine",
        "Engine",
        Kind::Engine,
        Does::Engine,
        [0.7, 0.7, 0.7],
        0.0,
    )
    .knobs(engine_knobs())
    .slots(engine_slots));
    for (f, family) in catalogue.families.iter().enumerate() {
        for (v, version) in family.versions.iter().enumerate() {
            let size = [family.length, family.width, family.height];
            add(Def::new(
                &version.code.to_lowercase(),
                &format!("{} {} long block", version.code, version.name),
                Kind::LongBlock,
                Does::Block {
                    family: f,
                    version: v,
                },
                size,
                0.7 * family.mass,
            )
            .bored()
            .slots(LONG_BLOCK_SLOTS.map(engine_slot).to_vec()));
        }
    }
    for (s, kind) in catalogue.slots.iter().enumerate() {
        let (size, mass) = engine_part_size(&kind.name);
        for (p, part) in kind.parts.iter().enumerate() {
            let mut def = Def::new(
                &format!("{}.{}", slug(&kind.name), slug(&part.name)),
                &format!("{} ({})", part.name, kind.name.to_lowercase()),
                Kind::EnginePart(s),
                Does::EnginePart { slot: s, part: p },
                size,
                mass,
            );
            // "None": no turbo, no intercooler, no catalyst.
            def.absent = part.name == "None";
            // What wears in the engine sits on the part it's in.
            def.wear = match kind.name.as_str() {
                _ if def.absent => None,
                "Pistons" => Some(Wear::Pistons),
                "Connecting rods" => Some(Wear::Rods),
                "Cylinder head" => Some(Wear::Valves),
                "Head gasket" => Some(Wear::HeadGasket),
                "Crankshaft" => Some(Wear::Bearings),
                "Turbocharger" => Some(Wear::Turbo),
                _ => None,
            };
            // Pistons come in the bore sizes.
            def.bores = kind.name == "Pistons";
            add(def);
        }
    }
    defs
}

/// What sets a stock car apart: its factory engine, wheels (the same all round), rims,
/// brakes (front and rear, by the part ids after the chassis' "<id>.brake."),
/// differential and whether it has a rear spoiler.
struct TrimSpec {
    engine: String,
    name: String,
    wheel: Wheel,
    rim: &'static str,
    brakes: [&'static str; 2],
    limited_slip: bool,
    spoiler: bool,
}

/// The trims of `chassis` (none: one per factory engine, as the chassis leaves the
/// factory).
fn trim_specs(chassis: &str) -> Vec<TrimSpec> {
    match chassis {
        "Vireo" => vec![
            TrimSpec {
                engine: "B16".into(),
                name: "1.6".into(),
                wheel: super::chassis::wheel(14, 55, 185, 60, STREET),
                rim: "rim.steel",
                brakes: ["front-solid", "rear-drum"],
                limited_slip: false,
                spoiler: false,
            },
            TrimSpec {
                engine: "B18".into(),
                name: "1.8".into(),
                wheel: super::chassis::wheel(15, 60, 195, 55, STREET),
                rim: "rim.alloy",
                brakes: ["front-vented", "rear-disc"],
                limited_slip: true,
                spoiler: true,
            },
        ],
        _ => Vec::new(),
    }
}

/// The stock cars of every chassis (see `trim_specs`).
fn trims(catalogue: &Shared) -> Vec<Trim> {
    let def = |id: &str| {
        (catalogue.defs.iter())
            .position(|def| def.id == id)
            .unwrap_or_else(|| panic!("no part {id}"))
    };
    let mut trims = Vec::new();
    for (c, chassis) in catalogue.chassis.iter().enumerate() {
        let id = chassis.name.to_lowercase();
        let mut specs = trim_specs(&chassis.name);
        if specs.is_empty() {
            let factory = CarBuild::factory_in(catalogue, c);
            specs = (chassis.engines.iter())
                .map(|code| TrimSpec {
                    engine: code.clone(),
                    name: code.clone(),
                    wheel: factory.wheels[0],
                    rim: "rim.alloy",
                    brakes: ["front-vented", "rear-disc"],
                    limited_slip: chassis.limited_slip,
                    spoiler: false,
                })
                .collect();
        }
        for spec in specs {
            let mut car = Item::new(catalogue, def(&id), chassis.paint);
            let made = |part: &str| Item::new(catalogue, def(part), chassis.paint);
            let slots = catalogue.defs[car.def].slots.clone();
            for (slot, item) in slots.iter().zip(&mut car.slots) {
                let part = match slot.kind {
                    Kind::Body(Panel::FrontBumper) => format!("{id}.front-bumper"),
                    Kind::Body(Panel::Bonnet) => format!("{id}.bonnet"),
                    Kind::Body(Panel::Fender) => format!("{id}.fender"),
                    Kind::Body(Panel::Door) => format!("{id}.door"),
                    Kind::Body(Panel::Mirror) => format!("{id}.mirror"),
                    Kind::Body(Panel::Hatch) => format!("{id}.hatch"),
                    Kind::Body(Panel::RearSpoiler) if spec.spoiler => {
                        format!("{id}.rear-spoiler")
                    }
                    Kind::Body(Panel::RearBumper) => format!("{id}.rear-bumper"),
                    Kind::Seat(Seat::Front) => "seat.stock".into(),
                    Kind::Seat(Seat::RearBench) => "rear-bench.stock".into(),
                    Kind::Brake => {
                        let front = slot.name.contains("front");
                        format!("{id}.brake.{}", spec.brakes[usize::from(!front)])
                    }
                    Kind::Wheel => {
                        // The rim of the trim's kind; `assemble` sizes it.
                        let mut wheel = made("wheel");
                        wheel.slots[0] = Some(made(spec.rim));
                        *item = Some(wheel);
                        continue;
                    }
                    _ => continue,
                };
                *item = Some(made(&part));
            }
            let mut build = CarBuild::factory_in(catalogue, c);
            build.wheels = [spec.wheel; 2];
            build.limited_slip = spec.limited_slip;
            let option = (chassis.engines.iter())
                .position(|code| *code == spec.engine)
                .expect("a factory engine");
            let parts = build.factory_engine(option);
            let car = assemble(catalogue, Some(&car), &build, &parts).expect("a stock car");
            trims.push(Trim {
                id: format!("{id}.{}", slug(&spec.name)),
                name: format!("{} {}", chassis.name, spec.name),
                car,
            });
        }
    }
    trims
}

/// The factory engines as the trims have them, each sold whole (`engine.<code>`).
fn engines(catalogue: &Catalogue) -> Vec<Assembly> {
    let mut engines: Vec<Assembly> = Vec::new();
    for trim in &catalogue.trims {
        let Some(engine) = trim.car.fitted(catalogue, Kind::Engine).next() else {
            continue;
        };
        let block = engine.fitted(catalogue, Kind::LongBlock).next();
        let Some(Does::Block { family, version }) = block.map(|b| &b.def(catalogue).does) else {
            continue;
        };
        let version = &catalogue.families[*family].versions[*version];
        let id = format!("engine.{}", version.code.to_lowercase());
        if engines.iter().all(|assembly| assembly.id != id) {
            engines.push(Assembly {
                id,
                name: format!("{} {} engine", version.code, version.name),
                item: engine.clone(),
            });
        }
    }
    engines
}
