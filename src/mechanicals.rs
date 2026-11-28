//! What's under the body, drawn from the parts a car is built of: the engine bay (the
//! engine's long block, intake and throttle, air filter, exhaust manifold and turbo, the
//! gearbox, radiator, intercooler and battery), the cabin's seats and dashboard, and
//! underneath the fuel tank and the exhaust line part by part (front pipe, catalyst,
//! silencer, tailpipe). One flat-shaded mesh with vertex colours in the body's design
//! frame (see `body`), drawn with it; seen where a body part is missing, through the
//! glass, or from below.

use bevy::prelude::*;

use crate::body::{Body, Builder, CARPET, DARK, METAL, TRIM, Wheels};
use crate::catalogue::Catalogue;
use crate::chassis::{Chassis, Orientation};
use crate::items::{BONNET_CUT_OUT, Does, Item, Kind, Panel, Seat};
use crate::physics::drivetrain::Drive;
use crate::physics::engine::hardware::CamDrive;

const CAST: [f32; 3] = [0.42, 0.43, 0.45];
const ALLOY: [f32; 3] = [0.68, 0.69, 0.7];
const COVER: [f32; 3] = [0.08, 0.08, 0.09];
const RED: [f32; 3] = [0.72, 0.08, 0.06];
const HOT: [f32; 3] = [0.4, 0.31, 0.26];
/// Cast iron manifolds and turbine housings.
const IRON: [f32; 3] = [0.24, 0.21, 0.19];
const BLACK: [f32; 3] = [0.03, 0.03, 0.03];
const CORE: [f32; 3] = [0.14, 0.14, 0.15];
const CLOTH: [f32; 3] = [0.16, 0.16, 0.18];
/// How far the exhaust hangs under the floor (its pipe's centre).
const HANG: f32 = 0.03;

/// The name of the part in the catalogue's engine slot called `slot`, wherever it sits
/// in `car`'s tree.
fn part<'a>(catalogue: &'a Catalogue, car: &Item, slot: &str) -> Option<&'a str> {
    car.all()
        .into_iter()
        .find_map(|item| match item.def(catalogue).does {
            Does::EnginePart { slot: s, part } if catalogue.slots[s].name == slot => {
                Some(catalogue.slots[s].parts[part].name.as_str())
            }
            _ => None,
        })
}

/// A key for caching meshes of `car`'s tree: what's fitted where and in what paint.
pub fn key(car: &Item) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fn walk(item: &Item, hasher: &mut impl Hasher) {
        (item.def, item.paint, item.variant).hash(hasher);
        for slot in &item.slots {
            match slot {
                Some(child) => walk(child, hasher),
                None => usize::MAX.hash(hasher),
            }
        }
    }
    walk(car, &mut hasher);
    hasher.finish()
}

/// How the exhaust manifold fitted looks: cast (a log with stubs) or bent of tubes, its
/// primaries' length against the family's own manifold's, how many branches it gathers
/// the cylinders into, and the turbos on its mounts (their turbine snails' radius, m).
struct Headers {
    cast: bool,
    share: f32,
    branches: u32,
    turbos: Vec<f32>,
}

impl Headers {
    fn of(
        catalogue: &Catalogue,
        manifold: Option<&Item>,
        family: &crate::physics::engine::families::Family,
        version: usize,
    ) -> Self {
        let made = manifold.and_then(|item| match item.def(catalogue).does {
            Does::Manifold { manifold, .. } => family.manifolds.get(manifold),
            _ => None,
        });
        let stock = family.manifolds.first();
        let Some(made) = made.or(stock) else {
            return Self {
                cast: true,
                share: 1.0,
                branches: if family.layout.is_vee() { 2 } else { 1 },
                turbos: Vec::new(),
            };
        };
        let length = |m: &crate::physics::engine::families::ManifoldDef| m.manifold.primary_length;
        let share = length(made) / stock.map_or(length(made), length);
        // A turbo's snail grows with the displacement behind its mount (a "Medium" on a
        // V10's bank is sized for 4 litres) and with its size.
        let displacement = family.versions.get(version).map_or(1.6e-3, |v| {
            v.block.displacement() / made.manifold.branches.max(1) as f32
        });
        let grown = (displacement / 1.6e-3).cbrt().clamp(0.8, 1.6);
        let turbos = manifold
            .map(|item| {
                (item.fitted(catalogue, Kind::Turbo))
                    .map(|turbo| {
                        let base = match turbo.def(catalogue).does {
                            Does::EnginePart { part: 1, .. } => 0.062,
                            Does::EnginePart { part: 3, .. } => 0.088,
                            _ => 0.075,
                        };
                        base * grown
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            cast: made.cast,
            share,
            branches: made.manifold.branches,
            turbos,
        }
    }

    fn short(&self) -> bool {
        self.share < 0.8
    }

    fn long(&self) -> bool {
        self.share > 1.2
    }
}

/// Places things in the body's design frame by x from the nose, height, and across
/// (+ right).
struct Frame<'a> {
    body: &'a Body,
    wheels: Wheels,
}

impl Frame<'_> {
    fn at(&self, x: f32, y: f32, across: f32) -> Vec3 {
        Vec3::new(across, y, self.body.z(x, &self.wheels))
    }
}

/// What's under `chassis`' body when it's built of `car` (its tree).
pub fn mesh(catalogue: &Catalogue, chassis: &Chassis, car: &Item) -> Mesh {
    let mut b = Builder::default();
    let frame = Frame {
        body: &chassis.body,
        wheels: chassis.wheels(),
    };
    let exhaust_start = engine_bay(&mut b, &frame, catalogue, chassis, car);
    cabin(&mut b, &frame, catalogue, chassis, car);
    underneath(&mut b, &frame, catalogue, chassis, car, exhaust_start);
    b.build()
}

/// The engine (with its gearbox beside or behind it, and a rear-wheel drive's shafts and
/// differential), radiator, intercooler and battery. Returns where the exhaust leaves the
/// engine (the manifolds' collectors or the turbine).
fn engine_bay(
    b: &mut Builder,
    frame: &Frame,
    catalogue: &Catalogue,
    chassis: &Chassis,
    car: &Item,
) -> Option<Vec<Vec3>> {
    let body = frame.body;
    let bay = &chassis.bay;
    let named = |slot: &str| part(catalogue, car, slot);
    // The radiator behind the front bumper, an intercooler low in front of it.
    let nose = body.bumpers[0] + 0.03;
    let width = 2.0 * 0.75 * body.half_width(nose);
    if let Some(radiator) = named("Radiator") {
        let core = if radiator == "Stock" { CORE } else { ALLOY };
        let y = body.floor + 0.3;
        b.cuboid(frame.at(nose, y, 0.0), Vec3::new(width, 0.34, 0.05), core);
        b.cylinder(
            frame.at(nose + 0.03, y, 0.1),
            frame.at(nose + 0.08, y, 0.1),
            0.13,
            10,
            BLACK,
        );
    }
    // A battery in the bay's front left corner, its terminals under the bonnet.
    let y = (body.floor + 0.42).min(body.deck_height(nose + 0.2) - 0.14);
    let battery = frame.at(nose + 0.2, y, -0.36 * body.width);
    b.cuboid(battery, Vec3::new(0.18, 0.17, 0.14), BLACK);
    b.cuboid(
        battery + Vec3::new(0.05, 0.09, 0.0),
        Vec3::splat(0.025),
        RED,
    );

    let engine = car.fitted(catalogue, Kind::Engine).next()?;
    let block = engine.fitted(catalogue, Kind::LongBlock).next()?;
    let Does::Block { family, version } = block.def(catalogue).does else {
        return None;
    };
    let family = &catalogue.families[family];
    let cylinders = family.versions[version].block.cylinders.max(1) as usize;
    // Its head's cams: one cover over a single cam, a wide one with a ridge over each of
    // two.
    let twin_cam = match block.fitted(catalogue, Kind::Head).next() {
        Some(head) => match head.def(catalogue).does {
            Does::Head { head, .. } => {
                (family.heads.get(head)).is_some_and(|head| head.valvetrain.drive == CamDrive::Dohc)
            }
            _ => false,
        },
        None => true,
    };
    use crate::catalogue::{CENTRIFUGAL, ROOTS, SUPERCHARGER_SLOT};
    let charger = named(SUPERCHARGER_SLOT);
    let roots = charger == Some(ROOTS);
    let centrifugal = charger == Some(CENTRIFUGAL);
    // The exhaust manifold, and the turbos on its mounts.
    let manifold = engine.fitted(catalogue, Kind::Manifold).next();
    let turbo = manifold.is_some_and(|m| m.fitted(catalogue, Kind::Turbo).next().is_some());
    if turbo && named("Intercooler").is_some_and(|cooler| cooler != "None") {
        let y = body.floor + 0.12;
        b.cuboid(
            frame.at(nose - 0.08, y, 0.0),
            Vec3::new(width, 0.16, 0.06),
            ALLOY,
        );
    }

    // The engine's own frame: u along the crank, v up, w towards the exhaust side; a
    // transverse engine's crank runs across the car (exhaust towards the bulkhead),
    // with the gearbox on the end of it, on the left. A longitudinal V's u points
    // forwards (its belt end at the front), w to the right.
    let transverse = bay.orientation == Orientation::Transverse;
    let vee = !transverse && family.layout.is_vee();
    let (along, across) = (family.length, family.width);
    let x = body.front_overhang + bay.engine_at;
    let bottom = body.floor + 0.03;
    let under_bonnet = body.deck_height(x) - 0.04;
    let height = family.height.min(under_bonnet - bottom);
    let (centre, axes) = if transverse {
        let side = 0.18;
        (frame.at(x, bottom, side), [Vec3::X, Vec3::Y, Vec3::Z])
    } else if vee {
        (frame.at(x, bottom, 0.0), [-Vec3::Z, Vec3::Y, Vec3::X])
    } else {
        (frame.at(x, bottom, 0.0), [Vec3::Z, Vec3::Y, -Vec3::X])
    };
    let to = |u: f32, v: f32, w: f32| {
        centre + axes[0] * (u * along) + axes[1] * (v * height) + axes[2] * (w * across)
    };
    let size = |u: f32, v: f32, w: f32| Vec3::new(u * along, v * height, w * across);
    let block_at = |b: &mut Builder, u: f32, v: f32, w: f32, s: Vec3, color| {
        b.oriented_box(to(u, v, w), axes, s, color);
    };
    let cover = match named("Porting") {
        Some("Stock") | None => COVER,
        Some(_) => RED,
    };
    let throttle = match named("Throttle body") {
        Some("Bigger") => 0.04,
        Some("Biggest") => 0.045,
        _ => 0.035,
    };
    // The exhaust manifold as fitted (the family's own without one), and the turbos on its
    // mounts.
    let headers = Headers::of(catalogue, manifold, family, version);
    let mut turbos = Vec::new();
    let (collectors, mouth) = if vee {
        // Oil pan and crankcase, the two banks at 90° with low heads and flat covers
        // (the cam's in the block), the intake's plenum in the valley between them with a
        // runner down to each cylinder, the throttle at its front end.
        block_at(b, 0.0, 0.06, 0.0, size(0.74, 0.12, 0.44), DARK);
        block_at(b, 0.0, 0.27, 0.0, size(0.86, 0.3, 0.56), CAST);
        block_at(b, 0.48, 0.45, 0.0, size(0.05, 0.7, 0.5), DARK);
        let scale = height / 0.6;
        let crank = to(0.0, 0.32, 0.0);
        let plenum = to(0.0, 0.84, 0.0);
        b.oriented_box(plenum, axes, size(0.66, 0.14, 0.28), ALLOY);
        let per_bank = cylinders.div_ceil(2);
        let mut collectors = Vec::new();
        // The 10-into-1's: the far (left) bank's gathered behind the engine and crossing
        // over to the right bank's collector, where its one turbo sits.
        let crossover = headers.branches == 1;
        for side in [-1.0f32, 1.0] {
            let (sin, cos) = (side * std::f32::consts::FRAC_PI_4).sin_cos();
            let up = Vec3::Y * cos + axes[2] * sin;
            let bank_axes = [axes[0], up, axes[0].cross(up)];
            let outward = side * axes[0].cross(up);
            let bank = |d: f32| crank + up * (d * scale);
            let s = |u: f32, v: f32, w: f32| Vec3::new(u * along, v * scale, w);
            b.oriented_box(bank(0.2), bank_axes, s(0.84, 0.24, 0.22), CAST);
            b.oriented_box(bank(0.35), bank_axes, s(0.88, 0.08, 0.27), ALLOY);
            b.oriented_box(bank(0.41), bank_axes, s(0.8, 0.04, 0.21), cover);
            // The exhaust ports, rearmost first (the banks are offset by a rod's width
            // along the crank).
            let ports: Vec<Vec3> = (0..per_bank)
                .map(|j| {
                    let u = ((j as f32 + 0.5) / per_bank as f32 - 0.5) * 0.72 + side * 0.02;
                    bank(0.32) + outward * 0.12 + axes[0] * (u * along)
                })
                .collect();
            let turbo = (!crossover || side > 0.0)
                .then(|| headers.turbos.first().copied())
                .flatten();
            let collector = if headers.cast {
                // A log along the bank with a stub from each port; out of its rear end
                // down to the collector, or into the turbine on its flange, low and
                // outboard.
                let log = |port: Vec3| port + outward * 0.07 - Vec3::Y * 0.02;
                for &port in &ports {
                    b.bar(port, log(port), 0.05, IRON);
                }
                let (first, last) = (log(ports[0]), log(ports[ports.len() - 1]));
                let rear = first - axes[0] * 0.05;
                b.bar(rear, last + axes[0] * 0.03, 0.075, IRON);
                match turbo {
                    Some(radius) => {
                        let turbine = rear + outward * (0.04 + radius) - Vec3::Y * 0.05;
                        b.bar(rear, turbine, 0.07, IRON);
                        turbos.push((turbine, radius));
                        turbine
                    }
                    None => {
                        let collector = to(-0.32, 0.22, side * 0.56);
                        b.bar(rear, collector, 0.065, IRON);
                        collector
                    }
                }
            } else {
                // Tubes: a primary from each port out and down to a collector behind the
                // bank, lower and further back the longer they are; the 10-into-1's
                // high, by the bulkhead.
                let collector = if crossover {
                    to(-0.42, 0.42, side * 0.6)
                } else if headers.long() {
                    to(-0.52, 0.04, side * 0.5)
                } else if headers.short() {
                    to(-0.3, 0.36, side * 0.6)
                } else {
                    to(-0.32, 0.22, side * 0.56)
                };
                for &port in &ports {
                    let bend = port + outward * 0.07 - Vec3::Y * 0.1;
                    b.bar(port, bend, 0.035, HOT);
                    b.bar(bend, collector, 0.035, HOT);
                }
                if crossover && side < 0.0 {
                    // Over the bell housing to the other bank's collector.
                    let behind = to(-0.6, 0.72, -0.25);
                    let across = to(-0.6, 0.72, 0.25);
                    b.bar(collector, behind, 0.065, HOT);
                    b.bar(behind, across, 0.065, HOT);
                    b.bar(across, to(-0.42, 0.42, 0.6), 0.065, HOT);
                    continue;
                }
                if let Some(radius) = turbo {
                    let turbine = collector + Vec3::Y * 0.02 + outward * (radius * 0.5);
                    turbos.push((turbine, radius));
                    turbine
                } else {
                    collector
                }
            };
            for &port in &ports {
                let along_u = port - bank(0.32) - outward * 0.12;
                let inlet = bank(0.36) - outward * 0.11 + along_u;
                let entry = plenum + axes[2] * (side * 0.1) + along_u - Vec3::Y * 0.02;
                b.bar(inlet, entry, 0.04, ALLOY);
            }
            collectors.push(collector);
        }
        let throat = plenum + axes[0] * (0.36 * along);
        let mouth = throat + axes[0] * 0.07;
        b.cylinder(throat, mouth, throttle, 8, ALLOY);
        (collectors, mouth)
    } else {
        // Oil pan, block, head and cam cover (red on a reworked head).
        block_at(b, 0.0, 0.07, 0.0, size(0.76, 0.14, 0.36), DARK);
        block_at(b, 0.0, 0.38, 0.0, size(0.84, 0.48, 0.42), CAST);
        block_at(b, 0.0, 0.7, 0.0, size(0.86, 0.16, 0.44), ALLOY);
        if twin_cam {
            block_at(b, 0.0, 0.84, 0.0, size(0.8, 0.12, 0.34), cover);
            for w in [-0.09, 0.09] {
                block_at(b, 0.0, 0.93, w, size(0.74, 0.08, 0.1), cover);
            }
        } else {
            block_at(b, 0.0, 0.84, 0.0, size(0.8, 0.12, 0.24), cover);
            block_at(b, 0.0, 0.92, 0.0, size(0.72, 0.05, 0.16), cover);
        }
        // The belt cover on the crank's far end, the flywheel's housing towards the
        // gearbox.
        block_at(b, 0.47, 0.5, 0.0, size(0.06, 0.7, 0.32), DARK);
        if transverse {
            let gearbox = frame.at(x + 0.04, bottom + 0.28 * height, -along / 2.0);
            b.cuboid(gearbox, Vec3::new(0.36, 0.42 * height, 0.35 * across), CAST);
        }
        // The intake: a runner per cylinder from the head out to the plenum in front,
        // longer or shorter as the manifold is, and the throttle body on its left end.
        let reach = match named("Intake manifold") {
            Some("Long runners") => 0.56,
            Some("Short runners") => 0.36,
            _ => 0.45,
        };
        let plenum = to(0.0, 0.55, -reach);
        for k in 0..cylinders {
            let u = ((k as f32 + 0.5) / cylinders as f32 - 0.5) * 0.7;
            let port = to(u, 0.68, -0.22);
            b.bar(port, to(u, 0.6, -reach + 0.04), 0.045, ALLOY);
        }
        b.oriented_box(plenum, axes, size(0.66, 0.16, 0.14), ALLOY);
        let throat = plenum - axes[0] * (0.36 * along);
        let mouth = throat - axes[0] * 0.07;
        b.cylinder(throat, mouth, throttle, 8, ALLOY);
        let ports: Vec<(f32, Vec3)> = (0..cylinders)
            .map(|k| {
                let u = ((k as f32 + 0.5) / cylinders as f32 - 0.5) * 0.7;
                (u, to(u, 0.66, 0.22))
            })
            .collect();
        let turbo = headers.turbos.first().copied();
        let collector = if headers.cast {
            // A log along the head with a stub from each port, its outlet in the middle:
            // down to the collector, or the turbine on its flange.
            let log = |u: f32| to(u, 0.6, 0.3);
            for &(u, port) in &ports {
                b.bar(port, log(u), 0.05, IRON);
            }
            b.bar(log(-0.38), log(0.38), 0.075, IRON);
            match turbo {
                Some(radius) => {
                    let turbine = log(0.0) + axes[2] * (0.04 + radius) - Vec3::Y * 0.02;
                    b.bar(log(0.0), turbine, 0.07, IRON);
                    turbos.push((turbine, radius));
                    turbine
                }
                None => {
                    let collector = to(0.0, 0.22, 0.46);
                    b.bar(log(0.0), collector, 0.065, IRON);
                    collector
                }
            }
        } else if headers.long() && cylinders == 4 {
            // 4-2-1: the primaries in pairs (the outer two, the inner two) into two
            // secondaries, low, then one.
            let collector = to(0.0, 0.05, 0.5);
            for (pair, (a, c)) in [(0, 3), (1, 2)].into_iter().enumerate() {
                let merge = to(0.12 * (pair as f32 * 2.0 - 1.0), 0.28, 0.48);
                for k in [a, c] {
                    let (u, port) = ports[k];
                    let bend = to(u * 0.8, 0.55, 0.44);
                    b.bar(port, bend, 0.035, HOT);
                    b.bar(bend, merge, 0.035, HOT);
                }
                b.bar(merge, collector, 0.045, HOT);
            }
            collector
        } else {
            // Tubes into a collector behind the engine, lower the longer they are.
            let (out, drop) = if headers.short() {
                (0.4, 0.4)
            } else if headers.long() {
                (0.5, 0.05)
            } else {
                (0.46, 0.22)
            };
            let collector = to(0.0, drop, out);
            for &(u, port) in &ports {
                let bend = to(u * 0.6, 0.55, out - 0.04);
                b.bar(port, bend, 0.035, HOT);
                b.bar(bend, collector, 0.035, HOT);
            }
            match turbo {
                Some(radius) => {
                    let turbine = collector + Vec3::Y * 0.05;
                    turbos.push((turbine, radius));
                    turbine
                }
                None => collector,
            }
        };
        (vec![collector], mouth)
    };
    // Behind a longitudinal engine its gearbox (the bell housing, then the case), and a
    // shaft to the differential at the driven rear axle, with a shaft out to each wheel.
    if !transverse {
        let start = x + along / 2.0;
        let end = (body.front_overhang + bay.gearbox_at + 0.35).max(start + 0.4);
        let y = bottom + 0.3 * height;
        b.cylinder(
            frame.at(start, y, 0.0),
            frame.at(start + 0.16, y, 0.0),
            0.19,
            10,
            CAST,
        );
        let case = Vec3::new(0.24, 0.24, end - start - 0.16);
        b.cuboid(
            frame.at((start + 0.16 + end) / 2.0, y - 0.02, 0.0),
            case,
            CAST,
        );
        if chassis.drive != Drive::Front {
            let wheels = &frame.wheels;
            let axle = body.front_overhang + wheels.wheelbase;
            let hub = wheels.radius[1];
            b.bar(
                frame.at(end, y, 0.0),
                frame.at(axle - 0.15, hub, 0.0),
                0.07,
                DARK,
            );
            b.cuboid(frame.at(axle, hub, 0.0), Vec3::new(0.3, 0.24, 0.3), CAST);
            let reach = wheels.track[1] / 2.0 - wheels.width[1] / 2.0 - 0.04;
            for side in [-1.0, 1.0] {
                let (from, to) = (side * 0.15, side * reach);
                b.bar(
                    frame.at(axle, hub, from),
                    frame.at(axle, hub, to),
                    0.05,
                    DARK,
                );
            }
        }
    }
    // A turbo at each turbine inlet: the turbine's snail, the compressor's beside it
    // (towards the intake's end) and its pipe to the throttle (through the intercooler,
    // which the pipes don't show).
    let intake_end = if vee { 1.0 } else { -1.0 };
    let mut intakes = vec![mouth + axes[0] * (0.05 * intake_end)];
    let mut exhaust = collectors.clone();
    if !turbos.is_empty() {
        exhaust.clear();
        intakes.clear();
        for &(turbine, radius) in &turbos {
            let width = radius * 1.05;
            b.cylinder(
                turbine - axes[0] * (width / 2.0),
                turbine + axes[0] * (width / 2.0),
                radius,
                12,
                IRON,
            );
            let compressor = turbine + axes[0] * (intake_end * (width + 0.02));
            b.cylinder(
                compressor - axes[0] * (radius * 0.5),
                compressor + axes[0] * (radius * 0.5),
                radius * 1.13,
                12,
                ALLOY,
            );
            b.bar(compressor + Vec3::Y * radius, mouth, 0.06, ALLOY);
            // The turbine's outlet, down into the exhaust; the compressor's inlet.
            exhaust.push(turbine - Vec3::Y * (radius + 0.01));
            intakes.push(compressor + axes[0] * (intake_end * (radius * 0.5 + 0.06)));
        }
    }
    // Where a bonnet's cut-out goes (over the intake, where a Roots blower sticks up
    // through it): its middle, half its length along the engine's crank and across, and
    // the bonnet's highest point round it (a bulge's top where it has one).
    let cut_out_middle = to(0.0, 0.92, if vee { 0.0 } else { -0.18 });
    let (half_u, half_w) = (0.32 * along + 0.04, 0.16);
    let cut_out_top = [-1.0, 1.0]
        .into_iter()
        .flat_map(|su| [-1.0, 1.0].map(|sw| (su, sw)))
        .map(|(su, sw)| {
            let p = cut_out_middle + axes[0] * (su * half_u) + axes[2] * (sw * half_w);
            body.bonnet_top(body.x(p.z, &frame.wheels), p.x)
        })
        .fold(f32::MIN, f32::max);
    // A Roots blower on top of the engine, over the intake, sticking up through the
    // bonnet's cut-out; its belt from the
    // crank's pulley at the belt end, the air filter's pipe to its inlet.
    if roots {
        let base = to(0.0, 0.92, if vee { 0.0 } else { -0.18 });
        let top = cut_out_top + 0.12;
        let rise = (top - base.y).max(0.12);
        let centre = base + Vec3::Y * (rise / 2.0);
        b.oriented_box(centre, axes, Vec3::new(0.58 * along, rise, 0.24), ALLOY);
        for k in 0..5 {
            let u = (k as f32 / 4.0 - 0.5) * 0.5 * along;
            b.oriented_box(
                centre + axes[0] * u + Vec3::Y * 0.02,
                axes,
                Vec3::new(0.015, rise - 0.02, 0.26),
                CAST,
            );
        }
        // The scoop on top, and the pulley and belt down to the crank.
        b.oriented_box(
            Vec3::new(centre.x, top + 0.04, centre.z),
            axes,
            Vec3::new(0.32 * along, 0.08, 0.2),
            BLACK,
        );
        let pulley = base + axes[0] * (0.34 * along) + Vec3::Y * 0.04;
        b.cylinder(
            pulley - axes[0] * 0.02,
            pulley + axes[0] * 0.02,
            0.06,
            10,
            DARK,
        );
        b.bar(pulley, to(0.47, 0.25, 0.0), 0.02, BLACK);
        intakes = vec![base + axes[0] * (0.3 * along * intake_end) + Vec3::Y * 0.04];
    }
    // A bonnet with a cut-out: the opening over the intake (where a Roots blower sticks
    // through) framed in the bonnet's paint, dark down into the engine bay when there's no
    // blower in it.
    if let Some(bonnet) = car.fitted(catalogue, Kind::Body(Panel::Bonnet)).next()
        && (bonnet.def(catalogue).provides.iter()).any(|p| p == BONNET_CUT_OUT)
    {
        let paint = bonnet.paint.or(car.paint).unwrap_or(0);
        let color = catalogue.paints.get(paint).map_or([0.5; 3], |p| p.color);
        let ring = Vec3::new(cut_out_middle.x, cut_out_top + 0.01, cut_out_middle.z);
        for (u, w, long) in [
            (0.0, half_w, true),
            (0.0, -half_w, true),
            (half_u, 0.0, false),
            (-half_u, 0.0, false),
        ] {
            let size = if long {
                Vec3::new(2.0 * half_u + 0.04, 0.04, 0.04)
            } else {
                Vec3::new(0.04, 0.04, 2.0 * half_w)
            };
            b.oriented_box(ring + axes[0] * u + axes[2] * w, axes, size, color);
        }
        if !roots {
            let size = Vec3::new(2.0 * half_u, 0.02, 2.0 * half_w);
            b.oriented_box(ring + Vec3::Y * 0.005, axes, size, BLACK);
        }
    }
    // A centrifugal supercharger at the belt end, low on the intake side: its snail, the
    // belt from the crank, and its pipe up to the throttle.
    if centrifugal {
        let snail = to(0.42, 0.42, -0.42);
        b.cylinder(
            snail - axes[0] * 0.05,
            snail + axes[0] * 0.05,
            0.1,
            12,
            ALLOY,
        );
        b.cylinder(
            snail + axes[0] * 0.05,
            snail + axes[0] * 0.09,
            0.05,
            10,
            DARK,
        );
        b.bar(snail + axes[0] * 0.07, to(0.47, 0.2, 0.0), 0.02, BLACK);
        b.bar(snail + Vec3::Y * 0.1, mouth, 0.05, ALLOY);
        intakes = vec![snail - axes[0] * 0.08];
    }
    // The air filter: the stock box (a panel filter in it, red if it's the sport one)
    // over the gearbox with a pipe to the throttle or each turbo's compressor (ahead of a
    // longitudinal V, at the front of its intake), or an open cone on each pipe's end.
    let filter = if vee {
        let ahead = x - along / 2.0 - 0.22;
        frame.at(ahead, body.deck_height(ahead) - 0.16, 0.0)
    } else {
        frame.at(x - 0.1, under_bonnet - 0.09, -0.34 * body.width)
    };
    let filter_part = named("Air filter");
    for &from in &intakes {
        if filter_part == Some("Open cone") {
            let end = from.lerp(filter, 0.7);
            b.bar(from, end, 0.06, BLACK);
            b.cylinder(
                end,
                end + (end - from).normalize_or_zero() * 0.16,
                0.07,
                10,
                RED,
            );
        } else {
            b.bar(from, filter, 0.06, BLACK);
        }
    }
    if filter_part != Some("Open cone") {
        b.cuboid(filter, Vec3::new(0.26, 0.12, 0.22), BLACK);
        if filter_part == Some("Sport panel") {
            b.cuboid(filter + Vec3::Y * 0.065, Vec3::new(0.2, 0.01, 0.16), RED);
        }
    }
    Some(exhaust)
}

/// The seats fitted and a dashboard across the cabin's front.
fn cabin(b: &mut Builder, frame: &Frame, catalogue: &Catalogue, chassis: &Chassis, car: &Item) {
    let body = frame.body;
    let Some((start, _)) = body.cockpit() else {
        return;
    };
    let deck = body.deck_height(start + 0.2);
    let width = 2.0 * body.cabin_half_width(start + 0.3);
    b.cuboid(
        frame.at(start + 0.2, deck - 0.1, 0.0),
        Vec3::new(width, 0.16, 0.4),
        TRIM,
    );
    let [eye, eye_height, left] = chassis.eye;
    let floor = body.floor + 0.08;
    // The cushions' middle: off the floor, lower for a driver sitting low.
    let cushion = (floor + 0.2).min(eye_height - 0.68);
    // Carpet over the floor pan, between the wells.
    let rear = body.front_overhang + chassis.wheelbase - 0.4;
    b.cuboid(
        frame.at((start + rear) / 2.0, floor - 0.04, 0.0),
        Vec3::new(width, 0.02, rear - start),
        CARPET,
    );
    let mut fronts = 0;
    let slots = &car.def(catalogue).slots;
    for (slot, item) in slots.iter().zip(&car.slots) {
        let Kind::Seat(seat) = slot.kind else {
            continue;
        };
        let front = seat == Seat::Front;
        if front {
            fronts += 1;
        }
        if item.is_none() {
            continue;
        }
        if front {
            // The driver's on the left, the passenger on the right.
            let across = if fronts == 1 { -left } else { left };
            let x = eye + 0.1;
            b.cuboid(
                frame.at(x, cushion, across),
                Vec3::new(0.48, 0.12, 0.5),
                CLOTH,
            );
            let lean = Quat::from_rotation_x(-0.25);
            let axes = [lean * Vec3::X, lean * Vec3::Y, lean * Vec3::Z];
            let back = frame.at(x + 0.3, cushion + 0.35, across);
            b.oriented_box(back, axes, Vec3::new(0.48, 0.62, 0.1), CLOTH);
            let head = back + axes[1] * 0.38;
            b.oriented_box(head, axes, Vec3::new(0.24, 0.16, 0.08), CLOTH);
        } else {
            let x = eye + 1.0;
            let bench = 2.0 * 0.85 * body.cabin_half_width(x);
            b.cuboid(
                frame.at(x, cushion, 0.0),
                Vec3::new(bench, 0.12, 0.45),
                CLOTH,
            );
            let lean = Quat::from_rotation_x(-0.3);
            let axes = [lean * Vec3::X, lean * Vec3::Y, lean * Vec3::Z];
            let back = frame.at(x + 0.28, cushion + 0.28, 0.0);
            b.oriented_box(back, axes, Vec3::new(bench, 0.55, 0.1), CLOTH);
        }
    }
}

/// The fuel tank and the exhaust line under the floor, from where it leaves the engine
/// to the tailpipe under the rear bumper.
fn underneath(
    b: &mut Builder,
    frame: &Frame,
    catalogue: &Catalogue,
    chassis: &Chassis,
    car: &Item,
    from: Option<Vec<Vec3>>,
) {
    let body = frame.body;
    let floor = body.floor;
    let tank = body.front_overhang + chassis.tank.at;
    let litres = chassis.tank.litres / 1000.0;
    let depth = 0.18;
    let (length, width) = (
        0.8 * (litres / depth).sqrt(),
        1.25 * (litres / depth).sqrt(),
    );
    let tank_at = frame.at(tank, floor + 0.04, -0.4 * width + 0.05);
    b.cuboid(tank_at, Vec3::new(width, depth, length), DARK);

    let Some(starts) = from.filter(|starts| !starts.is_empty()) else {
        return;
    };
    let named = |slot: &str| part(catalogue, car, slot);
    let radius = match named("Exhaust pipe") {
        Some("Big bore") => 0.03,
        _ => 0.024,
    };
    let low = floor - HANG;
    let pipe = |b: &mut Builder, a: Vec3, c: Vec3| b.bar(a, c, 2.0 * radius, HOT);
    let catalyst = |b: &mut Builder, from: Vec3, to: Vec3| match named("Catalyst") {
        Some("None") | None => pipe(b, from, to),
        Some(kind) => {
            let size = if kind == "High-flow" { 0.14 } else { 0.12 };
            let length = (to - from).length();
            let axes = [Vec3::X, Vec3::Y, Vec3::Z];
            let turned = Quat::from_rotation_arc(Vec3::Z, (to - from).normalize_or(Vec3::Z));
            let axes = axes.map(|axis| turned * axis);
            let box_size = Vec3::new(size + 0.04, size, length);
            b.oriented_box((from + to) / 2.0, axes, box_size, METAL);
        }
    };
    if body.side_pipes {
        // Out at the sides: from each bank's collector down and back past the catalyst,
        // out under the sill, and along it in a pipe outside the body (the silencer's
        // in it) that ends just ahead of the rear wheel.
        let wheels = &frame.wheels;
        let arch = body.front_overhang + wheels.wheelbase - wheels.radius[1] - 0.07;
        let (first, last) = (arch - 1.0, arch - 0.06);
        let sides: Vec<(Vec3, f32)> = match starts.as_slice() {
            [one] => vec![(*one, -1.0), (*one, 1.0)],
            all => (all.iter()).map(|p| (*p, p.x.signum())).collect(),
        };
        for (start, side) in sides {
            let x = body.x(start.z, wheels);
            let cat_in = frame.at(x + 0.25, low + 0.02, side * 0.34);
            let cat_out = frame.at(x + 0.75, low + 0.02, side * 0.38);
            pipe(b, start, cat_in);
            catalyst(b, cat_in, cat_out);
            let out = body.half_width(first) + 0.04;
            let under = frame.at(first - 0.12, floor + 0.04, side * (out - 0.18));
            pipe(b, cat_out, under);
            let y = floor + 0.09;
            let (front, back) = (
                frame.at(first, y, side * out),
                frame.at(last, y, side * out),
            );
            pipe(b, under, front);
            match named("Silencer") {
                Some(name) if !name.starts_with("None") => {
                    let fat = if name == "Sport" { 0.06 } else { 0.052 };
                    b.cylinder(front, back, fat, 10, METAL);
                    b.cylinder(back, back + Vec3::Z * 0.03, 0.7 * fat, 8, DARK);
                }
                _ => b.cylinder(front, back, 1.4 * radius, 8, METAL),
            }
        }
        return;
    }
    let side = (tank_at.x + width / 2.0 + radius + 0.04).max(0.12);
    let from = starts[0];
    let start = body.x(from.z, &frame.wheels);
    let tail = body.length;
    let silencer = tail - 0.62;
    let down = frame.at(start + 0.35, low, 0.06);
    let cat = [down.z + 0.05, down.z + 0.5];
    for &from in &starts {
        pipe(b, from, down);
    }
    let cat_in = down + Vec3::Z * 0.05;
    let cat_out = Vec3::new(0.06, low, cat[1]);
    match named("Catalyst") {
        Some("None") | None => pipe(b, down, cat_out),
        Some(kind) => {
            pipe(b, down, cat_in);
            let size = if kind == "High-flow" { 0.14 } else { 0.12 };
            b.cuboid(
                (cat_in + cat_out) / 2.0,
                Vec3::new(size + 0.04, size, cat_out.z - cat_in.z),
                METAL,
            );
        }
    }
    let past_tank = frame.at(tank - length, low, side);
    pipe(b, cat_out, past_tank);
    let along_tank = frame.at(tank + length / 2.0 + 0.1, low, side);
    pipe(b, past_tank, along_tank);
    let can_in = frame.at(silencer, floor + 0.04, side.max(0.3));
    pipe(b, along_tank, can_in);
    let tip_y = floor + UNDER_TIP;
    let can_out = frame.at(tail - 0.1, tip_y, can_in.x);
    match named("Silencer") {
        Some(name) if !name.starts_with("None") => {
            let (tall, wide) = if name == "Sport" {
                (0.17, 0.3)
            } else {
                (0.14, 0.26)
            };
            let middle = (can_in + can_out) / 2.0;
            b.cuboid(
                middle.with_y(floor + 0.05),
                Vec3::new(wide, tall, can_out.z - can_in.z),
                METAL,
            );
            let tip = if name == "Sport" { 0.045 } else { 0.03 };
            b.cylinder(
                can_out,
                frame.at(tail + 0.03, tip_y, can_in.x),
                tip,
                8,
                METAL,
            );
        }
        _ => {
            pipe(b, can_in, can_out);
            b.cylinder(
                can_out,
                frame.at(tail + 0.03, tip_y, can_in.x),
                radius,
                8,
                METAL,
            );
        }
    }
}

/// How far above the floor the tailpipe leaves under the rear bumper.
const UNDER_TIP: f32 = 0.06;

#[cfg(test)]
mod tests {
    use super::*;

    fn triangles(mesh: &Mesh) -> usize {
        mesh.count_vertices() / 3
    }

    #[test]
    fn what_is_fitted_is_drawn() {
        let catalogue = crate::catalogue::builtin();
        for trim in &catalogue.trims {
            let Does::Chassis(c) = trim.car.def(&catalogue).does else {
                panic!("not a car");
            };
            let chassis = &catalogue.chassis[c];
            let mesh = mesh(&catalogue, chassis, &trim.car);
            let count = triangles(&mesh);
            assert!((300..4000).contains(&count), "{}: {count}", trim.name);
            // Within the body's length and width, above the ground.
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("no positions");
            };
            // Side pipes stand out of the body.
            let body = &chassis.body;
            let wide = body.width / 2.0 + if body.side_pipes { 0.12 } else { 0.0 };
            for p in positions {
                let p = Vec3::from_array(*p);
                assert!(p.y > 0.0 && p.y < body.height(), "{p}");
                assert!(p.x.abs() < wide, "{}: {p}", trim.name);
            }
            // Seats out, fewer triangles; another key.
            let mut stripped = trim.car.clone();
            let slots = stripped.def(&catalogue).slots.clone();
            for (slot, item) in slots.iter().zip(&mut stripped.slots) {
                if matches!(slot.kind, Kind::Seat(_)) {
                    *item = None;
                }
            }
            assert!(triangles(&super::mesh(&catalogue, chassis, &stripped)) < count);
            assert_ne!(key(&stripped), key(&trim.car));
        }
        // A turbo shows.
        let chassis = &catalogue.chassis[0];
        let car = &catalogue.trims[0].car;
        let setup = crate::car::CarSetup::factory(0, 0);
        let mut parts = setup.engine.parts.clone();
        parts.chosen[crate::catalogue::TURBO] = 2;
        parts.manifold = parts.turbo_manifold().unwrap();
        let turbo = crate::items::assemble(&catalogue, Some(car), &setup.build, &parts).unwrap();
        assert_eq!(
            part(&catalogue, &turbo, crate::catalogue::TURBO_SLOT),
            Some("Medium")
        );
        let slot = crate::catalogue::SUPERCHARGER;
        fn swap(item: &mut Item, catalogue: &Catalogue, slot: usize, def: usize) {
            let kinds: Vec<Kind> = (item.def(catalogue).slots.iter()).map(|s| s.kind).collect();
            for (kind, child) in kinds.into_iter().zip(&mut item.slots) {
                if let Some(child) = child {
                    if kind == Kind::EnginePart(slot) {
                        child.def = def;
                    } else {
                        swap(child, catalogue, slot, def);
                    }
                }
            }
        }
        let (plain, boosted) = (
            triangles(&mesh(&catalogue, chassis, car)),
            triangles(&mesh(&catalogue, chassis, &turbo)),
        );
        assert!(boosted > plain, "{boosted} vs {plain}");
        // So do the superchargers.
        for part in [1, 2] {
            let def = (catalogue.defs.iter())
                .position(|d| d.does == Does::EnginePart { slot, part })
                .unwrap();
            let mut charged = car.clone();
            swap(&mut charged, &catalogue, slot, def);
            assert!(triangles(&mesh(&catalogue, chassis, &charged)) > plain);
        }
    }

    /// `car` with the manifold `id` on its engine and the turbo `turbo` on every mount.
    fn with_manifold(catalogue: &Catalogue, car: &Item, id: &str, turbo: Option<&str>) -> Item {
        let def = |id: &str| (catalogue.defs.iter()).position(|d| d.id == id).unwrap();
        let mut car = car.clone();
        let engine_at = (car.def(catalogue).slots.iter())
            .position(|s| s.kind == Kind::Engine)
            .unwrap();
        let engine = car.slots[engine_at].as_mut().unwrap();
        let at = (engine.def(catalogue).slots.iter())
            .position(|s| s.kind == Kind::Manifold)
            .unwrap();
        let mut manifold = Item::new(catalogue, def(id), 0);
        let mounts = manifold.def(catalogue).slots.clone();
        for (slot, mount) in mounts.iter().zip(&mut manifold.slots) {
            if let (Kind::Turbo, Some(turbo)) = (slot.kind, turbo) {
                *mount = Some(Item::new(catalogue, def(turbo), 0));
            }
        }
        engine.slots[at] = Some(manifold);
        assert!(car.fine(catalogue), "{id} with {turbo:?}");
        car
    }

    #[test]
    fn every_manifold_is_drawn_with_a_turbo_on_each_mount() {
        let catalogue = crate::catalogue::builtin();
        for trim in &catalogue.trims {
            let Does::Chassis(c) = trim.car.def(&catalogue).does else {
                panic!("not a car");
            };
            let chassis = &catalogue.chassis[c];
            let body = &chassis.body;
            let engine = trim.car.fitted(&catalogue, Kind::Engine).next().unwrap();
            let block = engine.fitted(&catalogue, Kind::LongBlock).next().unwrap();
            let Does::Block { family, .. } = block.def(&catalogue).does else {
                panic!("no block");
            };
            let mut seen = Vec::new();
            for made in &catalogue.families[family].manifolds {
                let turbo = made.mounts.then_some("turbocharger.medium");
                let car = with_manifold(&catalogue, &trim.car, &made.id, turbo);
                let mesh = mesh(&catalogue, chassis, &car);
                let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    panic!("no positions");
                };
                let wide = body.width / 2.0 + if body.side_pipes { 0.12 } else { 0.0 };
                for p in positions {
                    let p = Vec3::from_array(*p);
                    assert!(p.y > 0.0 && p.y < body.height(), "{}: {p}", made.id);
                    assert!(p.x.abs() < wide, "{}: {p}", made.id);
                }
                // Every manifold looks different, and the empty mounts less than full.
                let count = triangles(&mesh);
                assert!(!seen.contains(&count), "{}: {count}", made.id);
                seen.push(count);
                if made.mounts {
                    let empty = with_manifold(&catalogue, &trim.car, &made.id, None);
                    let bare = triangles(&super::mesh(&catalogue, chassis, &empty));
                    assert!(bare < count, "{}: {bare} {count}", made.id);
                }
            }
        }
        // The R80's pair of turbo manifolds carries two turbos, the 10-into-1 one: the
        // pair more to draw (a snail, a compressor and their pipes each).
        let merrow = (catalogue.trims.iter())
            .find(|t| t.id == "merrow.r80")
            .unwrap();
        let chassis = &catalogue.chassis[1];
        let [pair, crossover] = ["r.manifolds.turbo", "r.manifold.turbo-crossover"].map(|id| {
            let car = with_manifold(&catalogue, &merrow.car, id, Some("turbocharger.medium"));
            let empty = with_manifold(&catalogue, &merrow.car, id, None);
            triangles(&mesh(&catalogue, chassis, &car))
                - triangles(&mesh(&catalogue, chassis, &empty))
        });
        assert!(pair > crossover, "{pair} {crossover}");
    }
}
