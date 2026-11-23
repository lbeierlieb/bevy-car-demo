//! The gas stations: the car at a pump under the station's canopy, the filler flap open
//! and the nozzle in, the hose from the pump. The window on the right picks the grade and
//! how much, and pumps it into the driven car at a pump's pace; the pump's display shows
//! the litres ticking up.
//!
//! The forecourt is high up over the shop (on a meadow of its own, as far as the eye
//! goes); the driven car stays parked on the shop's marker meanwhile (a ghost, its input
//! held like in the dyno's room), and the car at the pump is drawn from its looks, the
//! engine off.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, RichText, Stroke};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::audio::{EngineAudio, EngineRun};
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car, CarSetup};
use crate::chassis::PAINTS;
use crate::looks::{Looks, Motion};
use crate::shops::{AtShop, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;

/// The grades: name, research octane number, and the colour of the nozzle's grip.
pub const GRADES: [(&str, f32, [f32; 3]); 3] = [
    ("Super 95", 95.0, [0.10, 0.55, 0.20]),
    ("Super Plus 98", 98.0, [0.12, 0.30, 0.80]),
    ("Racing 102", 102.0, [0.85, 0.18, 0.10]),
];

/// How fast a pump fills (L/s): quicker than a real one (about 0.7), so a tank doesn't
/// take a minute.
const FLOW: f32 = 2.0;
/// How far over the ground at the shop the forecourt is (m).
const ABOVE: f32 = 600.0;
/// The pump island: its width, height and length (m), and the gap between it and the car.
const ISLAND: Vec3 = Vec3::new(1.2, 0.15, 7.5);
const ISLAND_GAP: f32 = 0.6;
/// The pump: across the island, up and along it (m), and how far ahead of the filler.
const PUMP: Vec3 = Vec3::new(0.5, 1.85, 1.05);
const PUMP_AHEAD: f32 = 1.3;
/// The display on the pump's outer face: its size, and how high its middle is over the
/// island (m).
const DISPLAY: Vec2 = Vec2::new(0.66, 0.36);
const DISPLAY_UP: f32 = 1.4;
/// The canopy's underside over the ground (m).
const CANOPY: f32 = 4.8;
/// The lanes between the islands, from island middle to island middle (m).
const LANE: f32 = 5.2;
/// The camera: where it looks (over the island, between the pump and the filler) and from
/// where, how far it may swing round (rad), and how near and far it may zoom.
const LOOK_AT: Vec3 = Vec3::new(-0.6, 1.0, 0.4);
const EYE: Vec3 = Vec3::new(4.0, 0.75, 0.6);
const VIEW_SWING: f32 = 1.5;
const ZOOM: std::ops::RangeInclusive<f32> = 0.3..=1.6;
/// The display's backlit glass and its digits.
const LCD: Color32 = Color32::from_rgb(165, 188, 150);
const LCD_INK: Color32 = Color32::from_rgb(18, 28, 22);

pub struct GasPlugin;

impl Plugin for GasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Station>()
            .add_systems(
                Update,
                (
                    leave_when_gone,
                    (fill, dress, look.after(crate::camera::follow)).run_if(at_pump),
                )
                    .chain(),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (shop, (display, window).run_if(at_pump)),
            );
    }
}

/// The forecourt, while the car is at the pump: where it is, the camera's swing and zoom,
/// the display, and the pump as set.
#[derive(Resource, Default)]
struct Station {
    inside: bool,
    origin: Vec3,
    /// Where the camera looks (in the forecourt's frame), and its swing and zoom.
    target: Vec3,
    swing: f32,
    zoom: f32,
    eye: Option<Vec3>,
    /// The display's middle, and its right and up (as seen from in front), in the world.
    display: [Vec3; 3],
    /// The grade (in `GRADES`) and the litres to pump, as last set (kept between visits).
    grade: usize,
    litres: f32,
    /// Pumping, up to this much for this fill (L); and how much went in so far.
    pumping: Option<f32>,
    pumped: f32,
}

fn at_pump(station: Res<Station>) -> bool {
    station.inside
}

/// Everything on the forecourt, which goes when the car leaves it.
#[derive(Component)]
struct OnForecourt;

/// The car at the pump, drawn from the driven car's `Looks`.
#[derive(Component)]
struct CarModel;

/// What shows the grade: a grade's button on the pump (lit when it's picked), its nozzle
/// in the holster on the car's side (gone when it's in the car), and the grip of the
/// nozzle in the car (the grade's colour).
#[derive(Component, Clone, Copy)]
enum Shows {
    Button(usize),
    Holster(usize),
    Grip,
}

/// Where the filler is, on the car's right (in the forecourt's frame, the car's body frame
/// at its origin, facing -z): over the rear wheel, or over the front one when the tank is
/// up front.
fn filler(setup: &CarSetup) -> Vec3 {
    let chassis = setup.build.chassis();
    let (body, wheels) = (&chassis.body, chassis.wheels());
    let front = setup.tank_at.z < (setup.front_z + setup.rear_z) / 2.0;
    let half = wheels.wheelbase / 2.0;
    let z = if front { -half } else { half - 0.05 };
    let x = z + body.front_overhang + half;
    let deck = body.deck_height(x);
    let radius = wheels.radius[usize::from(!front)];
    let y = (deck - 0.1).max(2.0 * radius + 0.1).min(deck - 0.03);
    Vec3::new(body.half_width(x), y, z)
}

/// The middle of the island beside the car, across and along (in the forecourt's frame).
fn island(setup: &CarSetup, filler: Vec3) -> Vec2 {
    let side = setup.hull.half.x.max(filler.x);
    Vec2::new(side + ISLAND_GAP + ISLAND.x / 2.0, filler.z - 0.8)
}

/// The gas station's window (before the pump): what's in the tank, and up to the pump.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut commands: Commands,
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut station: ResMut<Station>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) -> Result {
    if station.inside {
        return Ok(());
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let mut enter = false;
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Gas, |ui, _| {
        ui.label(tank_line(car));
        enter = ui
            .button(RichText::new("Up to the pump").strong())
            .clicked();
    });
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    let filler = filler(&car.setup);
    let island = island(&car.setup, filler);
    let origin = Vec3::new(marker.x, ground + ABOVE, marker.y);
    let pump_at = origin + Vec3::new(island.x, ISLAND.y, filler.z - PUMP_AHEAD);
    let display = pump_at + Vec3::new(PUMP.x / 2.0 + 0.008, DISPLAY_UP, 0.0);
    *station = Station {
        inside: true,
        origin,
        target: Vec3::new(island.x, 0.0, filler.z) + LOOK_AT,
        zoom: 1.1,
        display: [display, Vec3::NEG_Z, Vec3::Y],
        grade: station.grade,
        litres: if station.litres > 0.0 {
            station.litres
        } else {
            10.0
        },
        ..default()
    };
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (0.8, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    build_forecourt(&mut commands, &station, car, &mut meshes, &mut materials);
    Ok(())
}

/// What's in the tank, in a line.
fn tank_line(car: &Car) -> String {
    format!(
        "In the tank: {:.1} of {:.0} L, {:.1} octane",
        car.fuel.litres(),
        car.setup.tank.litres,
        car.fuel.octane()
    )
}

/// Away from the pump and back to the driven car, parked at the shop (filled already).
fn leave(
    commands: &mut Commands,
    station: &mut Station,
    things: &Query<Entity, With<OnForecourt>>,
    (away, audio): (&mut Away, &mut EngineAudio),
    cameras: &mut Query<(&ChaseCamera, &mut Projection)>,
) {
    for entity in things {
        commands.entity(entity).despawn();
    }
    for (camera, mut projection) in cameras.iter_mut() {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = camera.lens();
        }
    }
    (station.inside, station.pumping) = (false, None);
    away.0 = false;
    audio.hushed = false;
}

/// The car left the shop (or the server, and with it the shop) while at the pump: away.
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut station: ResMut<Station>,
    things: Query<Entity, With<OnForecourt>>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if station.inside && at.shop.is_none() {
        let parts = (&mut *away, &mut *audio);
        leave(&mut commands, &mut station, &things, parts, &mut cameras);
    }
}

/// Spawns the forecourt's boxes, cylinders and the like.
struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    origin: Vec3,
    cube: Handle<Mesh>,
}

impl Builder<'_, '_, '_> {
    /// A box of `size` with its middle at `at` (in the forecourt's frame).
    fn block(&mut self, at: Vec3, size: Vec3, material: &Handle<StandardMaterial>) -> Entity {
        let transform = Transform::from_translation(at).with_scale(size);
        self.put(&self.cube.clone(), material, transform)
    }

    /// `mesh` where `transform` (in the forecourt's frame) puts it.
    fn put(
        &mut self,
        mesh: &Handle<Mesh>,
        material: &Handle<StandardMaterial>,
        mut transform: Transform,
    ) -> Entity {
        transform.translation += self.origin;
        (self.commands)
            .spawn((
                OnForecourt,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                transform,
            ))
            .id()
    }
}

/// The materials of the forecourt.
struct Paints {
    pump: Handle<StandardMaterial>,
    accent: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    steel: Handle<StandardMaterial>,
    glass: Handle<StandardMaterial>,
    kerb: Handle<StandardMaterial>,
    hose: Handle<StandardMaterial>,
    /// Per grade: the grip, and its button unlit and lit.
    grips: [Handle<StandardMaterial>; 3],
    buttons: [[Handle<StandardMaterial>; 2]; 3],
}

/// The forecourt: concrete under the canopy, asphalt round it and a meadow with trees out
/// to the haze; three pump islands under the canopy with the car in the middle lane at the
/// right one's pump, the nozzle in its filler; the shop behind them, and lights.
fn build_forecourt(
    commands: &mut Commands,
    station: &Station,
    car: &Car,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mut material = |color: Color, roughness: f32, metallic: f32| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            metallic,
            ..default()
        })
    };
    let mut lit = Vec::new();
    let rgb = |[r, g, b]: [f32; 3]| Color::srgb(r, g, b);
    let concrete = material(Color::srgb(0.62, 0.62, 0.60), 0.85, 0.0);
    let asphalt = material(Color::srgb(0.17, 0.17, 0.18), 0.9, 0.0);
    let grass = material(Color::srgb(0.25, 0.38, 0.14), 1.0, 0.0);
    let white = material(Color::srgb(0.90, 0.91, 0.92), 0.6, 0.0);
    let line = material(Color::srgb(0.92, 0.92, 0.88), 0.8, 0.0);
    let wall = material(Color::srgb(0.84, 0.82, 0.78), 0.9, 0.0);
    let bark = material(Color::srgb(0.30, 0.22, 0.15), 1.0, 0.0);
    let leaves = material(Color::srgb(0.16, 0.30, 0.10), 1.0, 0.0);
    let flap = material(
        Color::srgb_from_array(PAINTS[car.setup.build.paint].1),
        0.3,
        0.4,
    );
    let mut paints = Paints {
        pump: material(Color::srgb(0.88, 0.89, 0.90), 0.4, 0.1),
        accent: material(Color::srgb(0.02, 0.42, 0.46), 0.5, 0.1),
        dark: material(Color::srgb(0.07, 0.07, 0.08), 0.5, 0.3),
        steel: material(Color::srgb(0.72, 0.73, 0.75), 0.3, 0.9),
        glass: material(Color::srgb(0.03, 0.04, 0.05), 0.15, 0.0),
        kerb: material(Color::srgb(0.95, 0.75, 0.10), 0.6, 0.0),
        hose: material(Color::srgb(0.05, 0.05, 0.05), 0.6, 0.0),
        grips: GRADES.map(|(_, _, color)| material(rgb(color), 0.5, 0.0)),
        buttons: GRADES.map(|(_, _, color)| {
            lit.push(color);
            [
                material(rgb(color.map(|c| c * 0.3)), 0.5, 0.0),
                Handle::default(),
            ]
        }),
    };
    for (buttons, color) in paints.buttons.iter_mut().zip(lit) {
        let glow = LinearRgba::from(rgb(color)) * 6.0;
        buttons[1] = materials.add(StandardMaterial {
            base_color: Color::from(glow),
            emissive: glow,
            unlit: true,
            ..default()
        });
    }
    commands.insert_resource(GradePaints {
        grips: paints.grips.clone(),
        buttons: paints.buttons.clone(),
    });
    let glow = |materials: &mut Assets<StandardMaterial>, color: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: Color::from(color),
            emissive: color,
            unlit: true,
            ..default()
        })
    };
    let lamp = glow(materials, LinearRgba::rgb(6.0, 6.0, 5.6));
    let window = glow(materials, LinearRgba::rgb(1.4, 1.25, 0.9));
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let mut b = Builder {
        commands,
        origin: station.origin,
        cube,
    };

    let filler = filler(&car.setup);
    let island = island(&car.setup, filler);
    // Islands across: the car's on its left, one lane further each way.
    let islands = [island.x - 2.0 * LANE, island.x - LANE, island.x];
    let (left, right) = (islands[0] - 5.0, island.x + 5.0);
    let (front, back) = (island.y - 8.0, island.y + 8.0);
    let middle = Vec3::new((left + right) / 2.0, 0.0, (front + back) / 2.0);

    // The ground: a meadow out to the haze, asphalt round the station, concrete under the
    // canopy.
    b.block(
        Vec3::new(0.0, -0.06, 0.0),
        Vec3::new(2400.0, 0.1, 2400.0),
        &grass,
    );
    b.block(
        middle.with_z(island.y - 6.0) + Vec3::Y * -0.02,
        Vec3::new(right - left + 40.0, 0.04, 70.0),
        &asphalt,
    );
    b.block(
        middle + Vec3::Y * -0.01,
        Vec3::new(right - left, 0.04, back - front),
        &concrete,
    );
    // Lines along the asphalt's road side, behind.
    for k in 0..12 {
        let at = Vec3::new(left - 18.0 + 6.0 * k as f32, 0.002, island.y + 26.0);
        b.block(at, Vec3::new(3.0, 0.004, 0.15), &line);
    }

    // The islands, each with a pump (the car's has the hose) and two posts for the canopy.
    for (k, &x) in islands.iter().enumerate() {
        let at = Vec3::new(x, ISLAND.y / 2.0, island.y);
        b.block(at, ISLAND, &paints.kerb);
        let top = Vec3::new(ISLAND.x - 0.12, 0.02, ISLAND.z - 0.12);
        b.block(at.with_y(ISLAND.y + 0.01), top, &concrete);
        let ours = k == islands.len() - 1;
        pump(
            &mut b,
            &paints,
            Vec3::new(x, ISLAND.y, filler.z - PUMP_AHEAD),
            ours,
        );
        for end in [-1.0, 1.0] {
            let z = island.y + end * (ISLAND.z / 2.0 - 0.45);
            let at = Vec3::new(x, (ISLAND.y + CANOPY) / 2.0, z);
            b.block(at, Vec3::new(0.32, CANOPY - ISLAND.y, 0.32), &white);
            b.block(
                Vec3::new(x, ISLAND.y + 0.5, z),
                Vec3::new(0.36, 1.0, 0.36),
                &paints.accent,
            );
        }
    }

    // The canopy: a slab with a band in the station's colour round its edge, lights under
    // it.
    let (span, depth) = (right - left, back - front);
    b.block(
        middle.with_y(CANOPY + 0.35),
        Vec3::new(span, 0.7, depth),
        &white,
    );
    for s in [-1.0, 1.0] {
        let at = middle.with_y(CANOPY + 0.4) + Vec3::Z * s * (depth / 2.0 + 0.02);
        b.block(at, Vec3::new(span + 0.08, 0.5, 0.04), &paints.accent);
        let at = middle.with_y(CANOPY + 0.4) + Vec3::X * s * (span / 2.0 + 0.02);
        b.block(at, Vec3::new(0.04, 0.5, depth + 0.08), &paints.accent);
    }
    for &x in &islands {
        for side in [-1.0, 1.0] {
            for z in [-4.5, 0.0, 4.5] {
                let at = Vec3::new(x + side * LANE / 2.0, CANOPY - 0.02, island.y + z);
                b.block(at, Vec3::new(0.5, 0.04, 1.2), &lamp);
            }
        }
        b.commands.spawn((
            OnForecourt,
            PointLight {
                intensity: 600_000.0,
                range: 18.0,
                ..default()
            },
            Transform::from_translation(station.origin + Vec3::new(x, CANOPY - 0.6, island.y)),
        ));
    }

    // The shop ahead: walls, a lit glass front facing the pumps, a band in the station's
    // colour along its top.
    let shop = Vec3::new(middle.x - 3.0, 0.0, front - 9.0);
    let (shop_w, shop_d, shop_h) = (18.0, 9.0, 4.2);
    b.block(
        shop + Vec3::Y * shop_h / 2.0,
        Vec3::new(shop_w, shop_h, shop_d),
        &wall,
    );
    b.block(
        shop + Vec3::new(0.0, shop_h + 0.25, 0.0),
        Vec3::new(shop_w + 0.3, 0.5, shop_d + 0.3),
        &paints.accent,
    );
    let face = shop.z + shop_d / 2.0 + 0.02;
    b.block(
        Vec3::new(shop.x - 1.5, 1.5, face),
        Vec3::new(shop_w - 6.0, 2.6, 0.04),
        &window,
    );
    for k in 0..6 {
        let x = shop.x - 1.5 - (shop_w - 6.0) / 2.0 + (shop_w - 6.0) / 5.0 * k as f32;
        b.block(
            Vec3::new(x, 1.5, face + 0.02),
            Vec3::new(0.08, 2.6, 0.04),
            &paints.dark,
        );
    }
    b.block(
        Vec3::new(shop.x + shop_w / 2.0 - 2.0, 1.15, face),
        Vec3::new(1.6, 2.3, 0.05),
        &paints.glass,
    );

    // Trees round the meadow.
    let trunk = meshes.add(Cylinder::new(0.2, 3.0));
    let crown = meshes.add(Sphere::new(2.4));
    let trees = [
        (-34.0, -30.0, 1.0),
        (-40.0, -12.0, 0.8),
        (-38.0, 8.0, 1.2),
        (-30.0, 30.0, 0.9),
        (30.0, -26.0, 1.1),
        (36.0, -6.0, 0.9),
        (42.0, 14.0, 1.3),
        (26.0, 34.0, 1.0),
        (-8.0, -38.0, 1.2),
        (12.0, -40.0, 0.9),
        (-12.0, 48.0, 1.1),
        (8.0, 52.0, 0.8),
    ];
    for (x, z, size) in trees {
        let at = middle + Vec3::new(x, 0.0, z);
        let trunk_at =
            Transform::from_translation(at + Vec3::Y * 1.5 * size).with_scale(Vec3::splat(size));
        b.put(&trunk, &bark, trunk_at);
        let crown_at = Transform::from_translation(at + Vec3::Y * 4.6 * size)
            .with_scale(Vec3::new(1.0, 1.25, 1.0) * size);
        b.put(&crown, &leaves, crown_at);
    }

    // The car at the pump, the flap open, the nozzle in and the hose from the pump.
    let (looks, motion) = looks_at_pump(car);
    let model = Transform::from_translation(station.origin - looks.body_offset);
    b.commands
        .spawn((OnForecourt, CarModel, looks, motion, model));
    let hole = meshes.add(Cylinder::new(0.055, 0.01));
    let side = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    b.put(
        &hole,
        &paints.dark,
        Transform::from_translation(filler + Vec3::X * 0.004).with_rotation(side),
    );
    // The flap, hinged at its front edge and swung open.
    let hinge = filler + Vec3::new(0.0, 0.0, -0.085);
    let swung = Quat::from_rotation_y(1.9);
    b.put(
        &b.cube.clone(),
        &flap,
        Transform::from_translation(hinge + swung * Vec3::new(0.0, 0.0, 0.085) + Vec3::X * 0.01)
            .with_rotation(swung)
            .with_scale(Vec3::new(0.008, 0.16, 0.16)),
    );
    // The nozzle: its spout into the filler, the grip angled up and out, a guard under it.
    let tilt = Quat::from_rotation_z(0.45);
    let grip_at = filler + tilt * Vec3::new(0.14, 0.0, 0.0);
    b.put(
        &b.cube.clone(),
        &paints.steel,
        Transform::from_translation(filler + tilt * Vec3::new(0.02, 0.0, 0.0))
            .with_rotation(tilt)
            .with_scale(Vec3::new(0.1, 0.022, 0.022)),
    );
    let grip = b.put(
        &b.cube.clone(),
        &paints.grips[station.grade],
        Transform::from_translation(grip_at)
            .with_rotation(tilt)
            .with_scale(Vec3::new(0.2, 0.055, 0.06)),
    );
    b.commands.entity(grip).insert(Shows::Grip);
    b.put(
        &b.cube.clone(),
        &paints.dark,
        Transform::from_translation(grip_at + tilt * Vec3::new(0.0, -0.06, 0.0))
            .with_rotation(tilt)
            .with_scale(Vec3::new(0.14, 0.012, 0.04)),
    );
    // The hose, hanging between the pump's outlet and the nozzle's back end.
    let pump_at = Vec3::new(island.x, ISLAND.y, filler.z - PUMP_AHEAD);
    let outlet = pump_at + Vec3::new(-PUMP.x / 2.0 - 0.03, PUMP.y - 0.35, PUMP.z / 2.0 - 0.12);
    let end = filler + tilt * Vec3::new(0.25, 0.0, 0.0);
    let low = 0.4;
    let sag = Vec3::new(
        (outlet.x + end.x) / 2.0,
        2.0 * low - (outlet.y + end.y) / 2.0,
        (outlet.z + end.z) / 2.0,
    );
    let hose = meshes.add(Cylinder::new(0.018, 1.0));
    let point = |t: f32| outlet.lerp(sag, t).lerp(sag.lerp(end, t), t);
    const PIECES: usize = 28;
    for k in 0..PIECES {
        let (a, c) = (
            point(k as f32 / PIECES as f32),
            point((k + 1) as f32 / PIECES as f32),
        );
        let length = a.distance(c) + 0.01;
        let turn = Quat::from_rotation_arc(Vec3::Y, (c - a).normalize_or(Vec3::Y));
        b.put(
            &hose,
            &paints.hose,
            Transform::from_translation((a + c) / 2.0)
                .with_rotation(turn)
                .with_scale(Vec3::new(1.0, length, 1.0)),
        );
    }
}

/// A pump on an island, at `at` (on the island's top, in the forecourt's frame): a column
/// with the station's colour on top, the display on its outer face with a button per grade
/// under it, and a nozzle per grade in a holster on each side. On `ours`, the buttons and
/// the car's side's holsters show the grade.
fn pump(b: &mut Builder, paints: &Paints, at: Vec3, ours: bool) {
    b.block(
        at + Vec3::Y * 0.06,
        Vec3::new(PUMP.x + 0.1, 0.12, PUMP.z + 0.1),
        &paints.dark,
    );
    b.block(
        at + Vec3::Y * (PUMP.y - 0.3) / 2.0,
        Vec3::new(PUMP.x, PUMP.y - 0.3, PUMP.z),
        &paints.pump,
    );
    b.block(
        at + Vec3::Y * (PUMP.y - 0.15),
        Vec3::new(PUMP.x + 0.04, 0.3, PUMP.z + 0.04),
        &paints.accent,
    );
    for side in [-1.0, 1.0] {
        let face = at.x + side * PUMP.x / 2.0;
        // The display's glass (painted over on our pump's outer side).
        b.block(
            Vec3::new(face + side * 0.004, at.y + DISPLAY_UP, at.z),
            Vec3::new(0.008, DISPLAY.y + 0.04, DISPLAY.x + 0.04),
            &paints.glass,
        );
        for grade in 0..GRADES.len() {
            let along = (grade as f32 - 1.0) * 0.2 * -side;
            let button = b.block(
                Vec3::new(face + side * 0.008, at.y + 1.12, at.z + along),
                Vec3::new(0.016, 0.06, 0.14),
                &paints.buttons[grade][0],
            );
            // A holster, with the nozzle's grip over it.
            let z = at.z + along * 1.5;
            b.block(
                Vec3::new(face + side * 0.03, at.y + 0.82, z),
                Vec3::new(0.06, 0.16, 0.09),
                &paints.dark,
            );
            let grip = b.block(
                Vec3::new(face + side * 0.07, at.y + 0.95, z),
                Vec3::new(0.06, 0.16, 0.05),
                &paints.grips[grade],
            );
            if ours {
                b.commands.entity(button).insert(Shows::Button(grade));
                if side < 0.0 {
                    b.commands.entity(grip).insert(Shows::Holster(grade));
                }
            }
        }
    }
}

/// The car at the pump: its looks and its suspension as the driven car's, the front wheels
/// straight, nothing turning, the engine off.
fn looks_at_pump(car: &Car) -> (Looks, Motion) {
    let mut motion = Motion::of(car);
    for wheel in &mut motion.wheels {
        (wheel.steer, wheel.spin) = (0.0, 0.0);
    }
    motion.engine = EngineRun::default();
    (Looks::of(&car.setup), motion)
}

/// Pumps into the driven car while it's on, until this fill's litres are in or the tank
/// is full.
fn pump_fuel(station: &mut Station, car: &mut Car, dt: f32) {
    let Some(goal) = station.pumping else {
        return;
    };
    let tank = car.setup.tank;
    let room = (tank.litres - car.fuel.litres()).max(0.0);
    let step = (FLOW * dt).min(goal - station.pumped).min(room).max(0.0);
    car.fuel.fill(&tank, step, GRADES[station.grade].1);
    station.pumped += step;
    if station.pumped >= goal - 1e-4 || room - step < 1e-3 {
        station.pumping = None;
    }
}

fn fill(
    time: Res<Time>,
    mut station: ResMut<Station>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
) {
    if let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) {
        pump_fuel(&mut station, &mut car, time.delta_secs());
    }
}

/// The materials that show the grade: per grade, the nozzle's grip, and its button unlit
/// and lit.
#[derive(Resource)]
struct GradePaints {
    grips: [Handle<StandardMaterial>; 3],
    buttons: [[Handle<StandardMaterial>; 2]; 3],
}

/// The pump shows the grade: its button lit, its nozzle out of the holster and in the car.
fn dress(
    station: Res<Station>,
    paints: Option<Res<GradePaints>>,
    mut things: Query<(
        &Shows,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    let Some(paints) = paints else {
        return;
    };
    let grade = station.grade;
    for (shows, mut material, mut visibility) in &mut things {
        match *shows {
            Shows::Button(k) => {
                let handle = &paints.buttons[k][usize::from(k == grade)];
                if material.0 != *handle {
                    material.0 = handle.clone();
                }
            }
            Shows::Holster(k) => {
                visibility.set_if_neq(if k == grade {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                });
            }
            Shows::Grip => {
                if material.0 != paints.grips[grade] {
                    material.0 = paints.grips[grade].clone();
                }
            }
        }
    }
}

/// The camera over the island, looking at the car's filler past the pump; dragging swings
/// it round, the mouse wheel takes it nearer or farther.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut station: ResMut<Station>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        let swing = station.swing - motion.delta.x * 0.005;
        station.swing = swing.clamp(-VIEW_SWING, VIEW_SWING);
    }
    let zoom = station.zoom * (1.0 - 0.1 * scroll.delta.y.clamp(-3.0, 3.0));
    station.zoom = zoom.clamp(*ZOOM.start(), *ZOOM.end());
    let target = station.origin + station.target;
    let eye = target + Quat::from_rotation_y(station.swing) * EYE * station.zoom;
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = station.eye.map_or(eye, |was| was.lerp(eye, ease));
    station.eye = Some(now);
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(target, Vec3::Y);
    }
}

/// The pump's display, painted where it is on the pump's outer face: the grade, the litres
/// of this fill, and the tank.
fn display(
    mut contexts: EguiContexts,
    station: Res<Station>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    let Ok((camera, eye)) = cameras.single() else {
        return Ok(());
    };
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let [middle, right, up] = station.display;
    // Only from in front (from behind, the pump hides it).
    let facing = right.cross(up);
    if (eye.translation() - middle).normalize().dot(facing) < 0.2 {
        return Ok(());
    }
    // A point on the display, from its top left (0, 0) to its bottom right (1, 1), in the
    // window.
    let ctx = contexts.ctx_mut()?;
    let project = |u: f32, v: f32| {
        let at = middle + right * (u - 0.5) * DISPLAY.x - up * (v - 0.5) * DISPLAY.y;
        camera
            .world_to_viewport(eye, at)
            .ok()
            .map(|p| crate::menu::to_points(ctx, p))
    };
    let Some(corners) = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .into_iter()
        .map(|(u, v)| project(u, v))
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(());
    };
    let painter = ctx.layer_painter(egui::LayerId::background());
    painter.add(egui::Shape::convex_polygon(
        corners,
        LCD,
        Stroke::new(1.0, Color32::from_rgb(40, 50, 45)),
    ));
    // Text at a point on the display, as high as `size` of the display's height there.
    let text = |u: f32, v: f32, size: f32, align: Align2, words: &str, color: Color32| {
        let (Some(at), Some(below)) = (project(u, v), project(u, v + size)) else {
            return;
        };
        let height = at.distance(below);
        painter.text(at, align, words, FontId::monospace(height), color);
    };
    let dim = LCD_INK.gamma_multiply(0.6);
    let (name, octane, _) = GRADES[station.grade];
    text(
        0.05,
        0.06,
        0.13,
        Align2::LEFT_TOP,
        &name.to_uppercase(),
        LCD_INK,
    );
    text(
        0.95,
        0.06,
        0.13,
        Align2::RIGHT_TOP,
        &format!("RON {octane:.0}"),
        dim,
    );
    text(
        0.80,
        0.24,
        0.36,
        Align2::RIGHT_TOP,
        &format!("{:.2}", station.pumped),
        LCD_INK,
    );
    text(0.82, 0.42, 0.12, Align2::LEFT_TOP, "L", dim);
    let (litres, tank) = (car.fuel.litres(), car.setup.tank.litres);
    let state = if station.pumping.is_some() {
        "PUMPING"
    } else if tank - litres < 0.05 {
        "FULL"
    } else if station.pumped > 0.0 {
        "DONE"
    } else {
        "READY"
    };
    text(0.05, 0.70, 0.12, Align2::LEFT_TOP, state, LCD_INK);
    text(
        0.95,
        0.70,
        0.11,
        Align2::RIGHT_TOP,
        &format!("{litres:.1}/{tank:.0} L"),
        dim,
    );
    // The tank as a bar along the bottom.
    let share = (litres / tank).clamp(0.0, 1.0);
    let bar = [(0.05, 0.86), (0.95, 0.86), (0.95, 0.93), (0.05, 0.93)];
    if let Some(outline) = bar
        .iter()
        .map(|&(u, v)| project(u, v))
        .collect::<Option<Vec<_>>>()
    {
        painter.add(egui::Shape::convex_polygon(
            outline,
            Color32::TRANSPARENT,
            Stroke::new(1.0, dim),
        ));
    }
    let filled = [
        (0.05, 0.86),
        (0.05 + 0.9 * share, 0.86),
        (0.05 + 0.9 * share, 0.93),
        (0.05, 0.93),
    ];
    if share > 0.0
        && let Some(fill) = filled
            .iter()
            .map(|&(u, v)| project(u, v))
            .collect::<Option<Vec<_>>>()
    {
        painter.add(egui::Shape::convex_polygon(fill, LCD_INK, Stroke::NONE));
    }
    Ok(())
}

/// The pump's window, on the right: the grade and how much, pumping and stopping; and back
/// to the car.
#[allow(clippy::too_many_arguments)]
fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut station: ResMut<Station>,
    things: Query<Entity, With<OnForecourt>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) -> Result {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let (litres, tank) = (car.fuel.litres(), car.setup.tank.litres);
    let octane = car.fuel.octane();
    let room = (tank - litres).max(0.0);
    let mut out = false;
    egui::Window::new("Pump")
        .anchor(Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
        .collapsible(false)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            ui.label(tank_line(car)).on_hover_text(
                "Grades mix by volume. Higher octane resists knock: the factory tunes are \
                 for 95; more spark advance or boost may want more, and lower knocks.",
            );
            let pumping = station.pumping.is_some();
            ui.add_enabled_ui(!pumping, |ui| {
                ui.horizontal(|ui| {
                    for (i, (name, ..)) in GRADES.iter().enumerate() {
                        ui.selectable_value(&mut station.grade, i, *name);
                    }
                });
                ui.add(egui::Slider::new(&mut station.litres, 1.0..=tank.max(1.0)).suffix(" L"));
            });
            let grade = GRADES[station.grade].1;
            let mixed = |added: f32| {
                let added = added.min(room);
                (litres * octane + added * grade) / (litres + added).max(1e-6)
            };
            ui.horizontal(|ui| {
                if pumping {
                    if ui.button(RichText::new("Stop").strong()).clicked() {
                        station.pumping = None;
                    }
                    return;
                }
                let some = station.litres.min(room);
                let start = |station: &mut Station, goal: f32| {
                    (station.pumping, station.pumped) = (Some(goal), 0.0);
                };
                if ui
                    .add_enabled(some > 0.05, egui::Button::new(format!("Pump {some:.1} L")))
                    .on_hover_text(format!("Then {:.1} octane", mixed(some)))
                    .clicked()
                {
                    start(&mut station, some);
                }
                if ui
                    .add_enabled(
                        room > 0.05,
                        egui::Button::new(format!("Fill up ({room:.1} L)")),
                    )
                    .on_hover_text(format!("Then {:.1} octane", mixed(room)))
                    .clicked()
                {
                    start(&mut station, room);
                }
            });
            ui.label(
                RichText::new("Drag to look round, the mouse wheel to go nearer.")
                    .small()
                    .weak(),
            );
            ui.separator();
            out = ui
                .button(RichText::new("Back to the car").strong())
                .clicked();
        });
    if out {
        leave(
            &mut commands,
            &mut station,
            &things,
            (&mut away, &mut audio),
            &mut cameras,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::fuel::Fuel;

    #[test]
    fn the_pump_fills_at_its_pace_and_stops() {
        let terrain = Terrain::flat();
        let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, 0.0, 0.0, 0.0);
        let tank = car.setup.tank.litres;
        car.fuel = Fuel::new(10.0);
        let mut station = Station {
            grade: 2,
            pumping: Some(5.0),
            ..default()
        };
        pump_fuel(&mut station, &mut car, 1.0);
        assert!((car.fuel.litres() - (10.0 + FLOW)).abs() < 1e-4);
        // On to the litres asked for, and no further.
        for _ in 0..10 {
            pump_fuel(&mut station, &mut car, 1.0);
        }
        assert_eq!(station.pumping, None);
        assert!((car.fuel.litres() - 15.0).abs() < 1e-4);
        let octane = (10.0 * 95.0 + 5.0 * 102.0) / 15.0;
        assert!((car.fuel.octane() - octane).abs() < 1e-3);
        // Filling up stops when it's full.
        station.pumping = Some(100.0);
        station.pumped = 0.0;
        for _ in 0..100 {
            pump_fuel(&mut station, &mut car, 1.0);
        }
        assert_eq!(station.pumping, None);
        assert!((car.fuel.litres() - tank).abs() < 1e-3);
    }

    #[test]
    fn every_filler_is_on_the_body_over_a_wheel() {
        for chassis in 0..crate::chassis::CHASSIS.len() {
            let setup = CarSetup::factory(chassis, 0);
            let at = filler(&setup);
            let wheels = setup.build.chassis().wheels();
            assert!(
                at.z.abs() <= wheels.wheelbase / 2.0 + 0.01,
                "{}",
                setup.name
            );
            assert!(at.y > 0.4 && at.y < 1.4, "{}: {at}", setup.name);
            assert!(at.x > 0.4 && at.x < 1.1, "{}: {at}", setup.name);
        }
    }
}
