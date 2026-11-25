//! The parts shops' workshop: the car up on a two-post lift (its wheels hanging), in a bay
//! deep under the shop, with the engine picked in the window on a stand in front of it (its
//! family's size and layout, a turbo if it has one), the tires picked standing in a rack at
//! the back, a workbench with tools on the wall, a tool chest, shelves of parts and an oil
//! drain under the car. The window on the right is the parts shop's (`parts_shop`).
//!
//! The driven car stays parked on the shop's marker meanwhile (a ghost, its input held like
//! in the dyno's room); the car on the lift is drawn from its looks, the engine off, and
//! follows what's built (another chassis puts the lift's arms under its sills afresh).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, FRAC_PI_6};

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::{Garage, parts_shop, start, take_draft};
use crate::audio::{EngineAudio, EngineRun};
use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Away, Car};
use crate::looks::{Looks, Motion};
use crate::menu::kit::{self, Panel, Place};
use crate::physics::engine::families::Layout;
use crate::physics::engine::parts::Parts;
use crate::shops::{AtShop, SceneBack, ShopKind, Shops, shop_window};
use crate::terrain::Terrain;
use crate::wheels::Wheel;

/// How far under the ground at the shop the workshop is (m).
const DEPTH: f32 = 260.0;
/// The workshop's half width (across the car), half length and height (m).
const HALF_WIDTH: f32 = 6.5;
const HALF_LENGTH: f32 = 8.0;
const HEIGHT: f32 = 4.6;
/// How high the lift holds the car: its body's design ground plane over the floor (m).
const RAISE: f32 = 1.3;
/// How far the wheels hang below their design height, as the lower arm's angle (rad).
const DROOP: f32 = 0.2;
/// The lift's columns stand this far out from the car's widest point (m), with their
/// beam over the car at least this high over its roof, and the posts this thick.
const COLUMN_GAP: f32 = 0.55;
const BEAM_OVER_ROOF: f32 = 0.5;
const COLUMN: Vec2 = Vec2::new(0.3, 0.26);
/// Where the engine stand is: the engine's middle over the floor (m).
const STAND: Vec2 = Vec2::new(1.0, -3.1);
/// The tire rack along the back wall: from where to where (x, m).
const RACK_X: std::ops::Range<f32> = 1.2..5.9;
/// The camera's place relative to the car's middle (in front and to its left), what it
/// looks at, how far it may swing round (rad), how near and far it may zoom (a share of
/// its distance), how far right of the car it looks (with the window up on the right),
/// and how far inside the walls it keeps (m).
const EYE: Vec3 = Vec3::new(-2.4, 1.6, -6.2);
const TARGET: Vec3 = Vec3::new(0.0, RAISE + 0.3, -0.3);
const VIEW_SWING: f32 = 1.5;
const ZOOM: std::ops::RangeInclusive<f32> = 0.45..=1.25;
const ASIDE: f32 = 0.9;
const WALL_GAP: f32 = 0.3;

pub struct BayPlugin;

impl Plugin for BayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Bay>()
            .add_systems(
                Update,
                (
                    leave_when_gone,
                    (show, refit, look.after(crate::camera::follow)).run_if(in_bay),
                )
                    .chain()
                    .run_if(resource_exists::<Garage>),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (shop, window.run_if(in_bay)).run_if(resource_exists::<Garage>),
            );
    }
}

/// The workshop, while the car is in it: where it is, the camera's swing and zoom, and
/// what the lift, the stand and the rack were made for.
#[derive(Resource, Default)]
struct Bay {
    inside: bool,
    origin: Vec3,
    swing: f32,
    zoom: f32,
    eye: Option<Vec3>,
    aside: f32,
    lift_for: Option<usize>,
    engine_for: Option<Parts>,
    tires_for: Option<[Wheel; 2]>,
}

fn in_bay(bay: Res<Bay>) -> bool {
    bay.inside
}

/// Everything in the workshop, which goes when the car leaves it.
#[derive(Component)]
struct InBay;

/// What's made afresh when the car's chassis, the engine picked or the tires picked
/// change.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Remade {
    Lift,
    Engine,
    Tires,
}

/// The car on the lift, drawn from the driven car's `Looks`.
#[derive(Component)]
struct CarModel;

/// The workshop's materials and the meshes used often, for what's made afresh.
#[derive(Resource, Clone)]
struct Kit {
    cube: Handle<Mesh>,
    cylinder: Handle<Mesh>,
    floor: Handle<StandardMaterial>,
    wall: Handle<StandardMaterial>,
    skirting: Handle<StandardMaterial>,
    lift: Handle<StandardMaterial>,
    steel: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    rubber: Handle<StandardMaterial>,
    yellow: Handle<StandardMaterial>,
    red: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    pegboard: Handle<StandardMaterial>,
    cast: Handle<StandardMaterial>,
    rust: Handle<StandardMaterial>,
    cover: Handle<StandardMaterial>,
    alloy: Handle<StandardMaterial>,
    boxes: [Handle<StandardMaterial>; 3],
    lamp: Handle<StandardMaterial>,
}

impl Kit {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let mut material = |color: Color, roughness: f32, metallic: f32| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: roughness,
                metallic,
                ..default()
            })
        };
        let lamp = LinearRgba::rgb(8.0, 8.0, 7.5);
        Self {
            cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            cylinder: meshes.add(Cylinder::new(0.5, 1.0)),
            floor: material(Color::srgb(0.36, 0.40, 0.42), 0.3, 0.0),
            wall: material(Color::srgb(0.80, 0.80, 0.78), 0.9, 0.0),
            skirting: material(Color::srgb(0.30, 0.32, 0.34), 0.8, 0.0),
            lift: material(Color::srgb(0.08, 0.28, 0.62), 0.45, 0.3),
            steel: material(Color::srgb(0.72, 0.73, 0.75), 0.3, 0.9),
            dark: material(Color::srgb(0.08, 0.08, 0.09), 0.5, 0.3),
            rubber: material(Color::srgb(0.035, 0.035, 0.04), 0.85, 0.0),
            yellow: material(Color::srgb(0.95, 0.75, 0.10), 0.6, 0.0),
            red: material(Color::srgb(0.70, 0.06, 0.05), 0.4, 0.3),
            wood: material(Color::srgb(0.55, 0.40, 0.25), 0.8, 0.0),
            pegboard: material(Color::srgb(0.62, 0.55, 0.42), 0.9, 0.0),
            cast: material(Color::srgb(0.42, 0.43, 0.45), 0.6, 0.6),
            rust: material(Color::srgb(0.38, 0.22, 0.14), 0.7, 0.5),
            cover: material(Color::srgb(0.60, 0.05, 0.05), 0.55, 0.2),
            alloy: material(Color::srgb(0.66, 0.67, 0.70), 0.35, 0.8),
            boxes: [
                material(Color::srgb(0.75, 0.55, 0.30), 0.9, 0.0),
                material(Color::srgb(0.15, 0.35, 0.65), 0.7, 0.0),
                material(Color::srgb(0.85, 0.85, 0.82), 0.8, 0.0),
            ],
            lamp: materials.add(StandardMaterial {
                base_color: Color::from(lamp),
                emissive: lamp,
                unlit: true,
                ..default()
            }),
        }
    }
}

/// Spawns pieces of the workshop: boxes and cylinders by their transform, in the bay.
struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    kit: &'a Kit,
    origin: Vec3,
    remade: Option<Remade>,
}

impl Builder<'_, '_, '_> {
    fn spawn(&mut self, mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>, t: Transform) {
        let mut t = t;
        t.translation += self.origin;
        let mut entity = self.commands.spawn((
            InBay,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            t,
        ));
        if let Some(remade) = self.remade {
            entity.insert(remade);
        }
    }

    /// A box at `at` (in the bay) of `size`.
    fn block(&mut self, at: Vec3, size: Vec3, material: &Handle<StandardMaterial>) {
        let cube = self.kit.cube.clone();
        self.spawn(
            &cube,
            material,
            Transform::from_translation(at).with_scale(size),
        );
    }

    /// A box at `at`, turned by `rotation`.
    fn turned(&mut self, at: Vec3, rotation: Quat, size: Vec3, m: &Handle<StandardMaterial>) {
        let cube = self.kit.cube.clone();
        let t = Transform::from_translation(at)
            .with_rotation(rotation)
            .with_scale(size);
        self.spawn(&cube, m, t);
    }

    /// A bar of `thickness` (across, up) from `a` to `b`.
    fn bar(&mut self, a: Vec3, b: Vec3, thickness: Vec2, m: &Handle<StandardMaterial>) {
        let length = a.distance(b).max(1e-3);
        let up = if (b - a).normalize().y.abs() > 0.99 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let cube = self.kit.cube.clone();
        let t = Transform::from_translation((a + b) / 2.0)
            .looking_at(b, up)
            .with_scale(thickness.extend(length));
        self.spawn(&cube, m, t);
    }

    /// A cylinder at `at` of `radius` and `length` along `axis`.
    fn disc(
        &mut self,
        at: Vec3,
        axis: Vec3,
        radius: f32,
        length: f32,
        m: &Handle<StandardMaterial>,
    ) {
        let cylinder = self.kit.cylinder.clone();
        let t = Transform::from_translation(at)
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, axis.normalize()))
            .with_scale(Vec3::new(2.0 * radius, length, 2.0 * radius));
        self.spawn(&cylinder, m, t);
    }
}

/// The parts shop's window (before the workshop): onto the lift.
#[allow(clippy::too_many_arguments)]
fn shop(
    mut commands: Commands,
    mut contexts: EguiContexts,
    (shops, mut at): (Res<Shops>, ResMut<AtShop>),
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    (mut bay, mut garage): (ResMut<Bay>, ResMut<Garage>),
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<&mut Projection, With<ChaseCamera>>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
) -> Result {
    if bay.inside {
        return Ok(());
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return Ok(());
    };
    let marker = at.shop.and_then(|i| shops.list.get(i)).map(|p| p.marker);
    let mut enter = false;
    let ctx = contexts.ctx_mut()?;
    shop_window(ctx, &shops, &mut at, ShopKind::Parts, |ui, _| {
        enter = kit::button(ui, "Onto the lift").clicked();
        kit::note(
            ui,
            "A workshop with a two-post lift: engines and their parts, gearboxes, \
             suspension, rims and tires; repairs.",
        );
    });
    let Some(marker) = marker.filter(|_| enter) else {
        return Ok(());
    };
    let ground = terrain.sample(marker.x, marker.y).0;
    *bay = Bay {
        inside: true,
        origin: Vec3::new(marker.x, ground - DEPTH, marker.y),
        zoom: 1.0,
        ..default()
    };
    take_draft(&mut garage, car);
    for mut projection in &mut cameras {
        if let Projection::Perspective(perspective) = &mut *projection {
            (perspective.fov, perspective.near) = (0.9, 0.05);
        }
    }
    away.0 = true;
    audio.hushed = true;
    let kit = Kit::new(&mut meshes, &mut materials);
    build_bay(&mut commands, &bay, &kit, car);
    commands.insert_resource(kit);
    Ok(())
}

/// Off the lift and back to the driven car, parked at the shop.
fn leave(
    commands: &mut Commands,
    bay: &mut Bay,
    things: &Query<Entity, With<InBay>>,
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
    *bay = Bay::default();
    commands.remove_resource::<Kit>();
    away.0 = false;
    audio.hushed = false;
}

/// The car left the shop (or the server, and with it the shop) while in the workshop: out.
fn leave_when_gone(
    mut commands: Commands,
    at: Res<AtShop>,
    mut bay: ResMut<Bay>,
    things: Query<Entity, With<InBay>>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    mut cameras: Query<(&ChaseCamera, &mut Projection)>,
) {
    if bay.inside && at.shop.is_none() {
        let parts = (&mut *away, &mut *audio);
        leave(&mut commands, &mut bay, &things, parts, &mut cameras);
    }
}

/// The car up on the lift: its looks, the wheels hanging at full droop, pointing straight
/// ahead and still, the engine off.
fn looks_on_lift(car: &Car) -> (Looks, Motion) {
    let looks = Looks::of(&car.setup);
    let mut motion = Motion::of(car);
    for (corner, wheel) in motion.wheels.iter_mut().enumerate() {
        let geometry = &looks.axles[corner / 2].geometry;
        (wheel.theta, wheel.steer, wheel.spin) = (geometry.design_angle() - DROOP, 0.0, 0.0);
    }
    motion.engine = EngineRun::default();
    (looks, motion)
}

/// Where the car on the lift is (its centre of mass, as `Looks` are drawn from): its body's
/// design frame (on the ground midway between the axles) `RAISE` over the bay's middle,
/// facing -z.
fn model_at(bay: &Bay, looks: &Looks) -> Transform {
    Transform::from_translation(bay.origin + Vec3::Y * RAISE - looks.body_offset)
}

/// The workshop: floor, walls and lights, the workbench, the tool chest, shelves, the
/// roller door, the oil drain; then the lift with the car, the engine stand and the tires.
fn build_bay(commands: &mut Commands, bay: &Bay, kit: &Kit, car: &Car) {
    let (looks, motion) = looks_on_lift(car);
    let model = model_at(bay, &looks);
    commands.spawn((InBay, CarModel, looks, motion, model));
    let mut b = Builder {
        commands,
        kit,
        origin: bay.origin,
        remade: None,
    };
    let (w, l, h) = (HALF_WIDTH, HALF_LENGTH, HEIGHT);
    // The floor, walls (a grey band along their foot) and ceiling.
    b.block(
        Vec3::new(0.0, -0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * l),
        &kit.floor,
    );
    b.block(
        Vec3::new(0.0, h + 0.1, 0.0),
        Vec3::new(2.0 * w, 0.2, 2.0 * l),
        &kit.wall,
    );
    for s in [-1.0, 1.0] {
        b.block(
            Vec3::new(s * (w + 0.1), h / 2.0, 0.0),
            Vec3::new(0.2, h, 2.0 * l),
            &kit.wall,
        );
        b.block(
            Vec3::new(0.0, h / 2.0, s * (l + 0.1)),
            Vec3::new(2.0 * w, h, 0.2),
            &kit.wall,
        );
        let at = Vec3::new(s * (w - 0.01), 0.5, 0.0);
        b.block(at, Vec3::new(0.04, 1.0, 2.0 * l), &kit.skirting);
        let at = Vec3::new(0.0, 0.5, s * (l - 0.01));
        b.block(at, Vec3::new(2.0 * w, 1.0, 0.04), &kit.skirting);
    }
    // Yellow lines round the lift, and a drain across the floor behind it.
    let (line_x, line_z) = (3.0, (-3.2, 3.4));
    for s in [-1.0, 1.0] {
        let at = Vec3::new(s * line_x, 0.003, (line_z.0 + line_z.1) / 2.0);
        b.block(at, Vec3::new(0.1, 0.006, line_z.1 - line_z.0), &kit.yellow);
    }
    for z in [line_z.0, line_z.1] {
        b.block(
            Vec3::new(0.0, 0.003, z),
            Vec3::new(2.0 * line_x + 0.1, 0.006, 0.1),
            &kit.yellow,
        );
    }
    b.block(
        Vec3::new(0.0, 0.002, 4.6),
        Vec3::new(2.0 * w - 1.0, 0.004, 0.22),
        &kit.dark,
    );
    for k in 0..40 {
        let x = -w + 0.6 + k as f32 * (2.0 * w - 1.2) / 39.0;
        b.block(
            Vec3::new(x, 0.005, 4.6),
            Vec3::new(0.02, 0.004, 0.2),
            &kit.steel,
        );
    }
    workbench(&mut b, Vec3::new(w - 0.4, 0.0, 0.6));
    tool_chest(&mut b, Vec3::new(w - 0.45, 0.0, -3.6));
    shelves(&mut b, Vec3::new(-w + 0.35, 0.0, 1.5));
    roller_door(&mut b, Vec3::new(-2.6, 0.0, l));
    // The oil drain under the car, behind the front axle: a yellow tank on wheels with a
    // funnel on a pole.
    let drain = Vec3::new(
        0.45,
        0.0,
        car.setup.front_z - (car.setup.front_z + car.setup.rear_z) / 2.0 + 1.0,
    );
    b.block(
        drain + Vec3::new(0.0, 0.32, 0.0),
        Vec3::new(0.42, 0.5, 0.42),
        &kit.yellow,
    );
    for (x, z) in [(-0.17, -0.17), (0.17, -0.17), (-0.17, 0.17), (0.17, 0.17)] {
        b.disc(
            drain + Vec3::new(x, 0.04, z),
            Vec3::X,
            0.04,
            0.03,
            &kit.dark,
        );
    }
    b.disc(
        drain + Vec3::new(0.0, 0.8, 0.0),
        Vec3::Y,
        0.02,
        0.6,
        &kit.steel,
    );
    b.disc(
        drain + Vec3::new(0.0, 1.12, 0.0),
        Vec3::Y,
        0.24,
        0.07,
        &kit.yellow,
    );
    b.disc(
        drain + Vec3::new(0.0, 1.157, 0.0),
        Vec3::Y,
        0.2,
        0.01,
        &kit.dark,
    );
    // Lights in the ceiling.
    for x in [-2.8, 0.0, 2.8] {
        for z in [-5.0, -1.7, 1.7, 5.0] {
            b.block(
                Vec3::new(x, h - 0.03, z),
                Vec3::new(0.25, 0.05, 2.2),
                &kit.lamp,
            );
        }
    }
    for x in [-2.8, 2.8] {
        for z in [-5.0, -1.7, 1.7, 5.0] {
            b.commands.spawn((
                InBay,
                PointLight {
                    intensity: 380_000.0,
                    range: 22.0,
                    ..default()
                },
                Transform::from_translation(bay.origin + Vec3::new(x, h - 0.5, z)),
            ));
        }
    }
}

/// A workbench against the right wall at `at` (its foot by the wall): a wooden top on a
/// steel frame with drawers, a vice, a pegboard of tools over it and a lamp.
fn workbench(b: &mut Builder, at: Vec3) {
    let kit = b.kit.clone();
    let (depth, length, top) = (0.7, 2.6, 0.9);
    let middle = at - Vec3::X * (depth / 2.0 - 0.3);
    b.block(
        middle + Vec3::Y * top,
        Vec3::new(depth, 0.05, length),
        &kit.wood,
    );
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let leg = middle
            + Vec3::new(
                x * (depth / 2.0 - 0.04),
                top / 2.0,
                z * (length / 2.0 - 0.04),
            );
        b.block(leg, Vec3::new(0.05, top, 0.05), &kit.lift);
    }
    b.block(
        middle + Vec3::Y * 0.15,
        Vec3::new(depth - 0.06, 0.03, length - 0.06),
        &kit.lift,
    );
    // Drawers under the top at one end.
    let drawers = middle + Vec3::new(0.0, top - 0.3, -length / 2.0 + 0.35);
    b.block(drawers, Vec3::new(depth - 0.04, 0.5, 0.6), &kit.red);
    for k in 0..3 {
        let y = top - 0.12 - 0.16 * k as f32;
        let handle = Vec3::new(middle.x - depth / 2.0 + 0.01, y, drawers.z);
        b.block(handle, Vec3::new(0.02, 0.02, 0.3), &kit.steel);
    }
    // A vice on the front edge.
    let vice = middle + Vec3::new(-depth / 2.0 + 0.12, top + 0.1, 0.5);
    b.block(vice, Vec3::new(0.2, 0.14, 0.12), &kit.lift);
    b.block(
        vice + Vec3::new(-0.12, 0.02, 0.0),
        Vec3::new(0.06, 0.12, 0.16),
        &kit.dark,
    );
    b.disc(
        vice + Vec3::new(-0.2, -0.02, 0.0),
        Vec3::X,
        0.012,
        0.18,
        &kit.steel,
    );
    // Parts on the top: a brake disc, a box, an oil can.
    b.disc(
        middle + Vec3::new(0.0, top + 0.04, -0.2),
        Vec3::Y,
        0.15,
        0.03,
        &kit.cast,
    );
    b.block(
        middle + Vec3::new(0.05, top + 0.08, 0.95),
        Vec3::new(0.25, 0.12, 0.3),
        &kit.boxes[0],
    );
    b.disc(
        middle + Vec3::new(0.1, top + 0.13, 0.2),
        Vec3::Y,
        0.06,
        0.22,
        &kit.yellow,
    );
    // The pegboard with tools: spanners in a row, screwdrivers, a hammer.
    let board = Vec3::new(at.x + 0.27, top + 0.85, middle.z);
    b.block(board, Vec3::new(0.02, 1.1, length), &kit.pegboard);
    let face = board.x - 0.03;
    for k in 0..9 {
        let size = 0.12 + 0.025 * k as f32;
        let z = middle.z - 1.1 + 0.11 * k as f32;
        b.block(
            Vec3::new(face, board.y + 0.25 - size / 2.0, z),
            Vec3::new(0.012, size, 0.025),
            &kit.steel,
        );
    }
    for k in 0..6 {
        let z = middle.z + 0.1 + 0.09 * k as f32;
        let at = Vec3::new(face, board.y + 0.25, z);
        b.block(
            at,
            Vec3::new(0.025, 0.11, 0.025),
            if k % 2 == 0 { &kit.red } else { &kit.yellow },
        );
        b.block(
            at - Vec3::Y * 0.12,
            Vec3::new(0.008, 0.14, 0.008),
            &kit.steel,
        );
    }
    let hammer = Vec3::new(face, board.y - 0.2, middle.z + 0.85);
    b.block(hammer, Vec3::new(0.02, 0.3, 0.03), &kit.wood);
    b.block(
        hammer + Vec3::Y * 0.15,
        Vec3::new(0.03, 0.04, 0.12),
        &kit.dark,
    );
    for k in 0..4 {
        let at = Vec3::new(face, board.y - 0.25, middle.z - 0.9 + 0.22 * k as f32);
        b.disc(at, Vec3::X, 0.05 + 0.01 * k as f32, 0.02, &kit.dark);
    }
    // A lamp over the bench, hanging from the ceiling.
    for dz in [-0.5, 0.5] {
        let from = Vec3::new(at.x - 0.15, top + 1.55, middle.z + dz);
        b.bar(from, from.with_y(HEIGHT), Vec2::splat(0.008), &kit.dark);
    }
    b.block(
        Vec3::new(at.x - 0.15, top + 1.55, middle.z),
        Vec3::new(0.3, 0.05, 1.2),
        &kit.dark,
    );
    b.block(
        Vec3::new(at.x - 0.15, top + 1.52, middle.z),
        Vec3::new(0.2, 0.02, 1.1),
        &kit.lamp,
    );
}

/// A red roll cab of drawers by the right wall at `at`, with tools on its top.
fn tool_chest(b: &mut Builder, at: Vec3) {
    let kit = b.kit.clone();
    let (depth, width, height) = (0.5, 1.0, 1.0);
    b.block(
        at + Vec3::Y * (height / 2.0 + 0.1),
        Vec3::new(depth, height, width),
        &kit.red,
    );
    b.block(
        at + Vec3::Y * (height + 0.12),
        Vec3::new(depth + 0.02, 0.03, width + 0.02),
        &kit.dark,
    );
    for k in 0..7 {
        let y = 0.22 + 0.13 * k as f32;
        let front = Vec3::new(at.x - depth / 2.0 - 0.005, y, at.z);
        b.block(front, Vec3::new(0.01, 0.005, width - 0.04), &kit.dark);
        b.block(
            front + Vec3::new(-0.01, 0.05, 0.0),
            Vec3::new(0.02, 0.015, width * 0.7),
            &kit.steel,
        );
    }
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let wheel = at + Vec3::new(x * (depth / 2.0 - 0.06), 0.05, z * (width / 2.0 - 0.06));
        b.disc(wheel, Vec3::Z, 0.05, 0.04, &kit.dark);
    }
    // A ratchet and a socket tray on top.
    b.block(
        at + Vec3::new(0.0, height + 0.15, -0.2),
        Vec3::new(0.18, 0.02, 0.35),
        &kit.dark,
    );
    for k in 0..6 {
        let socket = at + Vec3::new(0.0, height + 0.18, -0.34 + 0.055 * k as f32);
        b.disc(socket, Vec3::Y, 0.014 + 0.002 * k as f32, 0.04, &kit.steel);
    }
    b.block(
        at + Vec3::new(-0.05, height + 0.15, 0.25),
        Vec3::new(0.03, 0.02, 0.28),
        &kit.steel,
    );
}

/// Steel shelves against the left wall at `at`, full of boxes of parts.
fn shelves(b: &mut Builder, at: Vec3) {
    let kit = b.kit.clone();
    let (depth, length, height) = (0.5, 3.0, 2.2);
    let middle = at + Vec3::X * (depth / 2.0 - 0.25);
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let post = middle + Vec3::new(x * depth / 2.0, height / 2.0, z * length / 2.0);
        b.block(post, Vec3::new(0.04, height, 0.04), &kit.steel);
    }
    for shelf in 0..5 {
        let y = 0.1 + shelf as f32 * 0.5;
        b.block(
            middle + Vec3::Y * y,
            Vec3::new(depth, 0.025, length),
            &kit.steel,
        );
        if shelf == 4 {
            break;
        }
        // Boxes of a few sizes along it.
        let mut z = -length / 2.0 + 0.08;
        let mut k = shelf * 3;
        while z < length / 2.0 - 0.3 {
            let size = Vec3::new(
                0.38,
                0.18 + 0.07 * (k % 3) as f32,
                0.25 + 0.08 * (k % 4) as f32,
            );
            if k % 5 != 3 {
                let at = middle + Vec3::new(0.0, y + 0.012 + size.y / 2.0, z + size.z / 2.0);
                b.block(at, size, &kit.boxes[k % 3]);
            }
            z += size.z + 0.06;
            k += 1;
        }
    }
}

/// A roller door in the back wall at `at`, shut: slats in a frame, a hood over it.
fn roller_door(b: &mut Builder, at: Vec3) {
    let kit = b.kit.clone();
    let (width, height) = (3.4, 3.3);
    let face = at.z - 0.04;
    for k in 0..22 {
        let y = 0.08 + (k as f32 + 0.5) * height / 22.0;
        b.block(
            Vec3::new(at.x, y, face),
            Vec3::new(width, height / 22.0 - 0.012, 0.03),
            &kit.alloy,
        );
    }
    for s in [-1.0, 1.0] {
        let rail = Vec3::new(at.x + s * (width / 2.0 + 0.06), height / 2.0 + 0.1, face);
        b.block(rail, Vec3::new(0.12, height + 0.2, 0.08), &kit.dark);
    }
    b.block(
        Vec3::new(at.x, height + 0.4, face - 0.15),
        Vec3::new(width + 0.3, 0.5, 0.4),
        &kit.dark,
    );
    b.block(
        Vec3::new(at.x, 0.12, face - 0.03),
        Vec3::new(width, 0.06, 0.04),
        &kit.yellow,
    );
    // A yellow bollard either side.
    for s in [-1.0, 1.0] {
        let bollard = Vec3::new(at.x + s * (width / 2.0 + 0.5), 0.45, face - 0.4);
        b.disc(bollard, Vec3::Y, 0.07, 0.9, &kit.yellow);
    }
}

/// The two-post lift under `car`: blue columns either side of its middle with a beam over
/// it, carriages at the arms' height, two arms from each reaching under the sills to pads
/// behind the front wheels and in front of the rear ones; a power unit on the left column.
fn build_lift(b: &mut Builder, looks: &Looks) {
    let kit = b.kit.clone();
    let hull = {
        let mut hull = looks.hull;
        hull.center -= looks.body_offset;
        hull
    };
    let floor = hull.center.y - hull.half.y;
    let roof = hull.center.y + hull.half.y;
    let pad_top = RAISE + floor;
    let axles = looks
        .axles
        .map(|axle| axle.z - (looks.axles[0].z + looks.axles[1].z) / 2.0);
    let build = &looks.build;
    let track = build.chassis().track;
    let sill = (track[0].min(track[1]) / 2.0 - 0.12).max(0.45);
    let pads = [
        axles[0] + looks.axles[0].radius + 0.25,
        axles[1] - looks.axles[1].radius - 0.2,
    ];
    let column_x = hull.half.x + COLUMN_GAP + COLUMN.x / 2.0;
    let column_z = (pads[0] + pads[1]) / 2.0;
    let top = (RAISE + roof + BEAM_OVER_ROOF).clamp(3.6, HEIGHT - 0.25);
    let arm_y = pad_top - 0.14;
    for s in [-1.0, 1.0] {
        let x = s * column_x;
        b.block(
            Vec3::new(x, top / 2.0, column_z),
            Vec3::new(COLUMN.x, top, COLUMN.y),
            &kit.lift,
        );
        b.block(
            Vec3::new(x, 0.01, column_z),
            Vec3::new(0.55, 0.02, 0.5),
            &kit.dark,
        );
        // The inner face's slot and the carriage riding in it.
        let inner = x - s * COLUMN.x / 2.0;
        b.block(
            Vec3::new(inner, top / 2.0, column_z),
            Vec3::new(0.02, top - 0.2, 0.08),
            &kit.dark,
        );
        let carriage = Vec3::new(inner - s * 0.12, arm_y + 0.12, column_z);
        b.block(carriage, Vec3::new(0.24, 0.55, 0.34), &kit.lift);
        b.block(
            carriage - Vec3::Y * 0.22,
            Vec3::new(0.28, 0.1, 0.44),
            &kit.dark,
        );
        // The arms: from the carriage's pivots out under the sill, a telescoping inner
        // part, and a pad on a screw.
        for (k, &pad_z) in pads.iter().enumerate() {
            let pivot = Vec3::new(
                carriage.x - s * 0.08,
                arm_y,
                column_z + (k as f32 - 0.5) * 0.3,
            );
            let pad = Vec3::new(s * sill, arm_y, pad_z);
            let mid = pivot.lerp(pad, 0.55);
            b.bar(pivot, mid, Vec2::new(0.13, 0.09), &kit.lift);
            b.bar(mid, pad, Vec2::new(0.1, 0.07), &kit.dark);
            b.disc(pad + Vec3::Y * 0.07, Vec3::Y, 0.03, 0.1, &kit.steel);
            b.disc(pad + Vec3::Y * 0.125, Vec3::Y, 0.075, 0.03, &kit.rubber);
        }
    }
    // The beam over the car between the columns' tops, with its hose and the switch bar.
    b.block(
        Vec3::new(0.0, top - 0.1, column_z),
        Vec3::new(2.0 * column_x + COLUMN.x, 0.2, COLUMN.y),
        &kit.lift,
    );
    b.block(
        Vec3::new(0.0, top - 0.24, column_z),
        Vec3::new(2.0 * column_x - 0.2, 0.03, 0.03),
        &kit.yellow,
    );
    // The power unit on the outside of the left column: a motor over an oil tank.
    let unit = Vec3::new(-column_x - COLUMN.x / 2.0 - 0.14, 1.2, column_z);
    b.block(unit, Vec3::new(0.24, 0.4, 0.22), &kit.dark);
    b.disc(unit + Vec3::Y * 0.36, Vec3::Y, 0.1, 0.32, &kit.cast);
    b.block(
        unit + Vec3::new(0.0, -0.05, -0.16),
        Vec3::new(0.08, 0.1, 0.08),
        &kit.red,
    );
}

/// The engine of `parts` on a stand in front of the car: its block, heads and covers by
/// its layout, filling its family's length, width and height, with the intake, exhaust
/// manifolds (and a turbo if it has one), the crank pulley in front and the flywheel bolted
/// to the stand's head at the back. The crank runs across the workshop (along x).
fn build_engine(b: &mut Builder, parts: &Parts) {
    let kit = b.kit.clone();
    let family = parts.family();
    let hardware = parts.hardware();
    let (length, width, height) = (family.length, family.width, family.height);
    let middle = Vec3::new(STAND.x, (height / 2.0 + 0.25).max(0.8), STAND.y);
    let at = |x: f32, y: f32, z: f32| middle + Vec3::new(x * length, y * height, z * width);
    let size = |x: f32, y: f32, z: f32| Vec3::new(x * length, y * height, z * width);
    // The block and the oil pan, common to all.
    let (banks, block_width, block_height) = match family.layout {
        Layout::Inline3 | Layout::Inline4 | Layout::Inline6 => (0.0, 0.42, 0.5),
        Layout::Flat4 | Layout::Flat6 => (FRAC_PI_2, 0.34, 0.5),
        Layout::V12 | Layout::V6 => (FRAC_PI_6, 0.4, 0.42),
        _ => (FRAC_PI_4, 0.4, 0.42),
    };
    b.block(
        at(0.0, -0.12, 0.0),
        size(0.84, block_height, block_width),
        &kit.cast,
    );
    b.block(
        at(0.0, -0.42, 0.0),
        size(0.76, 0.14, block_width * 0.85),
        &kit.dark,
    );
    let flat = matches!(family.layout, Layout::Flat4 | Layout::Flat6);
    if banks == 0.0 {
        // An inline engine: one head and cover on top; the intake on its left (+z), the
        // exhaust on its right.
        b.block(at(0.0, 0.2, 0.0), size(0.86, 0.16, 0.44), &kit.alloy);
        b.block(at(0.0, 0.33, 0.0), size(0.8, 0.1, 0.34), &kit.cover);
        b.block(at(0.0, 0.2, 0.38), size(0.66, 0.14, 0.12), &kit.alloy);
        let runners = hardware.cylinders.max(3);
        for k in 0..runners {
            let x = (k as f32 + 0.5) / runners as f32 - 0.5;
            b.block(at(x * 0.7, 0.08, 0.29), size(0.06, 0.18, 0.1), &kit.alloy);
        }
        b.block(at(0.0, 0.02, -0.3), size(0.6, 0.1, 0.1), &kit.rust);
    } else if flat {
        // A flat engine: heads and covers out either side, the intake on top.
        for s in [-1.0, 1.0] {
            b.block(at(0.0, -0.08, s * 0.33), size(0.8, 0.42, 0.18), &kit.alloy);
            b.block(at(0.0, -0.08, s * 0.45), size(0.74, 0.34, 0.08), &kit.cover);
            b.block(at(0.0, -0.4, s * 0.3), size(0.6, 0.07, 0.12), &kit.rust);
        }
        b.block(at(0.0, 0.25, 0.0), size(0.6, 0.12, 0.34), &kit.alloy);
    } else {
        // A V: each bank's head and cover leaning out by half the V's angle, the intake's
        // plenum in the valley, the exhausts under the heads outside.
        for s in [-1.0, 1.0] {
            let lean = Quat::from_rotation_x(s * banks);
            let up = lean * Vec3::Y;
            let base = at(0.0, 0.08, 0.0);
            let head = base + up * 0.22 * height + Vec3::Z * s * 0.1 * width;
            b.turned(head, lean, size(0.86, 0.16, 0.26), &kit.alloy);
            b.turned(
                head + up * 0.12 * height,
                lean,
                size(0.8, 0.08, 0.2),
                &kit.cover,
            );
            b.block(at(0.0, -0.05, -s * 0.36), size(0.6, 0.09, 0.08), &kit.rust);
        }
        b.block(at(0.0, 0.3, 0.0), size(0.66, 0.13, 0.2), &kit.alloy);
    }
    // A turbo by the exhaust: the hot side's snail, the cold side's, and the pipe up.
    if hardware.turbo.is_some() {
        let turbo = at(0.12, -0.05, -0.47);
        b.disc(turbo, Vec3::X, 0.075, 0.07, &kit.rust);
        b.disc(turbo + Vec3::X * 0.09, Vec3::X, 0.085, 0.07, &kit.alloy);
        b.disc(turbo + Vec3::X * 0.045, Vec3::X, 0.03, 0.06, &kit.dark);
        b.disc(
            turbo + Vec3::new(0.16, 0.0, 0.0),
            Vec3::X,
            0.04,
            0.1,
            &kit.alloy,
        );
    }
    // The crank pulley and belt cover in front, the flywheel at the back.
    let front = middle.x - 0.43 * length;
    b.disc(
        Vec3::new(front - 0.03, middle.y - 0.2 * height, middle.z),
        Vec3::X,
        0.08,
        0.04,
        &kit.dark,
    );
    b.block(
        Vec3::new(front - 0.01, middle.y + 0.05 * height, middle.z),
        size(0.02, 0.5, 0.3),
        &kit.dark,
    );
    let back = middle.x + 0.43 * length;
    let flywheel = (0.3 * height).max(0.14);
    b.disc(
        Vec3::new(back + 0.02, middle.y - 0.12 * height, middle.z),
        Vec3::X,
        flywheel,
        0.04,
        &kit.steel,
    );
    // The stand: a head plate with four arms to the flywheel's bolts, a short arm to a
    // post, and a T-shaped base on castors.
    let plate = Vec3::new(back + 0.13, middle.y - 0.12 * height, middle.z);
    b.disc(plate, Vec3::X, 0.1, 0.03, &kit.red);
    for k in 0..4 {
        let angle = FRAC_PI_4 + k as f32 * FRAC_PI_2;
        let bolt = Vec3::new(
            back + 0.05,
            plate.y + angle.sin() * flywheel * 0.8,
            plate.z + angle.cos() * flywheel * 0.8,
        );
        b.bar(plate, bolt, Vec2::splat(0.03), &kit.red);
    }
    let post = Vec3::new(plate.x + 0.3, 0.0, plate.z);
    b.bar(plate, post.with_y(plate.y), Vec2::splat(0.06), &kit.red);
    b.block(
        post.with_y(plate.y / 2.0 + 0.05),
        Vec3::new(0.07, plate.y + 0.05, 0.07),
        &kit.red,
    );
    b.block(
        post + Vec3::new(0.0, 0.06, 0.0),
        Vec3::new(0.07, 0.06, 0.9),
        &kit.red,
    );
    b.block(
        post + Vec3::new(-0.45, 0.06, 0.0),
        Vec3::new(0.9, 0.06, 0.07),
        &kit.red,
    );
    for at in [
        post + Vec3::new(0.0, 0.03, -0.42),
        post + Vec3::new(0.0, 0.03, 0.42),
        post + Vec3::new(-0.88, 0.03, 0.0),
    ] {
        b.disc(at, Vec3::Z, 0.035, 0.03, &kit.dark);
    }
}

/// The tires picked, standing in a two-tier rack along the back wall: the front tires
/// below, the rear ones above (eight of one size when they're the same).
fn build_tires(b: &mut Builder, wheels: &[Wheel; 2]) {
    let kit = b.kit.clone();
    let z = HALF_LENGTH - 0.45;
    let (from, to) = (RACK_X.start, RACK_X.end);
    let tallest = wheels.iter().map(|w| w.tire.radius()).fold(0.0, f32::max);
    let tiers = [0.12, 0.12 + 2.0 * tallest + 0.15];
    for (tier, &y) in tiers.iter().enumerate() {
        b.block(
            Vec3::new((from + to) / 2.0, y, z),
            Vec3::new(to - from, 0.03, 0.7),
            &kit.steel,
        );
        let tire = wheels[tier].tire;
        let (radius, width) = (tire.radius(), tire.width_m());
        let count = (((to - from - 0.2) / (width + 0.03)) as usize).min(10);
        for k in 0..count {
            let x = from + 0.1 + (k as f32 + 0.5) * (width + 0.03);
            let at = Vec3::new(x, y + 0.015 + radius, z);
            b.disc(at, Vec3::X, radius, width, &kit.rubber);
            // The bead's opening, dark through.
            b.disc(at, Vec3::X, tire.rim_radius(), width + 0.004, &kit.dark);
        }
    }
    for x in [from, to] {
        for dz in [-0.33, 0.33] {
            let top = tiers[1] + 0.3;
            b.block(
                Vec3::new(x, top / 2.0, z + dz),
                Vec3::new(0.04, top, 0.04),
                &kit.lift,
            );
        }
    }
}

/// The car on the lift as the driven car is now.
fn show(
    bay: Res<Bay>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut models: Query<(&mut Looks, &mut Motion, &mut Transform), With<CarModel>>,
) {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let (looks, motion) = looks_on_lift(car);
    for (mut shown, mut moved, mut transform) in &mut models {
        *transform = model_at(&bay, &looks);
        shown.set_if_neq(looks.clone());
        moved.set_if_neq(motion);
    }
}

/// Makes the lift afresh for another chassis, the stand's engine for other engine parts
/// picked, and the rack's tires for other tires picked.
fn refit(
    mut commands: Commands,
    mut bay: ResMut<Bay>,
    kit: Option<Res<Kit>>,
    garage: Res<Garage>,
    models: Query<&Looks, With<CarModel>>,
    remade: Query<(Entity, &Remade)>,
) {
    let (Some(kit), Ok(looks)) = (kit, models.single()) else {
        return;
    };
    let (build, parts) = &garage.draft;
    let wheels = build.wheels;
    let chassis = looks.build.chassis;
    let mut redo = Vec::new();
    if bay.lift_for != Some(chassis) {
        bay.lift_for = Some(chassis);
        redo.push(Remade::Lift);
    }
    if bay.engine_for.as_ref() != Some(parts) {
        bay.engine_for = Some(parts.clone());
        redo.push(Remade::Engine);
    }
    if bay.tires_for != Some(wheels) {
        bay.tires_for = Some(wheels);
        redo.push(Remade::Tires);
    }
    for (entity, what) in &remade {
        if redo.contains(what) {
            commands.entity(entity).despawn();
        }
    }
    for what in redo {
        let mut b = Builder {
            commands: &mut commands,
            kit: &kit,
            origin: bay.origin,
            remade: Some(what),
        };
        match what {
            Remade::Lift => build_lift(&mut b, looks),
            Remade::Engine => build_engine(&mut b, parts),
            Remade::Tires => build_tires(&mut b, &wheels),
        }
    }
}

/// The camera on the car from in front and to its left; dragging swings it round, the
/// mouse wheel takes it nearer or farther. It keeps inside the walls.
fn look(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut bay: ResMut<Bay>,
    mut cameras: Query<&mut Transform, With<ChaseCamera>>,
) {
    if buttons.pressed(MouseButton::Left) {
        bay.swing = (bay.swing - motion.delta.x * 0.005).clamp(-VIEW_SWING, VIEW_SWING);
    }
    let zoom = bay.zoom * (1.0 - 0.1 * scroll.delta.y.clamp(-3.0, 3.0));
    bay.zoom = zoom.clamp(*ZOOM.start(), *ZOOM.end());
    let target = bay.origin + TARGET;
    let offset = Quat::from_rotation_y(bay.swing) * (EYE - TARGET) * bay.zoom;
    let room = Vec3::new(HALF_WIDTH - WALL_GAP, HEIGHT, HALF_LENGTH - WALL_GAP);
    let eye = bay.origin + (TARGET + offset).clamp(-room, room);
    let ease = 1.0 - (-6.0 * time.delta_secs()).exp();
    let now = bay.eye.map_or(eye, |was| was.lerp(eye, ease));
    bay.eye = Some(now);
    bay.aside += (1.0 - bay.aside) * ease * 0.5;
    let right = (target - now).cross(Vec3::Y).normalize();
    let looked_at = target + right * ASIDE * bay.aside * bay.zoom;
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(now).looking_at(looked_at, Vec3::Y);
    }
}

/// The parts shop's window, on the right: what's fitted and what to build; and back to the
/// car.
#[allow(clippy::too_many_arguments)]
fn window(
    mut commands: Commands,
    mut contexts: EguiContexts,
    terrain: Res<Terrain>,
    (mut bay, mut garage): (ResMut<Bay>, ResMut<Garage>),
    things: Query<Entity, With<InBay>>,
    active: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    (mut away, mut audio): (ResMut<Away>, ResMut<EngineAudio>),
    (mut cameras, mut back): (Query<(&ChaseCamera, &mut Projection)>, ResMut<SceneBack>),
) -> Result {
    let Some((entity, mut car)) = super::active(&mut cars, active.0) else {
        return Ok(());
    };
    let mut out = back.take();
    let mut build_now = false;
    let ctx = contexts.ctx_mut()?;
    Panel::new("parts shop", "Parts shop")
        .place(Place::Right)
        .width(470.0)
        .back(Some("Back to the car"))
        .show(ctx, |ui| {
            if kit::button(ui, "Back to the car").clicked() {
                out = true;
            }
            build_now = parts_shop(ui, &mut garage, &mut car);
            kit::note(
                ui,
                "The engine picked stands in front of the car, the tires in the rack. Drag \
                 to look round, the mouse wheel to go nearer.",
            );
        });
    if build_now {
        start(&mut commands, &terrain, &mut garage, entity, &mut car);
    }
    if out {
        leave(
            &mut commands,
            &mut bay,
            &things,
            (&mut away, &mut audio),
            &mut cameras,
        );
    }
    Ok(())
}
