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

const CAST: [f32; 3] = [0.42, 0.43, 0.45];
const ALLOY: [f32; 3] = [0.68, 0.69, 0.7];
const COVER: [f32; 3] = [0.08, 0.08, 0.09];
const RED: [f32; 3] = [0.72, 0.08, 0.06];
const HOT: [f32; 3] = [0.4, 0.31, 0.26];
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

/// The engine (with its gearbox beside it), radiator, intercooler and battery. Returns
/// where the exhaust leaves the engine (the manifold's collector or the turbine).
fn engine_bay(
    b: &mut Builder,
    frame: &Frame,
    catalogue: &Catalogue,
    chassis: &Chassis,
    car: &Item,
) -> Option<Vec3> {
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
    // A battery in the bay's front left corner.
    let battery = frame.at(nose + 0.2, body.floor + 0.42, -0.36 * body.width);
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
    use crate::catalogue::{CENTRIFUGAL, ROOTS, TURBO_SLOT};
    let charger = named(TURBO_SLOT);
    let roots = charger == Some(ROOTS);
    let centrifugal = charger == Some(CENTRIFUGAL);
    let turbo = charger.is_some_and(|turbo| turbo != "None") && !roots && !centrifugal;
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
    // with the gearbox on the end of it, on the left.
    let transverse = bay.orientation == Orientation::Transverse;
    let (along, across) = (family.length, family.width);
    let x = body.front_overhang + bay.engine_at;
    let bottom = body.floor + 0.03;
    let under_bonnet = body.deck_height(x) - 0.04;
    let height = family.height.min(under_bonnet - bottom);
    let (centre, axes) = if transverse {
        let side = 0.18;
        (frame.at(x, bottom, side), [Vec3::X, Vec3::Y, Vec3::Z])
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
    // Oil pan, block, head and cam cover (red on a reworked head).
    block_at(b, 0.0, 0.07, 0.0, size(0.76, 0.14, 0.36), DARK);
    block_at(b, 0.0, 0.38, 0.0, size(0.84, 0.48, 0.42), CAST);
    block_at(b, 0.0, 0.7, 0.0, size(0.86, 0.16, 0.44), ALLOY);
    let cover = match named("Cylinder head") {
        Some("Stock") | None => COVER,
        Some(_) => RED,
    };
    block_at(b, 0.0, 0.84, 0.0, size(0.8, 0.12, 0.34), cover);
    // The belt cover on the crank's far end, the flywheel's housing towards the gearbox.
    block_at(b, 0.47, 0.5, 0.0, size(0.06, 0.7, 0.32), DARK);
    if transverse {
        let gearbox = frame.at(x + 0.04, bottom + 0.28 * height, -along / 2.0);
        b.cuboid(gearbox, Vec3::new(0.36, 0.42 * height, 0.35 * across), CAST);
    }
    // The intake: a runner per cylinder from the head out to the plenum in front, longer
    // or shorter as the manifold is, and the throttle body on its left end.
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
    let throttle = match named("Throttle body") {
        Some("Bigger") => 0.04,
        Some("Biggest") => 0.045,
        _ => 0.035,
    };
    let throat = plenum - axes[0] * (0.36 * along);
    let mouth = throat - axes[0] * 0.07;
    b.cylinder(throat, mouth, throttle, 8, ALLOY);
    // The exhaust manifold: primaries from the head to a collector behind the engine,
    // lower the longer they are.
    let (out, drop) = match named("Exhaust manifold") {
        Some("Short headers") => (0.4, 0.4),
        Some("Long-tube headers") => (0.5, 0.05),
        _ => (0.46, 0.22),
    };
    let collector = to(0.0, drop, out);
    for k in 0..cylinders {
        let u = ((k as f32 + 0.5) / cylinders as f32 - 0.5) * 0.7;
        let port = to(u, 0.66, 0.22);
        let bend = to(u * 0.6, 0.55, out - 0.04);
        b.bar(port, bend, 0.035, HOT);
        b.bar(bend, collector, 0.035, HOT);
    }
    // A turbo at the collector: the turbine's snail, the compressor's beside it and its
    // pipe up to the throttle (through the intercooler, which the pipes don't show).
    let mut exhaust = collector;
    let mut intake_from = mouth - axes[0] * 0.05;
    if turbo {
        let turbine = collector + Vec3::Y * 0.05;
        b.cylinder(
            turbine - axes[0] * 0.04,
            turbine + axes[0] * 0.04,
            0.075,
            10,
            HOT,
        );
        let compressor = turbine + axes[0] * 0.1;
        b.cylinder(
            compressor - axes[0] * 0.035,
            compressor + axes[0] * 0.035,
            0.085,
            10,
            ALLOY,
        );
        b.bar(compressor + Vec3::Y * 0.08, mouth, 0.06, ALLOY);
        exhaust = turbine - Vec3::Y * 0.08;
        intake_from = compressor + axes[0] * 0.12;
    }
    // A Roots blower on top of the engine, over the intake, sticking up through the
    // bonnet (its cut-out's frame around it, in the bonnet's paint); its belt from the
    // crank's pulley at the belt end, the air filter's pipe to its inlet.
    if roots {
        let base = to(0.0, 0.92, -0.18);
        let top = under_bonnet + 0.16;
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
        if let Some(bonnet) = car.fitted(catalogue, Kind::Body(Panel::Bonnet)).next()
            && (bonnet.def(catalogue).provides.iter()).any(|p| p == BONNET_CUT_OUT)
        {
            let paint = bonnet.paint.or(car.paint).unwrap_or(0);
            let color = catalogue.paints.get(paint).map_or([0.5; 3], |p| p.color);
            let y = under_bonnet + 0.05;
            let (half_u, half_w) = (0.32 * along + 0.04, 0.16);
            let ring = Vec3::new(centre.x, y, centre.z);
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
        }
        intake_from = base - axes[0] * (0.3 * along) + Vec3::Y * 0.04;
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
        intake_from = snail - axes[0] * 0.08;
    }
    // The air filter: the stock box (a panel filter in it, red if it's the sport one)
    // over the gearbox with a pipe to the throttle, or an open cone on the pipe's end.
    let filter = frame.at(x - 0.1, under_bonnet - 0.09, -0.34 * body.width);
    match named("Air filter") {
        Some("Open cone") => {
            let end = intake_from.lerp(filter, 0.7);
            b.bar(intake_from, end, 0.06, BLACK);
            b.cylinder(
                end,
                end + (end - intake_from).normalize_or_zero() * 0.16,
                0.07,
                10,
                RED,
            );
        }
        filter_part => {
            b.bar(intake_from, filter, 0.06, BLACK);
            b.cuboid(filter, Vec3::new(0.26, 0.12, 0.22), BLACK);
            if filter_part == Some("Sport panel") {
                b.cuboid(filter + Vec3::Y * 0.065, Vec3::new(0.2, 0.01, 0.16), RED);
            }
        }
    }
    Some(exhaust)
}

/// The seats fitted and a dashboard across the cabin's front.
fn cabin(b: &mut Builder, frame: &Frame, catalogue: &Catalogue, chassis: &Chassis, car: &Item) {
    let body = frame.body;
    let Some((start, _)) = body.greenhouse() else {
        return;
    };
    let deck = body.deck_height(start + 0.2);
    let width = 2.0 * body.cabin_half_width(start + 0.3);
    b.cuboid(
        frame.at(start + 0.2, deck - 0.1, 0.0),
        Vec3::new(width, 0.16, 0.4),
        TRIM,
    );
    let [eye, _, left] = chassis.eye;
    let floor = body.floor + 0.08;
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
                frame.at(x, floor + 0.2, across),
                Vec3::new(0.48, 0.12, 0.5),
                CLOTH,
            );
            let lean = Quat::from_rotation_x(-0.25);
            let axes = [lean * Vec3::X, lean * Vec3::Y, lean * Vec3::Z];
            let back = frame.at(x + 0.3, floor + 0.55, across);
            b.oriented_box(back, axes, Vec3::new(0.48, 0.62, 0.1), CLOTH);
            let head = back + axes[1] * 0.38;
            b.oriented_box(head, axes, Vec3::new(0.24, 0.16, 0.08), CLOTH);
        } else {
            let x = eye + 1.0;
            let bench = 2.0 * 0.85 * body.cabin_half_width(x);
            b.cuboid(
                frame.at(x, floor + 0.2, 0.0),
                Vec3::new(bench, 0.12, 0.45),
                CLOTH,
            );
            let lean = Quat::from_rotation_x(-0.3);
            let axes = [lean * Vec3::X, lean * Vec3::Y, lean * Vec3::Z];
            let back = frame.at(x + 0.28, floor + 0.48, 0.0);
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
    from: Option<Vec3>,
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

    let Some(from) = from else {
        return;
    };
    let named = |slot: &str| part(catalogue, car, slot);
    let radius = match named("Exhaust pipe") {
        Some("Big bore") => 0.03,
        _ => 0.024,
    };
    let low = floor - HANG;
    let side = (tank_at.x + width / 2.0 + radius + 0.04).max(0.12);
    let start = body.x(from.z, &frame.wheels);
    let tail = body.length;
    let silencer = tail - 0.62;
    let down = frame.at(start + 0.35, low, 0.06);
    let cat = [down.z + 0.05, down.z + 0.5];
    let pipe = |b: &mut Builder, a: Vec3, c: Vec3| b.bar(a, c, 2.0 * radius, HOT);
    pipe(b, from, down);
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
        let chassis = &catalogue.chassis[0];
        for trim in &catalogue.trims {
            let mesh = mesh(&catalogue, chassis, &trim.car);
            let count = triangles(&mesh);
            assert!((300..4000).contains(&count), "{}: {count}", trim.name);
            // Within the body's length and width, above the ground.
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("no positions");
            };
            let body = &chassis.body;
            for p in positions {
                let p = Vec3::from_array(*p);
                assert!(p.y > 0.0 && p.y < body.height(), "{p}");
                assert!(p.x.abs() < body.width / 2.0, "{p}");
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
        let car = &catalogue.trims[0].car;
        let mut turbo = car.clone();
        let slot = (catalogue.slots.iter())
            .position(|s| s.name == crate::catalogue::TURBO_SLOT)
            .unwrap();
        let def = (catalogue.defs.iter())
            .position(|d| d.does == Does::EnginePart { slot, part: 2 })
            .unwrap();
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
        swap(&mut turbo, &catalogue, slot, def);
        assert_eq!(
            part(&catalogue, &turbo, crate::catalogue::TURBO_SLOT),
            Some("Medium")
        );
        let (plain, boosted) = (
            triangles(&mesh(&catalogue, chassis, car)),
            triangles(&mesh(&catalogue, chassis, &turbo)),
        );
        assert!(boosted > plain, "{boosted} vs {plain}");
        // So do the superchargers.
        for part in [4, 5] {
            let def = (catalogue.defs.iter())
                .position(|d| d.does == Does::EnginePart { slot, part })
                .unwrap();
            let mut charged = car.clone();
            swap(&mut charged, &catalogue, slot, def);
            assert!(triangles(&mesh(&catalogue, chassis, &charged)) > plain);
        }
    }
}
