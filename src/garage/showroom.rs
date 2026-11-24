//! The car dealers' showroom: a hall with glass walls on a forecourt high over the shop, a
//! polished floor and lights in the ceiling. Every car stands on the floor in its factory
//! paint, each in a place of its own; the one picked in the window turns slowly on a
//! turntable at the front under spotlights (its place on the floor stays empty), and "Start
//! it" idles its engine and blips it now and then. The driven car waits outside the glass.
//! The window on the right is the brochure (`brochure`); "Drive it" builds the car picked
//! and goes straight out to it.
//!
//! The driven car stays parked on the shop's marker meanwhile (a ghost, its input held like
//! in the dyno's room); the one outside is drawn from its looks, the engine off.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Stroke};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::{Garage, Showroom, brochure, start};
use crate::audio::{EngineAudio, EngineRun};
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car, CarSetup};
use crate::chassis::{CHASSIS, CarBuild};
use crate::looks::{Looks, Motion};
use crate::menu::kit::{self, Panel, Place};
use crate::shops::{AtShop, SceneBack, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;
use crate::traffic::FactoryLooks;

/// How far over the ground at the shop the showroom is (m).
const ABOVE: f32 = 600.0;
/// The hall: from its left to its right wall (x), from its glass front to its back wall
/// (z), and its height (m).
const HALL_X: f32 = 17.0;
const FRONT: f32 = -12.0;
const BACK: f32 = 11.0;
const HEIGHT: f32 = 5.5;
/// The turntable: its middle, radius and height (m), and how fast it turns (rad/s).
const TABLE: Vec3 = Vec3::new(0.0, 0.0, -4.0);
const TABLE_RADIUS: f32 = 3.2;
const TABLE_HEIGHT: f32 = 0.12;
const SPIN: f32 = 0.22;
/// Where the driven car waits outside, and which way it faces (rad about up; 0 faces the
/// hall's front, -z).
const OUTSIDE: Vec3 = Vec3::new(-6.0, 0.0, -17.5);
const OUTSIDE_YAW: f32 = -FRAC_PI_2 + 0.25;
/// The sign by the turntable (the car's name painted on it): its board's middle, size and
/// which way it faces (rad about up; 0 faces the back, +z) and how far it leans back (rad).
const SIGN: Vec3 = Vec3::new(-2.7, 0.95, -1.7);
const SIGN_YAW: f32 = -0.1;
const SIGN_SIZE: Vec2 = Vec2::new(0.7, 0.42);
const SIGN_LEAN: f32 = 0.45;
/// The camera: what it looks at, from where, how near and far it may zoom (a share of its
/// distance), how far right of the car it looks (with the window up on the right), and
/// how far from the turntable's middle it may go (m; clear of the cars on the floor, the
/// desk and the glass).
const TARGET: Vec3 = Vec3::new(0.0, 0.8, TABLE.z);
const EYE: Vec3 = Vec3::new(-3.4, 1.7, 2.3);
const ZOOM: std::ops::RangeInclusive<f32> = 0.4..=1.0;
const ASIDE: f32 = 1.2;
const REACH: f32 = 7.4;
/// "Start it": a blip of the throttle every so often (s), held this long (s), and up to
/// this share of the rev limit.
const BLIP_EVERY: f32 = 3.5;
const BLIP_HELD: f32 = 0.35;
const BLIP_TO: f32 = 0.6;

pub struct ShowroomPlugin;

impl Plugin for ShowroomPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hall>()
            .add_systems(
                Update,
                (
                    leave_when_gone,
                    (show, run_engine, look.after(crate::camera::follow)).run_if(in_hall),
                )
                    .chain(),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (shop, (sign, window).run_if(in_hall)),
            );
    }
}

/// The showroom, while the car is in it: where it is, the camera's swing and zoom, the
/// turntable's angle, what's on it, and its engine when started.
#[derive(Resource, Default)]
struct Hall {
    inside: bool,
    origin: Vec3,
    swing: f32,
    zoom: f32,
    eye: Option<Vec3>,
    aside: f32,
    turn: f32,
    /// The chassis and engine option on the turntable, its idle speed and rev limit.
    on_table: Option<(usize, usize)>,
    idle: f32,
    rev_limit: f32,
    /// The engine on the turntable, when started: its speed, and time since the last blip.
    running: bool,
    rpm: f32,
    clock: f32,
    /// A car bought, its engine's maps still baking: out of the showroom once it's built.
    buying: bool,
}

fn in_hall(hall: Res<Hall>) -> bool {
    hall.inside
}

/// Everything in the showroom, which goes when the car leaves it.
#[derive(Component)]
struct InHall;

/// A car shown: on the turntable, on the floor in its place (a chassis in `CHASSIS`,
/// hidden while it's on the turntable), or the driven car outside.
#[derive(Component, Clone, Copy, PartialEq)]
enum Shown {
    Table,
    Floor(usize),
    Outside,
}

/// Where a chassis (in `CHASSIS`) stands on the floor, and which way it faces (rad about
/// up; 0 faces the glass front): three along the left wall facing in, seven along the back
/// at an angle, three along the right wall facing in.
fn place(chassis: usize) -> (Vec3, f32) {
    match chassis {
        0..3 => (
            Vec3::new(-13.5, 0.0, -8.0 + 5.0 * chassis as f32),
            -FRAC_PI_2 + 0.35,
        ),
        3..10 => (Vec3::new(-13.8 + 4.6 * (chassis - 3) as f32, 0.0, 7.5), 0.5),
        _ => (
            Vec3::new(13.5, 0.0, -8.0 + 5.0 * (chassis - 10) as f32),
            FRAC_PI_2 - 0.35,
        ),
    }
}

/// The transform of a car of `looks` (its centre of mass, as `Looks` are drawn from) with
/// its body's design frame (on the ground midway between the axles) at `at` in the hall's
/// frame, facing `yaw`.
fn placed(origin: Vec3, at: Vec3, yaw: f32, looks: &Looks) -> Transform {
    let turn = Quat::from_rotation_y(yaw);
    Transform::from_translation(origin + at - turn * looks.body_offset).with_rotation(turn)
}

/// A car standing still: its wheels at their design height, straight and not turning, the
/// engine off.
fn at_rest(looks: &Looks) -> Motion {
    let mut motion = Motion::default();
    for (corner, wheel) in motion.wheels.iter_mut().enumerate() {
        wheel.theta = looks.axles[corner / 2].geometry.design_angle();
    }
    motion
}

/// The showroom's window (before going in): into the showroom.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut commands: Commands,
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    (mut hall, mut garage): (ResMut<Hall>, ResMut<Garage>),
    mut factory: ResMut<FactoryLooks>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) -> Result {
    if hall.inside {
        return Ok(());
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let mut enter = false;
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Showroom, |ui, _| {
        enter = kit::button(ui, "Into the showroom").clicked();
        kit::note(
            ui,
            "Every car as it leaves the factory, to look round and drive away in.",
        );
    });
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    *hall = Hall {
        inside: true,
        origin: Vec3::new(marker.x, ground + ABOVE, marker.y),
        turn: 0.6,
        zoom: 1.0,
        ..default()
    };
    // Start at the car being driven.
    garage.showroom = Showroom {
        chassis: car.setup.build.chassis,
        engine: 0,
    };
    garage.status.clear();
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (0.8, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    build_hall(
        &mut commands,
        &hall,
        car,
        &mut factory,
        &mut meshes,
        &mut materials,
    );
    Ok(())
}

/// Out of the showroom and back to the driven car, parked at the shop.
fn leave(
    commands: &mut Commands,
    hall: &mut Hall,
    things: &Query<Entity, With<InHall>>,
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
    *hall = Hall::default();
    away.0 = false;
    audio.hushed = false;
}

/// The car left the shop (or the server, and with it the shop) while in the showroom: out.
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut hall: ResMut<Hall>,
    things: Query<Entity, With<InHall>>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if hall.inside && at.shop.is_none() {
        let parts = (&mut *away, &mut *audio);
        leave(&mut commands, &mut hall, &things, parts, &mut cameras);
    }
}

/// Spawns the showroom's boxes, cylinders and the like.
struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    origin: Vec3,
    cube: Handle<Mesh>,
}

impl Builder<'_, '_, '_> {
    /// A box of `size` with its middle at `at` (in the hall's frame).
    fn block(&mut self, at: Vec3, size: Vec3, material: &Handle<StandardMaterial>) -> Entity {
        let transform = Transform::from_translation(at).with_scale(size);
        self.put(&self.cube.clone(), material, transform)
    }

    /// `mesh` where `transform` (in the hall's frame) puts it.
    fn put(
        &mut self,
        mesh: &Handle<Mesh>,
        material: &Handle<StandardMaterial>,
        mut transform: Transform,
    ) -> Entity {
        transform.translation += self.origin;
        (self.commands)
            .spawn((
                InHall,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                transform,
            ))
            .id()
    }
}

/// The showroom: a meadow out to the haze with trees, the forecourt with flags, the
/// dealer's sign and the driven car; the hall with its glass front and left side, the
/// floor, roof and lights; the cars on the floor with a stand each, the turntable, the
/// sales desk and plants.
fn build_hall(
    commands: &mut Commands,
    hall: &Hall,
    car: &Car,
    factory: &mut FactoryLooks,
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
    let grass = material(Color::srgb(0.25, 0.38, 0.14), 1.0, 0.0);
    let asphalt = material(Color::srgb(0.17, 0.17, 0.18), 0.9, 0.0);
    let line = material(Color::srgb(0.92, 0.92, 0.88), 0.8, 0.0);
    let floor = material(Color::srgb(0.80, 0.80, 0.79), 0.12, 0.0);
    let joint = material(Color::srgb(0.62, 0.62, 0.62), 0.4, 0.0);
    let wall = material(Color::srgb(0.93, 0.93, 0.92), 0.8, 0.0);
    let ceiling = material(Color::srgb(0.88, 0.88, 0.88), 0.9, 0.0);
    let accent = material(Color::srgb(0.06, 0.18, 0.42), 0.5, 0.1);
    let steel = material(Color::srgb(0.72, 0.73, 0.75), 0.3, 0.9);
    let dark = material(Color::srgb(0.08, 0.08, 0.09), 0.4, 0.2);
    let table = material(Color::srgb(0.16, 0.16, 0.18), 0.15, 0.3);
    let wood = material(Color::srgb(0.55, 0.40, 0.26), 0.6, 0.0);
    let white = material(Color::srgb(0.95, 0.95, 0.95), 0.5, 0.0);
    let pot = material(Color::srgb(0.25, 0.25, 0.26), 0.6, 0.0);
    let bark = material(Color::srgb(0.30, 0.22, 0.15), 1.0, 0.0);
    let leaves = material(Color::srgb(0.16, 0.30, 0.10), 1.0, 0.0);
    let plant = material(Color::srgb(0.12, 0.35, 0.12), 0.9, 0.0);
    let flags = [
        material(Color::srgb(0.06, 0.18, 0.42), 0.8, 0.0),
        material(Color::srgb(0.95, 0.95, 0.95), 0.8, 0.0),
        material(Color::srgb(0.85, 0.65, 0.10), 0.8, 0.0),
    ];
    let glass = materials.add(StandardMaterial {
        base_color: Color::srgba(0.55, 0.70, 0.75, 0.12),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.05,
        ..default()
    });
    let glow = |materials: &mut Assets<StandardMaterial>, color: LinearRgba| {
        materials.add(StandardMaterial {
            base_color: Color::from(color),
            emissive: color,
            unlit: true,
            ..default()
        })
    };
    let lamp = glow(materials, LinearRgba::rgb(6.0, 6.0, 5.8));
    let screen = glow(materials, LinearRgba::rgb(0.5, 0.9, 1.6));
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let cylinder = meshes.add(Cylinder::new(1.0, 1.0));
    let ball = meshes.add(Sphere::new(1.0));
    let origin = hall.origin;
    let mut b = Builder {
        commands,
        origin,
        cube,
    };

    // The ground: a meadow out to the haze, the forecourt in front of the hall with
    // parking lines.
    b.block(
        Vec3::new(0.0, -0.08, 0.0),
        Vec3::new(2400.0, 0.1, 2400.0),
        &grass,
    );
    b.block(
        Vec3::new(0.0, -0.03, FRONT - 11.0),
        Vec3::new(2.0 * HALL_X + 30.0, 0.04, 22.0),
        &asphalt,
    );
    for k in 0..9 {
        let x = -14.0 + 3.0 * k as f32;
        b.block(
            Vec3::new(x, -0.008, FRONT - 19.5),
            Vec3::new(0.12, 0.004, 5.0),
            &line,
        );
    }

    // Flags along the forecourt's far edge, and the dealer's sign on a pylon.
    let pole = |b: &mut Builder, x: f32, height: f32| {
        let at = Transform::from_translation(Vec3::new(x, height / 2.0, FRONT - 23.0))
            .with_scale(Vec3::new(0.06, height, 0.06));
        b.put(&cylinder, &steel, at);
    };
    for k in 0..7 {
        let x = -15.0 + 5.0 * k as f32;
        pole(&mut b, x, 8.0);
        b.block(
            Vec3::new(x + 0.03, 6.5, FRONT - 23.0),
            Vec3::new(0.02, 2.6, 0.9),
            &flags[k % 3],
        );
    }
    let pylon = Vec3::new(HALL_X + 6.0, 0.0, FRONT - 6.0);
    b.block(pylon + Vec3::Y * 3.5, Vec3::new(0.5, 7.0, 1.6), &accent);
    b.block(pylon + Vec3::Y * 5.6, Vec3::new(0.56, 1.8, 1.4), &white);
    b.block(pylon + Vec3::Y * 3.6, Vec3::new(0.56, 0.4, 1.4), &white);

    // The hall: the floor with joints, the back and right walls, the glass front and left
    // side between posts, the roof with a band round its edge, the ceiling with lights.
    let (width, depth) = (2.0 * HALL_X, BACK - FRONT);
    let middle_z = (FRONT + BACK) / 2.0;
    b.block(
        Vec3::new(0.0, -0.05, middle_z),
        Vec3::new(width, 0.1, depth),
        &floor,
    );
    let mut z = FRONT + 1.2;
    while z < BACK {
        b.block(
            Vec3::new(0.0, 0.0005, z),
            Vec3::new(width, 0.001, 0.02),
            &joint,
        );
        z += 1.2;
    }
    let mut x = -HALL_X + 1.2;
    while x < HALL_X {
        b.block(
            Vec3::new(x, 0.0005, middle_z),
            Vec3::new(0.02, 0.001, depth),
            &joint,
        );
        x += 1.2;
    }
    b.block(
        Vec3::new(0.0, HEIGHT / 2.0, BACK + 0.15),
        Vec3::new(width + 0.6, HEIGHT, 0.3),
        &wall,
    );
    b.block(
        Vec3::new(HALL_X + 0.15, HEIGHT / 2.0, middle_z),
        Vec3::new(0.3, HEIGHT, depth),
        &wall,
    );
    // The dealer's colour along the back wall, behind the cars.
    b.block(
        Vec3::new(0.0, 3.6, BACK - 0.01),
        Vec3::new(width - 4.0, 0.5, 0.02),
        &accent,
    );
    let panes = [
        // The front: along x at FRONT.
        (
            Vec3::new(0.0, HEIGHT / 2.0, FRONT),
            Vec3::new(width, HEIGHT, 0.03),
        ),
        // The left side: along z at -HALL_X.
        (
            Vec3::new(-HALL_X, HEIGHT / 2.0, middle_z),
            Vec3::new(0.03, HEIGHT, depth),
        ),
    ];
    for (at, size) in panes {
        let pane = b.block(at, size, &glass);
        b.commands.entity(pane).insert(NotShadowCaster);
    }
    let mut x = -HALL_X;
    while x <= HALL_X + 0.01 {
        b.block(
            Vec3::new(x, HEIGHT / 2.0, FRONT),
            Vec3::new(0.12, HEIGHT, 0.16),
            &dark,
        );
        x += width / 10.0;
    }
    let mut z = FRONT;
    while z <= BACK + 0.01 {
        b.block(
            Vec3::new(-HALL_X, HEIGHT / 2.0, z),
            Vec3::new(0.16, HEIGHT, 0.12),
            &dark,
        );
        z += depth / 7.0;
    }
    for y in [0.05, 2.6] {
        b.block(Vec3::new(0.0, y, FRONT), Vec3::new(width, 0.1, 0.16), &dark);
        b.block(
            Vec3::new(-HALL_X, y, middle_z),
            Vec3::new(0.16, 0.1, depth),
            &dark,
        );
    }
    // A door in the front, ahead of the car waiting outside.
    b.block(
        Vec3::new(OUTSIDE.x + 2.0, 1.15, FRONT - 0.03),
        Vec3::new(0.05, 2.3, 0.05),
        &steel,
    );
    b.block(
        Vec3::new(OUTSIDE.x - 0.4, 1.15, FRONT - 0.03),
        Vec3::new(0.05, 2.3, 0.05),
        &steel,
    );
    b.block(
        Vec3::new(0.0, HEIGHT + 0.2, middle_z),
        Vec3::new(width + 0.6, 0.4, depth + 0.6),
        &ceiling,
    );
    b.block(
        Vec3::new(0.0, HEIGHT + 0.25, FRONT - 0.32),
        Vec3::new(width + 0.7, 0.7, 0.04),
        &accent,
    );
    b.block(
        Vec3::new(-HALL_X - 0.32, HEIGHT + 0.25, middle_z),
        Vec3::new(0.04, 0.7, depth + 0.7),
        &accent,
    );
    for row in 0..4 {
        let z = FRONT + 3.0 + 5.6 * row as f32;
        for k in 0..6 {
            let x = -HALL_X + 3.0 + 5.6 * k as f32;
            b.block(
                Vec3::new(x, HEIGHT - 0.01, z),
                Vec3::new(1.2, 0.02, 1.2),
                &lamp,
            );
        }
    }
    for (x, z) in [
        (-9.0, -6.0),
        (9.0, -6.0),
        (-9.0, 5.0),
        (0.0, 5.0),
        (9.0, 5.0),
    ] {
        b.commands.spawn((
            InHall,
            PointLight {
                intensity: 900_000.0,
                range: 26.0,
                ..default()
            },
            Transform::from_translation(origin + Vec3::new(x, HEIGHT - 0.6, z)),
        ));
    }
    // Spotlights on the turntable, from either side in front and behind.
    for (x, z) in [(-4.5, -2.0), (4.5, -2.0), (0.0, 1.5), (0.0, -9.0)] {
        let at = origin + Vec3::new(x, HEIGHT - 0.2, TABLE.z + z);
        let aim = origin + TABLE + Vec3::Y * 0.6;
        b.commands.spawn((
            InHall,
            SpotLight {
                intensity: 1_400_000.0,
                range: 20.0,
                inner_angle: 0.35,
                outer_angle: 0.6,
                ..default()
            },
            Transform::from_translation(at).looking_at(aim, Vec3::Y),
        ));
        let can = Transform::from_translation(at - origin)
            .looking_at(aim - origin, Vec3::Y)
            .with_scale(Vec3::new(0.18, 0.18, 0.3));
        b.put(&b.cube.clone(), &dark, can);
    }

    // The turntable: a dark glossy disc with a steel rim, the sign beside it.
    b.put(
        &cylinder,
        &steel,
        Transform::from_translation(TABLE + Vec3::Y * TABLE_HEIGHT / 2.0).with_scale(Vec3::new(
            TABLE_RADIUS + 0.06,
            TABLE_HEIGHT - 0.01,
            TABLE_RADIUS + 0.06,
        )),
    );
    b.put(
        &cylinder,
        &table,
        Transform::from_translation(TABLE + Vec3::Y * TABLE_HEIGHT / 2.0).with_scale(Vec3::new(
            TABLE_RADIUS,
            TABLE_HEIGHT,
            TABLE_RADIUS,
        )),
    );
    let lean = sign_turn();
    b.block(
        SIGN.with_y(0.45) + lean * Vec3::Z * 0.1,
        Vec3::new(0.08, 0.9, 0.08),
        &steel,
    );
    let foot = Transform::from_translation(SIGN.with_y(0.01))
        .with_rotation(Quat::from_rotation_y(SIGN_YAW))
        .with_scale(Vec3::new(0.5, 0.02, 0.4));
    b.put(&b.cube.clone(), &steel, foot);
    let board = Transform::from_translation(SIGN - lean * Vec3::Z * 0.02)
        .with_rotation(lean)
        .with_scale(Vec3::new(SIGN_SIZE.x + 0.04, SIGN_SIZE.y + 0.04, 0.03));
    b.put(&b.cube.clone(), &dark, board);

    // The sales desk behind the turntable on the right: a counter with a monitor, a chair,
    // and a plant.
    let desk = Vec3::new(6.5, 0.0, 1.5);
    b.block(desk + Vec3::Y * 0.37, Vec3::new(2.2, 0.74, 0.8), &white);
    b.block(desk + Vec3::Y * 0.76, Vec3::new(2.3, 0.04, 0.9), &wood);
    b.block(
        desk + Vec3::new(0.3, 1.0, 0.1),
        Vec3::new(0.6, 0.38, 0.03),
        &dark,
    );
    b.block(
        desk + Vec3::new(0.3, 1.0, 0.08),
        Vec3::new(0.56, 0.33, 0.01),
        &screen,
    );
    b.block(
        desk + Vec3::new(0.3, 0.82, 0.14),
        Vec3::new(0.04, 0.1, 0.04),
        &dark,
    );
    b.block(
        desk + Vec3::new(0.0, 0.46, 0.85),
        Vec3::new(0.5, 0.08, 0.5),
        &dark,
    );
    b.block(
        desk + Vec3::new(0.0, 0.75, 1.08),
        Vec3::new(0.5, 0.55, 0.06),
        &dark,
    );
    b.block(
        desk + Vec3::new(0.0, 0.22, 0.85),
        Vec3::new(0.06, 0.44, 0.06),
        &steel,
    );
    // Plants in pots: in the corners, by the desk and along the glass.
    for (x, z, size) in [
        (-HALL_X + 0.8, FRONT + 0.8, 1.0),
        (HALL_X - 0.8, FRONT + 0.8, 1.0),
        (-HALL_X + 0.8, BACK - 0.8, 1.2),
        (HALL_X - 0.8, BACK - 0.8, 1.2),
        (desk.x + 1.7, desk.z, 0.8),
        (9.0, FRONT + 0.8, 0.9),
    ] {
        let at = Vec3::new(x, 0.0, z);
        let tub = Transform::from_translation(at + Vec3::Y * 0.3 * size)
            .with_scale(Vec3::new(0.3, 0.6, 0.3) * size);
        b.put(&cylinder, &pot, tub);
        let bush = Transform::from_translation(at + Vec3::Y * 1.05 * size)
            .with_scale(Vec3::new(0.45, 0.6, 0.45) * size);
        b.put(&ball, &plant, bush);
    }

    // Trees round the meadow.
    for (x, z, size) in [
        (-40.0, -40.0, 1.0),
        (-46.0, -15.0, 0.8),
        (-44.0, 10.0, 1.2),
        (-36.0, 30.0, 0.9),
        (40.0, -36.0, 1.1),
        (44.0, -8.0, 0.9),
        (46.0, 16.0, 1.3),
        (30.0, 34.0, 1.0),
        (-10.0, -48.0, 1.2),
        (14.0, -50.0, 0.9),
        (-12.0, 30.0, 1.1),
        (10.0, 32.0, 0.8),
    ] {
        let at = Vec3::new(x, 0.0, z);
        let trunk = Transform::from_translation(at + Vec3::Y * 1.5 * size)
            .with_scale(Vec3::new(0.2, 3.0, 0.2) * size);
        b.put(&cylinder, &bark, trunk);
        let crown = Transform::from_translation(at + Vec3::Y * 4.6 * size)
            .with_scale(Vec3::new(2.4, 3.0, 2.4) * size);
        b.put(&ball, &leaves, crown);
    }

    // The cars on the floor, in their factory paint, each with a stand at its front left.
    for (chassis, made) in CHASSIS.iter().enumerate() {
        let looks = factory.get(chassis, made.paint);
        let (at, yaw) = place(chassis);
        let motion = at_rest(&looks);
        let transform = placed(origin, at, yaw, &looks);
        b.commands.spawn((
            InHall,
            Shown::Floor(chassis),
            motion,
            transform,
            Visibility::default(),
            looks,
        ));
        let turn = Quat::from_rotation_y(yaw);
        let stand = at + turn * Vec3::new(-1.5, 0.0, -2.9);
        b.block(stand + Vec3::Y * 0.5, Vec3::new(0.05, 1.0, 0.05), &steel);
        b.block(stand + Vec3::Y * 0.01, Vec3::new(0.3, 0.02, 0.3), &steel);
        let panel = Transform::from_translation(stand + Vec3::Y * 1.05)
            .with_rotation(turn * Quat::from_rotation_x(-0.5))
            .with_scale(Vec3::new(0.42, 0.3, 0.02));
        b.put(&b.cube.clone(), &white, panel);
        let header = Transform::from_translation(stand + Vec3::Y * 1.05)
            .with_rotation(turn * Quat::from_rotation_x(-0.5))
            .with_scale(Vec3::new(0.43, 0.3, 0.02))
            * Transform::from_xyz(0.0, 0.4, -0.3).with_scale(Vec3::new(1.0, 0.2, 1.0));
        b.put(&b.cube.clone(), &accent, header);
    }

    // The turntable's car (set by `show`) and the driven car outside.
    let looks = Looks::of(&car.setup);
    let motion = at_rest(&looks);
    b.commands.spawn((
        InHall,
        Shown::Outside,
        motion,
        placed(origin, OUTSIDE, OUTSIDE_YAW, &looks),
        looks,
    ));
    b.commands.spawn((
        InHall,
        Shown::Table,
        Transform::from_translation(origin + TABLE),
        Visibility::default(),
    ));
}

/// Shows the car picked on the turntable (turning, its engine as `run_engine` has it),
/// hides it on the floor, and the driven car outside as it is now.
#[allow(clippy::type_complexity)]
fn show(
    mut commands: Commands,
    time: Res<Time>,
    mut hall: ResMut<Hall>,
    garage: Res<Garage>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut shown: Query<(
        Entity,
        &Shown,
        Option<&mut Looks>,
        Option<&mut Motion>,
        &mut Transform,
        Option<&mut Visibility>,
    )>,
) {
    let picked = (garage.showroom.chassis, garage.showroom.engine);
    let new = hall.on_table != Some(picked);
    let table_looks = new.then(|| {
        let setup = CarSetup::factory(picked.0, picked.1);
        hall.on_table = Some(picked);
        hall.idle = setup.engine.build.ecu.idle_rpm;
        hall.rev_limit = setup.engine.build.ecu.rev_limit;
        hall.rpm = hall.rpm.max(hall.idle);
        Looks::of(&setup)
    });
    hall.turn = (hall.turn + SPIN * time.delta_secs()) % (2.0 * PI);
    let outside = cars
        .iter()
        .find(|car| car.index == active.0)
        .map(|car| Looks::of(&car.setup));
    let engine = EngineRun {
        running: hall.running,
        rpm: hall.rpm,
        fill: crate::physics::drivetrain::fill_for(if hall.clock < BLIP_HELD { 0.8 } else { 0.05 }),
        turbo_speed: 0.0,
    };
    for (entity, shown, looks, motion, mut transform, visibility) in &mut shown {
        match *shown {
            Shown::Table => {
                let (Some(mut looks), Some(mut motion)) = (looks, motion) else {
                    // Its first looks: added, and drawn from the next frame.
                    if let Some(looks) = table_looks.clone() {
                        commands.entity(entity).insert((at_rest(&looks), looks));
                    }
                    continue;
                };
                if let Some(new) = &table_looks {
                    *looks = new.clone();
                    *motion = at_rest(new);
                }
                motion.engine = engine;
                let top = TABLE + Vec3::Y * TABLE_HEIGHT;
                *transform = placed(hall.origin, top, hall.turn, &looks);
            }
            Shown::Floor(chassis) => {
                if let Some(mut visibility) = visibility {
                    visibility.set_if_neq(if chassis == picked.0 {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    });
                }
            }
            Shown::Outside => {
                if let (Some(mut looks), Some(now)) = (looks, &outside)
                    && *looks != *now
                {
                    *looks = now.clone();
                    if let Some(mut motion) = motion {
                        *motion = at_rest(now);
                    }
                    *transform = placed(hall.origin, OUTSIDE, OUTSIDE_YAW, now);
                }
            }
        }
    }
}

/// The turntable's engine, when started: idling, the throttle blipped now and then.
fn run_engine(time: Res<Time>, mut hall: ResMut<Hall>) {
    let dt = time.delta_secs();
    if !hall.running {
        hall.rpm = hall.idle;
        hall.clock = BLIP_HELD;
        return;
    }
    hall.clock += dt;
    if hall.clock > BLIP_EVERY {
        hall.clock = 0.0;
    }
    let (goal, lag) = if hall.clock < BLIP_HELD {
        (hall.rev_limit * BLIP_TO, 0.15)
    } else {
        (hall.idle, 0.45)
    };
    hall.rpm += (goal - hall.rpm) * (1.0 - (-dt / lag).exp());
}

/// The camera round the turntable; dragging swings it round, the mouse wheel takes it
/// nearer or farther. It keeps clear of the cars on the floor.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut hall: ResMut<Hall>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        hall.swing = (hall.swing - motion.delta.x * 0.005) % (2.0 * PI);
    }
    let zoom = hall.zoom * (1.0 - 0.1 * scroll.delta.y.clamp(-3.0, 3.0));
    hall.zoom = zoom.clamp(*ZOOM.start(), *ZOOM.end());
    let target = hall.origin + TARGET;
    let offset = Quat::from_rotation_y(hall.swing) * (EYE - TARGET) * hall.zoom;
    let around = offset.with_y(0.0).clamp_length_max(REACH);
    let eye = target + around.with_y(offset.y);
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = hall.eye.map_or(eye, |was| was.lerp(eye, ease));
    hall.eye = Some(now);
    hall.aside += (1.0 - hall.aside) * ease * 0.5;
    let right = (target - now).cross(Vec3::Y).normalize();
    let looked_at = target + right * ASIDE * hall.aside * hall.zoom;
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(looked_at, Vec3::Y);
    }
}

/// How the sign's board is turned: facing `SIGN_YAW`, leaning back `SIGN_LEAN`.
fn sign_turn() -> Quat {
    Quat::from_rotation_y(SIGN_YAW) * Quat::from_rotation_x(-SIGN_LEAN)
}

/// The sign by the turntable, painted where it is: the car's name, its class and engine.
fn sign(
    mut contexts: EguiContexts,
    hall: Res<Hall>,
    garage: Res<Garage>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    let Ok((camera, eye)) = cameras.single() else {
        return Ok(());
    };
    let lean = sign_turn();
    let (middle, right, up) = (hall.origin + SIGN, lean * Vec3::X, lean * Vec3::Y);
    if (eye.translation() - middle)
        .normalize()
        .dot(right.cross(up))
        < 0.2
    {
        return Ok(());
    }
    // A point on the board, from its top left (0, 0) to its bottom right (1, 1).
    let ctx = contexts.ctx_mut()?;
    let project = |u: f32, v: f32| {
        let at = middle + right * (u - 0.5) * SIGN_SIZE.x - up * (v - 0.5) * SIGN_SIZE.y;
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
        Color32::from_rgb(242, 242, 240),
        Stroke::NONE,
    ));
    if let Some(band) = [(0.0, 0.0), (1.0, 0.0), (1.0, 0.12), (0.0, 0.12)]
        .into_iter()
        .map(|(u, v)| project(u, v))
        .collect::<Option<Vec<_>>>()
    {
        let blue = Color32::from_rgb(15, 46, 107);
        painter.add(egui::Shape::convex_polygon(band, blue, Stroke::NONE));
    }
    let text = |v: f32, size: f32, words: &str, color: Color32| {
        let (Some(at), Some(below)) = (project(0.07, v), project(0.07, v + size)) else {
            return;
        };
        let height = at.distance(below);
        painter.text(
            at,
            Align2::LEFT_TOP,
            words,
            FontId::proportional(crate::menu::scene_text(height)),
            color,
        );
    };
    let chassis = &CHASSIS[garage.showroom.chassis];
    let build = CarBuild::factory(garage.showroom.chassis);
    let engine = build.factory_engine(garage.showroom.engine).hardware().name;
    let ink = Color32::from_rgb(25, 25, 30);
    text(0.2, 0.3, chassis.name, ink);
    text(0.52, 0.13, chassis.class, ink.gamma_multiply(0.75));
    text(0.72, 0.12, &engine, ink.gamma_multiply(0.75));
    Ok(())
}

/// The showroom's window, on the right: every car, the picked one's brochure, starting its
/// engine and driving it away; and back to the car.
#[allow(clippy::too_many_arguments)]
fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    terrain: Res<Terrain>,
    (mut hall, mut garage): (ResMut<Hall>, ResMut<Garage>),
    things: Query<Entity, With<InHall>>,
    active: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    (mut cameras, mut back): (Query<(&ChaseCamera, &mut Projection)>, ResMut<SceneBack>),
) -> Result {
    let Some((entity, mut car)) = super::active(&mut cars, active.0) else {
        return Ok(());
    };
    let mut shown = garage.showroom;
    let mut drive = false;
    let mut out = back.take();
    let ctx = contexts.ctx_mut()?;
    Panel::new("showroom", "Showroom")
        .place(Place::Right)
        .width(440.0)
        .back(Some("Back to the car"))
        .show(ctx, |ui| {
            let names: Vec<String> = CHASSIS.iter().map(|c| c.name.to_string()).collect();
            let mut chassis = shown.chassis;
            if kit::choice(ui, "Car", &mut chassis, &names).changed() {
                shown = Showroom { chassis, engine: 0 };
            }
            let engines: Vec<String> = (CHASSIS[shown.chassis].engines.iter())
                .filter_map(|code| crate::physics::engine::parts::Parts::factory(code))
                .map(|parts| parts.hardware().name.to_string())
                .collect();
            kit::choice(ui, "Engine", &mut shown.engine, &engines);
            drive = ui
                .add_enabled_ui(!hall.buying, |ui| kit::button(ui, "Drive it"))
                .inner
                .clicked();
            if hall.buying {
                kit::note(ui, "Getting it ready…");
            } else if !garage.status.is_empty() {
                kit::note(ui, &garage.status);
            }
            let words = if hall.running {
                "Switch it off"
            } else {
                "Start it"
            };
            if kit::button(ui, words).clicked() {
                hall.running = !hall.running;
            }
            brochure(ui, &shown);
            kit::note(ui, "Drag to look round, the mouse wheel to go nearer.");
            if kit::button(ui, "Back to the car").clicked() {
                out = true;
            }
        });
    if drive {
        let build = CarBuild::factory(shown.chassis);
        let parts = build.factory_engine(shown.engine);
        garage.draft = (build, parts);
        garage.car = Some(car.index);
        garage.retune = true;
        start(&mut commands, &terrain, &mut garage, entity, &mut car);
        hall.buying = true;
    }
    garage.showroom = shown;
    // Bought: straight into the new car, once it's built (at once, unless its maps bake).
    if hall.buying && garage.job.is_none() {
        out = true;
    }
    if out {
        leave(
            &mut commands,
            &mut hall,
            &things,
            (&mut away, &mut audio),
            &mut cameras,
        );
    }
    Ok(())
}
