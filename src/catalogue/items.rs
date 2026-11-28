//! The built-in catalogue's part definitions and trims: everything the hot hatch is
//! built of, and what can go onto it.

use std::sync::Arc;

use super::{Catalogue, Shared};
use crate::chassis::{CarBuild, GearboxKind};
use crate::items::{
    Aero, Assembly, BONNET_CUT_OUT, Does, Item, Kind, Knob, OVERHEAD_CAMS, Panel, PartDef, Seat,
    SlotDef, Trim, Wear, assemble, chassis_knobs, damper_knobs, engine_knobs, knob, spring_knobs,
    tire_knobs, wing_knobs,
};
use crate::physics::brakes::Brake;
use crate::physics::engine::hardware::CamDrive;
use crate::physics::tire::STREET;
use crate::wheels::Wheel;

/// Where the catalogue's engine slots are: in the long block (its cylinder head a slot of
/// its own, last), in the head (porting, cams, springs, phasers), on the engine (the
/// exhaust manifold, a slot of its own, after the first), or in the car (by their places
/// in `slots`). The turbos' slot is none of them: turbos go on the manifold's mounts.
pub const LONG_BLOCK_SLOTS: [usize; 5] = [0, 1, 2, 3, 4];
pub const HEAD_SLOTS: [usize; 4] = [19, 5, 6, 7];
pub const ENGINE_SLOTS: [usize; 6] = [8, 9, 10, 12, 18, 20];
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
    engines: Vec<String>,
    size: [f32; 3],
    mass: f32,
    painted: bool,
    absent: bool,
    wear: Option<Wear>,
    bores: bool,
    needs: Vec<String>,
    provides: Vec<String>,
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
            engines: Vec::new(),
            size,
            mass,
            painted: false,
            absent: false,
            wear: None,
            bores: false,
            needs: Vec::new(),
            provides: Vec::new(),
            knobs: Vec::new(),
            slots: Vec::new(),
        }
    }

    fn provides(mut self, what: &str) -> Self {
        self.provides.push(what.into());
        self
    }

    fn fits(mut self, chassis: &str) -> Self {
        self.fits.push(chassis.into());
        self
    }

    fn engines(mut self, engine: &str) -> Self {
        self.engines.push(engine.into());
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
            engines: self.engines,
            size: self.size,
            mass: self.mass,
            price: 0,
            painted: self.painted,
            absent: self.absent,
            wear: self.wear,
            bores: self.bores,
            needs: self.needs,
            provides: self.provides,
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

/// What a chassis' body is called where it differs: the lid at the back and the spoiler
/// on it, whether there's a rear bench behind the front seats, and whether it has rear
/// doors (a four-door).
struct Layout {
    lid: &'static str,
    spoiler: &'static str,
    rear_bench: bool,
    rear_doors: bool,
}

fn layout(chassis: &str) -> Layout {
    let hatch = Layout {
        lid: "Hatch",
        spoiler: "Roof spoiler",
        rear_bench: true,
        rear_doors: false,
    };
    match chassis {
        // An open two-seater.
        "Merrow" => Layout {
            lid: "Boot lid",
            spoiler: "Boot spoiler",
            rear_bench: false,
            ..hatch
        },
        // A saloon: a boot lid behind the cabin, and rear doors.
        "Aldine" => Layout {
            lid: "Boot lid",
            spoiler: "Boot spoiler",
            rear_doors: true,
            ..hatch
        },
        // A tall box: four doors and a split tailgate.
        "Bothy" => Layout {
            lid: "Tailgate",
            rear_doors: true,
            ..hatch
        },
        _ => hatch,
    }
}

/// A chassis' slots: body, seats, a wheel, brake, spring and damper at each corner,
/// anti-roll bars, the drivetrain, the exhaust after the manifold, cooling, the ECU and the
/// engine.
fn chassis_slots(catalogue: &Catalogue, chassis: &str) -> Vec<SlotDef> {
    let layout = layout(chassis);
    let body = |name: &str, panel| slot(name, Kind::Body(panel), false);
    let mut slots = vec![
        body("Front bumper", Panel::FrontBumper),
        body("Front lip", Panel::FrontLip),
        body("Bonnet", Panel::Bonnet),
        body("Left fender", Panel::Fender),
        body("Right fender", Panel::Fender),
        body("Left door", Panel::Door),
        body("Right door", Panel::Door),
        body("Left mirror", Panel::Mirror),
        body("Right mirror", Panel::Mirror),
    ];
    if layout.rear_doors {
        slots.extend([
            body("Left rear door", Panel::RearDoor),
            body("Right rear door", Panel::RearDoor),
        ]);
    }
    slots.extend([
        body("Left side skirt", Panel::SideSkirt),
        body("Right side skirt", Panel::SideSkirt),
        body(layout.lid, Panel::Hatch),
        body("Rear spoiler", Panel::RearSpoiler),
        body("Rear bumper", Panel::RearBumper),
        body("Diffuser", Panel::Diffuser),
        slot("Driver's seat", Kind::Seat(Seat::Front), true),
        slot("Passenger seat", Kind::Seat(Seat::Front), false),
    ]);
    // A lip goes on the front bumper, mirrors on the (front) doors, the spoiler on the
    // hatch, the diffuser under the rear bumper.
    let place = |slots: &[SlotDef], name: &str| {
        (slots.iter())
            .position(|slot| slot.name == name)
            .expect("a body slot")
    };
    for (name, on) in [
        ("Front lip", "Front bumper"),
        ("Left mirror", "Left door"),
        ("Right mirror", "Right door"),
        ("Rear spoiler", layout.lid),
        ("Diffuser", "Rear bumper"),
    ] {
        let (at, needed) = (place(&slots, name), place(&slots, on));
        slots[at] = needing(slots[at].clone(), needed);
    }
    if layout.rear_bench {
        slots.push(slot("Rear bench", Kind::Seat(Seat::RearBench), false));
    }
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
        "Air filter" => ([0.3, 0.3, 0.2], 1.0),
        "Throttle body" => ([0.15, 0.15, 0.15], 1.0),
        super::TURBO_SLOT | super::SUPERCHARGER_SLOT => ([0.35, 0.3, 0.3], 9.0),
        "Intercooler" => ([0.7, 0.2, 0.1], 5.0),
        "Catalyst" => ([0.5, 0.2, 0.2], 5.0),
        "Silencer" => ([0.6, 0.35, 0.2], 7.0),
        "Exhaust pipe" => ([1.5, 0.15, 0.15], 6.0),
        "Radiator" => ([0.7, 0.1, 0.45], 6.0),
        "Flywheel" => ([0.3, 0.3, 0.05], 8.0),
        "Porting" => ([0.3, 0.2, 0.1], 0.5),
        "Lambda sensor" => ([0.15, 0.05, 0.05], 0.1),
        _ => ([0.3, 0.3, 0.3], 3.0),
    }
}

/// What the hatch's body part `part` (its id after the chassis') does to the air: the
/// stock pieces nothing (the chassis' drag is with them), a roof spoiler a little
/// downforce at the back; a lip, skirts and a diffuser more, the nearer the ground they
/// run.
fn body_aero(part: &str) -> Aero {
    let aero = |drag, downforce, ground| Aero {
        drag,
        downforce,
        ground,
        wing: false,
    };
    match part {
        "front-lip" => aero(0.01, [0.1, 0.0], 0.06),
        "side-skirt" => aero(0.0, [0.01, 0.01], 0.04),
        "rear-spoiler" => aero(0.006, [0.0, 0.03], 0.0),
        "diffuser" => aero(-0.006, [0.0, 0.08], 0.05),
        _ => Aero::default(),
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
        .slots(chassis_slots(catalogue, name)));
        let layout = layout(name);
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
            (
                "rear-door",
                "Rear door",
                Panel::RearDoor,
                [1.0, 0.2, 1.1],
                16.0,
            ),
            ("mirror", "Mirror", Panel::Mirror, [0.25, 0.15, 0.15], 1.0),
            (
                "side-skirt",
                "Side skirt",
                Panel::SideSkirt,
                [1.8, 0.15, 0.1],
                3.0,
            ),
            ("hatch", layout.lid, Panel::Hatch, [1.3, 1.0, 0.25], 15.0),
            (
                "rear-spoiler",
                layout.spoiler,
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
            if panel == Panel::RearDoor && !layout.rear_doors {
                continue;
            }
            let def = Def::new(
                &format!("{id}.{part}"),
                title,
                Kind::Body(panel),
                Does::Body(body_aero(part)),
                size,
                mass,
            );
            add(def.fits(name).painted());
        }
        // A bonnet with a cut-out for a Roots blower to stick through.
        add(Def::new(
            &format!("{id}.bonnet.cut-out"),
            "Bonnet with a blower cut-out",
            Kind::Body(Panel::Bonnet),
            Does::Body(Aero {
                drag: 0.01,
                ..Aero::default()
            }),
            [1.2, 1.4, 0.12],
            11.0,
        )
        .fits(name)
        .painted()
        .provides(BONNET_CUT_OUT));
        // A tall rear wing on the hatch, its angle set at alignment shops.
        add(Def::new(
            &format!("{id}.wing"),
            "Rear wing",
            Kind::Body(Panel::RearSpoiler),
            Does::Body(Aero {
                drag: 0.05,
                downforce: [0.0, 0.32],
                ground: 0.0,
                wing: true,
            }),
            [1.4, 0.45, 0.35],
            6.0,
        )
        .fits(name)
        .painted()
        .knobs(wing_knobs()));
        // Torque (N·m), smallest rim, heat capacity (J/K), cooling standing and per m/s
        // (W/K), fade temperature (K), the pads' life (MJ).
        let brake = |torque, min_rim, heat, cooling, fade, life| Brake {
            torque,
            min_rim,
            heat,
            cooling,
            fade,
            life,
        };
        let hatch_brakes = [
            (
                "front-solid",
                "Solid front disc brake",
                8.0,
                brake(950.0, 13, 2300.0, [6.0, 1.2], 620.0, 300.0),
            ),
            (
                "front-vented",
                "Vented front disc brake",
                10.0,
                brake(1100.0, 14, 3000.0, [10.0, 2.4], 660.0, 400.0),
            ),
            (
                "front-big",
                "Big front brake kit, four-piston",
                12.0,
                brake(1400.0, 15, 4200.0, [14.0, 3.2], 720.0, 600.0),
            ),
            (
                "rear-drum",
                "Rear drum brake",
                7.0,
                brake(190.0, 13, 2600.0, [5.0, 0.6], 560.0, 250.0),
            ),
            (
                "rear-disc",
                "Rear disc brake",
                6.0,
                brake(230.0, 13, 1800.0, [6.0, 1.2], 640.0, 200.0),
            ),
        ];
        // A heavy, fast car's: big vented discs all round, behind 17" wheels.
        let roadster_brakes = [
            (
                "front-vented",
                "Vented front disc brake, four-piston",
                14.0,
                brake(1500.0, 17, 5600.0, [16.0, 3.6], 720.0, 800.0),
            ),
            (
                "rear-vented",
                "Vented rear disc brake",
                10.0,
                brake(720.0, 17, 3600.0, [10.0, 2.4], 680.0, 500.0),
            ),
        ];
        let brakes: Vec<_> = match name.as_str() {
            "Merrow" => roadster_brakes.to_vec(),
            "Vireo" => hatch_brakes.to_vec(),
            // The later, heavier cars': the hatch's kinds made for their weight and
            // wheels (see `scaled_brakes`).
            other => {
                let (scale, rim) = match other {
                    "Tessel" => (1.28, 15),
                    "Aldine" => (1.55, 15),
                    _ => (1.9, 16),
                };
                scaled_brakes(&hatch_brakes, scale, rim)
            }
        };
        for (part, title, mass, brake) in brakes {
            let def = Def::new(
                &format!("{id}.brake.{part}"),
                title,
                Kind::Brake,
                Does::Brake(brake),
                [0.35, 0.35, 0.2],
                mass,
            );
            add(def.fits(name).wears(Wear::Pads));
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
            // Stock springs sit where they sit; the sport kit's perches lower the body,
            // the track kit's coilovers more.
            let knobs = match kit {
                0 => Vec::new(),
                1 => spring_knobs(-40.0, 0.0, -25.0),
                _ => spring_knobs(-70.0, 10.0, -40.0),
            };
            add(Def::new(
                &format!("{id}.spring.{s}"),
                &format!("{} spring", suspension.name),
                Kind::Spring,
                Does::Spring(kit),
                [0.3, 0.15, 0.15],
                2.0,
            )
            .fits(name)
            .knobs(knobs));
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
        "seat.bucket",
        "Bucket seat",
        Kind::Seat(Seat::Front),
        Does::Nothing,
        [0.6, 0.5, 1.0],
        6.0,
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
        // Made for the chassis whose layout takes it (transverse, inline…).
        let mut def = Def::new(
            &format!("gearbox.{}", slug(&gearbox.name)),
            &gearbox.name,
            Kind::Gearbox,
            Does::Gearbox(g),
            [0.6, 0.45, 0.4],
            gearbox.mass,
        )
        .wears(Wear::Gears);
        let mut slots = vec![
            slot("Final drive", Kind::FinalDrive, true),
            slot("Differential", Kind::Differential, true),
        ];
        // An adapter plate on its face, if it may have one (`Catalogue::bolting`).
        if gearbox.kind != GearboxKind::Transverse {
            slots.push(slot("Adapter plate", Kind::Adapter, false));
        }
        def = def.slots(slots);
        for chassis in &catalogue.chassis {
            if catalogue.gearboxes[chassis.gearbox].kind == gearbox.kind {
                def = def.fits(&chassis.name);
            }
        }
        add(def);
    }
    for (a, adapter) in catalogue.adapters.iter().enumerate() {
        add(Def::new(
            &adapter.id,
            &adapter.name,
            Kind::Adapter,
            Does::Adapter(a),
            [0.45, 0.4, 0.05],
            adapter.mass,
        ));
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
        )
        .wears(Wear::Clutch));
    }
    for (e, ecu) in catalogue.ecus.iter().enumerate() {
        add(Def::new(
            &ecu.id,
            &ecu.name,
            Kind::Ecu,
            Does::Ecu(e),
            [0.25, 0.2, 0.05],
            1.0,
        ));
    }
    let engine_slot = |s: usize| slot(&catalogue.slots[s].name, Kind::EnginePart(s), true);
    let mut engine_slots = vec![slot("Long block", Kind::LongBlock, true)];
    engine_slots.extend(ENGINE_SLOTS.map(engine_slot));
    engine_slots.insert(2, slot("Exhaust manifold", Kind::Manifold, true));
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
            let mut def = Def::new(
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
            .slots(LONG_BLOCK_SLOTS.map(engine_slot).to_vec());
            def.slots.push(slot("Cylinder head", Kind::Head, true));
            add(def);
        }
        for (h, head) in family.heads.iter().enumerate() {
            let mut def = Def::new(
                &head.id,
                &head.name,
                Kind::Head,
                Does::Head { family: f, head: h },
                [family.length.max(0.5), 0.3, 0.25],
                0.18 * family.mass,
            )
            .engines(&family.name)
            .wears(Wear::Valves)
            .slots(HEAD_SLOTS.map(engine_slot).to_vec());
            if head.valvetrain.drive != CamDrive::Ohv {
                def = def.provides(OVERHEAD_CAMS);
            }
            add(def);
        }
    }
    for (f, family) in catalogue.families.iter().enumerate() {
        let cylinders = family.versions.first().map_or(4, |v| v.block.cylinders) as f32;
        for (m, made) in family.manifolds.iter().enumerate() {
            // A turbo mount on each branch: a V's on each bank, an inline engine's in turn.
            let branches = made.mounts();
            let mounts = (0..branches).map(|branch| {
                let name = match (branches, family.layout.is_vee(), branch) {
                    (1, ..) => "Turbo".to_string(),
                    (2, true, 0) => "Turbo, left bank".to_string(),
                    (2, true, _) => "Turbo, right bank".to_string(),
                    (2, false, 0) => "Turbo, front".to_string(),
                    (2, false, _) => "Turbo, rear".to_string(),
                    _ => format!("Turbo {}", branch + 1),
                };
                slot(&name, Kind::Turbo, false)
            });
            // Cast iron, heavier with a turbine flange; a pair as long as the engine.
            let mass = 1.5 * cylinders * if made.mounts { 1.3 } else { 1.0 };
            let size = [family.length.max(0.5), 0.35, 0.3];
            add(Def::new(
                &made.id,
                &made.name,
                Kind::Manifold,
                Does::Manifold {
                    family: f,
                    manifold: m,
                },
                size,
                mass,
            )
            .engines(&family.name)
            .slots(mounts.collect()));
        }
    }
    for (s, kind) in catalogue.slots.iter().enumerate() {
        let (size, mass) = engine_part_size(&kind.name);
        for (p, part) in kind.parts.iter().enumerate() {
            // Nothing on a mount is an empty mount.
            if kind.mounted && p == 0 {
                continue;
            }
            // The turbos and superchargers keep their parts' old ids (saved cars have
            // them), from when they shared a slot.
            let prefix = match kind.name.as_str() {
                super::TURBO_SLOT | super::SUPERCHARGER_SLOT => "turbocharger".to_string(),
                name => slug(name),
            };
            let slot_kind = if kind.mounted {
                Kind::Turbo
            } else {
                Kind::EnginePart(s)
            };
            let mut def = Def::new(
                &format!("{prefix}.{}", slug(&part.name)),
                &format!("{} ({})", part.name, kind.name.to_lowercase()),
                slot_kind,
                Does::EnginePart { slot: s, part: p },
                size,
                mass,
            );
            // "None": no supercharger, no intercooler, no catalyst.
            def.absent = part.name == "None";
            // What wears in the engine sits on the part it's in.
            def.wear = match kind.name.as_str() {
                _ if def.absent => None,
                "Pistons" => Some(Wear::Pistons),
                "Connecting rods" => Some(Wear::Rods),
                "Head gasket" => Some(Wear::HeadGasket),
                "Crankshaft" => Some(Wear::Bearings),
                _ if kind.mounted => Some(Wear::Turbo),
                _ => None,
            };
            // Pistons come in the bore sizes.
            def.bores = kind.name == "Pistons";
            // A Roots blower sticks up through the bonnet.
            if part.name == super::ROOTS {
                def.needs.push(BONNET_CUT_OUT.into());
            }
            // Phasers and switching cams turn or rock cams in the head.
            if moves_overhead_cams(part) {
                def.needs.push(OVERHEAD_CAMS.into());
            }
            add(def);
        }
    }
    defs
}

/// A brake part: its id after the chassis' "<id>.brake.", its name, mass (kg) and brake.
type BrakeDef = (&'static str, &'static str, f32, Brake);

/// The hatch's brakes (`hatch`) made for a heavier car on bigger wheels: torque, heat
/// capacity, cooling and the pads' life `scale` times theirs, behind rims of `rim` inches
/// and more (the big kit and the large vented disc an inch more, the rear ones an inch
/// less); no solid disc, and a
/// large vented front disc (a hot trim's) between the vented one and the big kit.
fn scaled_brakes(hatch: &[BrakeDef], scale: f32, rim: u8) -> Vec<BrakeDef> {
    let mut brakes = Vec::new();
    for &(part, title, mass, brake) in hatch {
        if part == "front-solid" {
            continue;
        }
        let bigger = part == "front-big";
        let made = |brake: Brake, bigger: bool| Brake {
            torque: (brake.torque * scale / 10.0).round() * 10.0,
            min_rim: match (bigger, part.starts_with("rear")) {
                (true, _) => rim + 1,
                (_, true) => rim - 1,
                _ => rim,
            },
            heat: (brake.heat * scale / 100.0).round() * 100.0,
            cooling: brake.cooling.map(|c| (c * scale * 10.0).round() / 10.0),
            life: (brake.life * scale / 10.0).round() * 10.0,
            ..brake
        };
        let mass = (mass * scale.sqrt()).round();
        if part == "front-big" {
            // Between the vented disc and the big kit.
            let vented = hatch
                .iter()
                .find(|b| b.0 == "front-vented")
                .expect("vented");
            let large = Brake {
                torque: 0.5 * (vented.3.torque + brake.torque),
                heat: 0.5 * (vented.3.heat + brake.heat),
                fade: 0.5 * (vented.3.fade + brake.fade),
                life: 0.5 * (vented.3.life + brake.life),
                ..brake
            };
            brakes.push((
                "front-vented-large",
                "Large vented front disc brake",
                mass - 1.0,
                made(large, true),
            ));
        }
        brakes.push((part, title, mass, made(brake, bigger)));
    }
    brakes
}

/// Whether `part` works on cams in the head: phasers or a switching cam.
fn moves_overhead_cams(part: &crate::physics::engine::parts::Part) -> bool {
    use crate::physics::engine::parts::{Effect, Field};
    (part.effects.iter()).any(|effect| match effect {
        Effect::Switching { .. } => true,
        Effect::Change { field, .. } => {
            matches!(field, Field::IntakePhasing | Field::ExhaustPhasing)
        }
        _ => false,
    })
}

/// What sets a stock car apart: its factory engine, wheels (front and rear), rims,
/// brakes (front and rear, by the part ids after the chassis' "<id>.brake."),
/// differential, whether it has a rear spoiler, and its gearbox (by name) and final drive
/// where they aren't the chassis'.
struct TrimSpec {
    engine: String,
    name: String,
    wheels: [Wheel; 2],
    rim: &'static str,
    brakes: [&'static str; 2],
    limited_slip: bool,
    spoiler: bool,
    gearbox: Option<&'static str>,
    final_drive: Option<f32>,
}

impl TrimSpec {
    /// A trim of `engine` called `name` on `wheels` (both axles) of `rim`, with `brakes`,
    /// an open differential, no spoiler, and the chassis' gearbox and final drive.
    fn new(
        engine: &str,
        name: &str,
        wheel: Wheel,
        rim: &'static str,
        brakes: [&'static str; 2],
    ) -> Self {
        Self {
            engine: engine.into(),
            name: name.into(),
            wheels: [wheel; 2],
            rim,
            brakes,
            limited_slip: false,
            spoiler: false,
            gearbox: None,
            final_drive: None,
        }
    }
}

/// The trims of `chassis` (none: one per factory engine, as the chassis leaves the
/// factory).
fn trim_specs(chassis: &str) -> Vec<TrimSpec> {
    match chassis {
        "Vireo" => vec![
            TrimSpec {
                engine: "B16".into(),
                name: "1.6".into(),
                wheels: [super::chassis::wheel(14, 55, 185, 60, STREET); 2],
                rim: "rim.steel",
                brakes: ["front-solid", "rear-drum"],
                limited_slip: false,
                spoiler: false,
                gearbox: None,
                final_drive: None,
            },
            TrimSpec {
                engine: "B20".into(),
                name: "2.0".into(),
                wheels: [super::chassis::wheel(15, 60, 195, 55, STREET); 2],
                rim: "rim.alloy",
                brakes: ["front-vented", "rear-disc"],
                limited_slip: true,
                spoiler: true,
                gearbox: None,
                final_drive: None,
            },
            TrimSpec {
                engine: "B16R".into(),
                name: "1.6R".into(),
                wheels: [super::chassis::wheel(15, 60, 195, 55, STREET); 2],
                rim: "rim.alloy",
                brakes: ["front-vented", "rear-disc"],
                limited_slip: true,
                spoiler: true,
                gearbox: None,
                final_drive: None,
            },
        ],
        "Merrow" => vec![TrimSpec {
            engine: "R80".into(),
            name: "R80".into(),
            wheels: [
                super::chassis::wheel(17, 95, 275, 40, STREET),
                super::chassis::wheel(17, 115, 335, 35, STREET),
            ],
            rim: "rim.alloy",
            brakes: ["front-vented", "rear-vented"],
            limited_slip: true,
            spoiler: false,
            gearbox: None,
            final_drive: None,
        }],
        "Tessel" => {
            use super::chassis::wheel;
            vec![
                TrimSpec::new(
                    "K20",
                    "2.0",
                    wheel(15, 65, 195, 60, STREET),
                    "rim.alloy",
                    ["front-vented", "rear-drum"],
                ),
                // The hot one: the close six-speed, a shorter final drive, a limited slip,
                // 17s, the large front discs, rear discs and the roof spoiler.
                TrimSpec {
                    limited_slip: true,
                    spoiler: true,
                    gearbox: Some("K 6-speed close-ratio (transverse)"),
                    final_drive: Some(4.76),
                    ..TrimSpec::new(
                        "K20R",
                        "2.0R",
                        wheel(17, 70, 205, 45, STREET),
                        "rim.alloy",
                        ["front-vented-large", "rear-disc"],
                    )
                },
            ]
        }
        "Aldine" => {
            use super::chassis::wheel;
            vec![
                TrimSpec::new(
                    "K20",
                    "2.0",
                    wheel(16, 65, 205, 60, STREET),
                    "rim.alloy",
                    ["front-vented", "rear-disc"],
                ),
                // The 2.4 on 17s.
                TrimSpec::new(
                    "K24",
                    "2.4",
                    wheel(17, 70, 215, 50, STREET),
                    "rim.alloy",
                    ["front-vented", "rear-disc"],
                ),
            ]
        }
        "Bothy" => vec![TrimSpec::new(
            "K24",
            "2.4",
            super::chassis::wheel(16, 65, 215, 70, STREET),
            "rim.steel",
            ["front-vented", "rear-drum"],
        )],
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
                    wheels: factory.wheels,
                    rim: "rim.alloy",
                    brakes: ["front-vented", "rear-disc"],
                    limited_slip: chassis.limited_slip,
                    spoiler: false,
                    gearbox: None,
                    final_drive: None,
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
                    Kind::Body(Panel::RearDoor) => format!("{id}.rear-door"),
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
            build.wheels = spec.wheels;
            build.limited_slip = spec.limited_slip;
            if let Some(name) = spec.gearbox {
                build.gearbox = (catalogue.gearboxes.iter())
                    .position(|gearbox| gearbox.name == name)
                    .unwrap_or_else(|| panic!("no gearbox {name}"));
            }
            build.final_drive = spec.final_drive.unwrap_or(build.final_drive);
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
